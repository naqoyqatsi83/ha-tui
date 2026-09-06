use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub ha_url: String,
    pub ha_token: String,
    /// Accept self-signed / otherwise invalid TLS certs on the HA connection.
    /// Only meant for trusted local instances - off by default.
    #[serde(default)]
    pub insecure_skip_verify: bool,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file at {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.ha_url.trim().is_empty() {
            anyhow::bail!("config `ha_url` must not be empty");
        }
        if self.ha_token.trim().is_empty() {
            anyhow::bail!("config `ha_token` must not be empty");
        }
        Ok(())
    }
}

/// Default config file location: `~/.config/ha-tui/config.toml`.
pub fn default_config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "ha-tui")
        .context("could not determine home directory for default config path")?;
    Ok(dirs.config_dir().join("config.toml"))
}
