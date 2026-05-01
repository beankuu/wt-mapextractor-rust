use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json;

#[derive(Debug, Deserialize)]
struct RawConfig {
    client: String,
    datamine: String,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub project_root: PathBuf,
    pub for_test_levels: PathBuf,
    pub all_levels: PathBuf,
    pub aces_levels: PathBuf,
    pub hq_context_dir: PathBuf,
    pub context_dir: PathBuf,
}

impl AppConfig {
    pub fn load(project_root: impl Into<PathBuf>) -> Result<Self> {
        let project_root = project_root.into();
        let cfg_path = project_root.join("config.json");

        let cfg_text = std::fs::read_to_string(&cfg_path)
            .with_context(|| format!("Failed to read {}", cfg_path.display()))?;
        let raw: RawConfig = serde_json::from_str(&cfg_text)
            .with_context(|| format!("Invalid JSON in {}", cfg_path.display()))?;

        let client = PathBuf::from(raw.client);
        let datamine = PathBuf::from(raw.datamine);

        Ok(Self {
            project_root: project_root.clone(),
            for_test_levels: project_root.join("for_test").join("levels"),
            all_levels: client.join("levels"),
            aces_levels: datamine.join("aces.vromfs.bin_u").join("levels"),
            hq_context_dir: client.join("content.hq").join("hq_tex").join("res"),
            context_dir: client.join("content").join("base").join("res"),
        })
    }
}
