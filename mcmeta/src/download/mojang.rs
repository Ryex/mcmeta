use libmcmeta::models::mojang::{MinecraftVersion, MojangVersionManifest};
use serde::Deserialize;
use serde_valid::Validate;
use tempdir::TempDir;
use tracing::debug;

use anyhow::{anyhow, Result};

use crate::download::errors::MetadataError;

fn default_download_url() -> String {
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json".to_string()
}

#[derive(Deserialize, Debug)]
struct DownloadConfig {
    #[serde(default = "default_download_url")]
    pub manifest_url: String,
}

impl DownloadConfig {
    fn from_config() -> Result<Self> {
        let config = config::Config::builder()
            .add_source(config::Environment::with_prefix("MCMETA_MOJANG"))
            .build()?;

        config.try_deserialize::<'_, Self>().map_err(Into::into)
    }
}

pub async fn load_manifest() -> Result<MojangVersionManifest> {
    let client = reqwest::Client::new();
    let config = DownloadConfig::from_config()?;

    let body = client
        .get(&config.manifest_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let manifest: MojangVersionManifest =
        serde_json::from_str(&body).map_err(|err| MetadataError::from_json_err(err, &body))?;
    manifest.validate()?;
    Ok(manifest)
}

pub async fn load_version_manifest(version_url: &str) -> Result<MinecraftVersion> {
    let client = reqwest::Client::new();

    debug!("Fetching version manifest from {:#?}", version_url);

    let body = client
        .get(version_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let manifest: MinecraftVersion =
        serde_json::from_str(&body).map_err(|err| MetadataError::from_json_err(err, &body))?;
    manifest.validate()?;
    Ok(manifest)
}

pub async fn load_zipped_version(version_url: &str) -> Result<MinecraftVersion> {
    use std::io::prelude::*;

    let client = reqwest::Client::new();

    debug!("Fetching zipped version from {:#?}", version_url);

    let file_response = client.get(version_url).send().await?.error_for_status()?;

    let tmp_dir = TempDir::new("mcmeta_mojang_zip")?;
    let dest_path = {
        let fname = file_response
            .url()
            .path_segments()
            .and_then(|segments| segments.last())
            .and_then(|name| if name.is_empty() { None } else { Some(name) })
            .unwrap_or("tmp.zip");

        tmp_dir.path().join(fname)
    };

    {
        // write to file, context drop to flush and close
        let mut file = std::fs::File::create(&dest_path)?;
        let mut content = std::io::Cursor::new(file_response.bytes().await?);
        std::io::copy(&mut content, &mut file)?;
    }

    let file = std::fs::File::open(&dest_path)?;

    let mut archive = zip::ZipArchive::new(file)?;

    let mut manifest: Option<MinecraftVersion> = None;
    for i in 0..archive.len() {
        let mut zfile = archive.by_index(i)?;
        if zfile.name().ends_with(".json") {
            debug!("Found {} as version json", zfile.name());
            let mut contents = String::new();
            zfile.read_to_string(&mut contents).unwrap();

            manifest = Some(
                serde_json::from_str(&contents)
                    .map_err(|err| MetadataError::from_json_err(err, &contents))?,
            );
        }
    }

    Ok(manifest.ok_or(anyhow!("Unable to find version manifest"))?)
}
