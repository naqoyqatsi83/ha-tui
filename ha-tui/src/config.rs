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
    /// Explicit dashboard tabs (`[[tab]]` entries), each an ordered list of
    /// entity_ids. When present, this replaces the automatic room/domain
    /// grouping entirely.
    #[serde(default, rename = "tab")]
    pub dashboard: Vec<DashboardTab>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DashboardTab {
    pub name: String,
    pub entity_ids: Vec<String>,
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

/// Parses `--config <path>` (or `--config=<path>`) out of the process
/// arguments, falling back to [`default_config_path`] when absent.
pub fn resolve_config_path(args: impl Iterator<Item = String>) -> Result<PathBuf> {
    let args: Vec<String> = args.collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(value) = arg.strip_prefix("--config=") {
            return Ok(PathBuf::from(value));
        }
        if arg == "--config" {
            let value = args
                .get(i + 1)
                .context("--config requires a path argument")?;
            return Ok(PathBuf::from(value));
        }
        i += 1;
    }
    default_config_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_config_path_uses_explicit_flag() {
        let args = vec!["ha-tui".to_string(), "--config".to_string(), "/tmp/foo.toml".to_string()];
        let path = resolve_config_path(args.into_iter()).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/foo.toml"));
    }

    #[test]
    fn resolve_config_path_uses_equals_form() {
        let args = vec!["ha-tui".to_string(), "--config=/tmp/bar.toml".to_string()];
        let path = resolve_config_path(args.into_iter()).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/bar.toml"));
    }

    #[test]
    fn resolve_config_path_falls_back_to_default() {
        let args = vec!["ha-tui".to_string()];
        let path = resolve_config_path(args.into_iter()).unwrap();
        assert_eq!(path, default_config_path().unwrap());
    }

    #[test]
    fn dashboard_tabs_parse_from_toml() {
        let toml = r#"
            ha_url = "http://x"
            ha_token = "y"

            [[tab]]
            name = "Living Room"
            entity_ids = ["light.a", "switch.b"]

            [[tab]]
            name = "Bedroom"
            entity_ids = ["light.c"]
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.dashboard.len(), 2);
        assert_eq!(config.dashboard[0].name, "Living Room");
        assert_eq!(config.dashboard[0].entity_ids, vec!["light.a", "switch.b"]);
    }
}
