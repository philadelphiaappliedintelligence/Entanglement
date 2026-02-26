use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub server_url: Option<String>,
    pub username: Option<String>,
    pub auth_token: Option<String>,
    pub refresh_token: Option<String>,
    pub sync_directory: Option<String>,
}

impl Config {
    pub fn config_path() -> anyhow::Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
        let config_dir = home.join(".config").join("entanglement");
        std::fs::create_dir_all(&config_dir)?;
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
        if self.auth_token.is_none() {
            anyhow::bail!("Not logged in. Run: tangle setup");
        }
        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.server_url.is_some() && self.auth_token.is_some() && self.sync_directory.is_some()
    }

    pub fn server_url(&self) -> anyhow::Result<&str> {
        self.server_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No server configured. Run: tangle setup"))
    }

    pub fn auth_token(&self) -> anyhow::Result<&str> {
        self.auth_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Not logged in. Run: tangle setup"))
    }

    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_path() {
        let path = Config::config_path().expect("config_path should succeed");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.ends_with("entanglement/config.toml"),
            "config path should end with entanglement/config.toml, got: {}",
            path_str
        );
    }

    #[test]
    fn test_config_roundtrip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("config.toml");

        let config = Config {
            server_url: Some("https://example.com:1975".to_string()),
            username: Some("alice".to_string()),
            auth_token: Some("tok_abc123".to_string()),
            refresh_token: Some("ref_xyz789".to_string()),
            sync_directory: Some("/home/alice/sync".to_string()),
        };

        config.save_to(&path).expect("save should succeed");
        let loaded = Config::load_from(&path).expect("load should succeed");

        assert_eq!(loaded.server_url, config.server_url);
        assert_eq!(loaded.username, config.username);
        assert_eq!(loaded.auth_token, config.auth_token);
        assert_eq!(loaded.refresh_token, config.refresh_token);
        assert_eq!(loaded.sync_directory, config.sync_directory);
    }
}
