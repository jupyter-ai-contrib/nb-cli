use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupyterConnection {
    pub server_url: String,
    pub token: String,
    pub connected_at: DateTime<Utc>,
    pub working_dir: Option<String>,
    pub last_validated: Option<DateTime<Utc>>,
    /// Environment manager used when connecting (direct, uv, pixi)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_manager: Option<String>,
    /// Project root path for uv/pixi environments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub version: String,
    pub connection: Option<JupyterConnection>,
}

impl Config {
    /// Get path to config file: ./.jupyter/cli.json (relative to current directory)
    pub fn config_path() -> Result<PathBuf> {
        let cwd = std::env::current_dir().context("Could not determine current directory")?;
        Ok(cwd.join(".jupyter").join("cli.json"))
    }

    /// Load config from ./.jupyter/cli.json
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Config::default());
        }

        let content = fs::read_to_string(&path).context("Failed to read config file")?;
        let config: Config =
            serde_json::from_str(&content).context("Failed to parse config file")?;

        Ok(config)
    }

    /// Save config to ./.jupyter/cli.json
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::config_path()?;

        // Create .jupyter directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create .jupyter directory")?;
        }

        let json = serde_json::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(&path, json).context("Failed to write config file")?;

        // Set restrictive permissions on Unix (token security)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)?.permissions();
            perms.set_mode(0o600); // rw-------
            fs::set_permissions(&path, perms)?;
        }

        Ok(path)
    }

    /// Resolve connection with CLI args taking priority
    pub fn resolve_connection(
        &self,
        cli_server: Option<String>,
        cli_token: Option<String>,
    ) -> Result<Option<(String, String)>> {
        // CLI args have highest priority
        if let (Some(server), Some(token)) = (cli_server, cli_token) {
            return Ok(Some((server, token)));
        }

        // Check saved connection
        if let Some(conn) = &self.connection {
            return Ok(Some((conn.server_url.clone(), conn.token.clone())));
        }

        Ok(None)
    }
}
