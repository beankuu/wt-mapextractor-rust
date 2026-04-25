use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json;

#[derive(Debug, Deserialize)]
struct RawConfig {
    client: String,
    datamine: String,
    oo2core: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub project_root: PathBuf,
    pub for_test_levels: PathBuf,
    pub all_levels: PathBuf,
    pub aces_levels: PathBuf,
    pub hq_context_dir: PathBuf,
    pub context_dir: PathBuf,
    /// Path to oo2core_9_win64.dll (if configured)
    pub oo2core_dll: Option<PathBuf>,
}

impl AppConfig {
    pub fn load(project_root: impl Into<PathBuf>) -> Result<Self> {
        let project_root = project_root.into();
        let cfg_path = project_root.join("config.json");

        let cfg_text = std::fs::read_to_string(&cfg_path)
            .with_context(|| format!("Failed to read {}", cfg_path.display()))?;
        let mut raw: RawConfig = serde_json::from_str(&cfg_text)
            .with_context(|| format!("Invalid JSON in {}", cfg_path.display()))?;

        // If oo2core is not set, try to find it automatically, then ask the user.
        if raw.oo2core.is_none() {
            let candidates = [
                project_root.join("src").join("oo2core_9_win64.dll"),
                project_root.join("oo2core_9_win64.dll"),
            ];
            if let Some(found) = candidates.iter().find(|p| p.exists()) {
                raw.oo2core = Some(found.to_string_lossy().into_owned());
            } else {
                // Prompt the user interactively
                eprint!("oo2core_9_win64.dll not found. Enter its full path (or press Enter to skip): ");
                let _ = io::stderr().flush();
                let stdin = io::stdin();
                let line = stdin.lock().lines().next();
                if let Some(Ok(input)) = line {
                    let trimmed = input.trim().to_string();
                    if !trimmed.is_empty() {
                        let dll_path = PathBuf::from(&trimmed);
                        if dll_path.exists() {
                            // Save back to config.json
                            raw.oo2core = Some(trimmed.clone());
                            if let Ok(existing) = std::fs::read_to_string(&cfg_path) {
                                if let Ok(mut obj) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&existing) {
                                    obj.insert("oo2core".to_string(), serde_json::Value::String(trimmed));
                                    if let Ok(updated) = serde_json::to_string_pretty(&serde_json::Value::Object(obj)) {
                                        let _ = std::fs::write(&cfg_path, updated);
                                        eprintln!("  Saved oo2core path to config.json");
                                    }
                                }
                            }
                        } else {
                            eprintln!("  Warning: path does not exist, skipping oo2core configuration");
                        }
                    }
                }
            }
        }

        let oo2core_dll = raw.oo2core.as_deref().map(PathBuf::from).filter(|p| p.exists());

        let client = PathBuf::from(raw.client);
        let datamine = PathBuf::from(raw.datamine);

        Ok(Self {
            project_root: project_root.clone(),
            for_test_levels: project_root.join("for_test").join("levels"),
            all_levels: client.join("levels"),
            aces_levels: datamine.join("aces.vromfs.bin_u").join("levels"),
            hq_context_dir: client.join("content.hq").join("hq_tex").join("res"),
            context_dir: client.join("content").join("base").join("res"),
            oo2core_dll,
        })
    }
}
