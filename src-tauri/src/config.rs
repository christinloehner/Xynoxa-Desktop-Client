use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub server_url: Option<String>,
    pub sync_path: Option<String>,
    pub auth_token: Option<String>,
    pub setup_completed: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            sync_path: None,
            auth_token: None,
            setup_completed: false,
        }
    }
}

pub struct ConfigManager {
    config_path: PathBuf,
    pub config: Mutex<AppConfig>,
}

impl ConfigManager {
    pub fn new(_app_data_dir: &Path) -> Self {
        // Use XDG Config Home or fallback.
        // Note: app_data_dir from Tauri is usually ~/.local/share/APP.
        // We want ~/.config/xynoxa/server.conf (with legacy migration from xynoxa)
        // We can use std::env or just perform relative path adjustments if we want to be strict strict,
        // but assuming Linux environment:

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_dir = PathBuf::from(&home).join(".config").join("xynoxa");
        let legacy_dir = PathBuf::from(&home).join(".config").join("xynoxa");
        fs::create_dir_all(&config_dir).ok(); // Ensure dir exists
        let config_path = config_dir.join("server.conf"); // Requested filename
        let legacy_path = legacy_dir.join("server.conf");

        let config = if config_path.exists() {
            let content = fs::read_to_string(&config_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else if legacy_path.exists() {
            let content = fs::read_to_string(&legacy_path).unwrap_or_default();
            let migrated: AppConfig = serde_json::from_str(&content).unwrap_or_default();
            let _ = fs::write(&config_path, serde_json::to_string_pretty(&migrated).unwrap_or_default());
            migrated
        } else {
            AppConfig::default()
        };

        Self {
            config_path,
            config: Mutex::new(config),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let config = self
            .config
            .lock()
            .map_err(|_| "Failed to lock config".to_string())?;
        let content = serde_json::to_string_pretty(&*config).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update(
        &self,
        url: Option<String>,
        path: Option<String>,
        token: Option<String>,
        completed: Option<bool>,
    ) -> Result<(), String> {
        let mut config = self
            .config
            .lock()
            .map_err(|_| "Failed to lock config".to_string())?;

        if let Some(u) = url {
            config.server_url = Some(u);
        }
        if let Some(p) = path {
            config.sync_path = Some(p);
        }
        if let Some(t) = token {
            config.auth_token = Some(t);
        }
        if let Some(c) = completed {
            config.setup_completed = c;
        }

        // Save automatically on update
        let content = serde_json::to_string_pretty(&*config).map_err(|e| e.to_string())?;
        fs::write(&self.config_path, content).map_err(|e| e.to_string())?;

        Ok(())
    }
}
