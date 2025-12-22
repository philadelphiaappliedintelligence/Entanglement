use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub server_url: Option<String>,
    pub grpc_url: Option<String>,
    pub server_name: Option<String>,
    pub token: Option<String>,
    pub user_id: Option<String>,
    pub sync_root: Option<String>,
}

impl Config {
    pub fn config_path() -> anyhow::Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "entanglement", "tangle")
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
        
        let config_dir = proj_dirs.config_dir();
        std::fs::create_dir_all(config_dir)?;
        
        Ok(config_dir.join("config.toml"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path()?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn require_auth(&self) -> anyhow::Result<()> {
        if self.token.is_none() {
            anyhow::bail!("Not logged in. Run: tangle setup");
        }
        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.server_url.is_some() && self.token.is_some() && self.sync_root.is_some()
    }

    pub fn get_grpc_url(&self) -> anyhow::Result<String> {
        // If explicit grpc_url is set, use it
        if let Some(grpc) = &self.grpc_url {
            return Ok(grpc.clone());
        }
        
        // Otherwise, derive from server_url (REST is 8080, gRPC is 50051)
        if let Some(server) = &self.server_url {
            let url = server
                .replace(":8080", ":50051")
                .replace("http://", "http://")
                .replace("https://", "https://");
            return Ok(url);
        }
        
        anyhow::bail!("No server configured")
    }
}

