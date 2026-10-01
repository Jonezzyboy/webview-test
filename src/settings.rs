use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What the setup screen asks for, remembered between runs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub endpoint: String,
    pub app_id: String,
    pub secret: String,
    pub flow_id: String,
    pub identity: String,
    pub device: String,
    pub url: String,
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("webview-test").join("settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Saves the settings, secret included, readable by the current user only.
pub fn save(settings: &Settings) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let Ok(raw) = serde_json::to_string_pretty(settings) else { return };
    if let Err(err) = fs::write(&path, raw) {
        eprintln!("failed to save settings: {err}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}
