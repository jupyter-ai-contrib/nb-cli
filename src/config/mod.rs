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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_connection(server_url: &str, token: &str) -> JupyterConnection {
        JupyterConnection {
            server_url: server_url.to_string(),
            token: token.to_string(),
            connected_at: Utc::now(),
            working_dir: None,
            last_validated: None,
            env_manager: None,
            project_root: None,
        }
    }

    #[test]
    fn test_config_default_is_none_connection() {
        let config = Config::default();
        assert!(config.connection.is_none());
        assert_eq!(config.version, "");
    }

    #[test]
    fn test_resolve_connection_cli_args_take_priority() {
        // Saved connection exists, but CLI args should win.
        let config = Config {
            version: String::new(),
            connection: Some(make_connection("http://saved:8888", "saved_token")),
        };
        let result = config
            .resolve_connection(
                Some("http://cli:9999".to_string()),
                Some("cli_token".to_string()),
            )
            .unwrap();
        assert_eq!(
            result,
            Some(("http://cli:9999".to_string(), "cli_token".to_string()))
        );
    }

    #[test]
    fn test_resolve_connection_partial_cli_args_falls_back_to_saved() {
        // Only server provided (no token): resolve_connection falls back to saved.
        // (resolve_execution_mode in common.rs handles the error case before calling this.)
        let config = Config {
            version: String::new(),
            connection: Some(make_connection("http://saved:8888", "saved_token")),
        };
        let result = config
            .resolve_connection(Some("http://cli:9999".to_string()), None)
            .unwrap();
        assert_eq!(
            result,
            Some(("http://saved:8888".to_string(), "saved_token".to_string()))
        );
    }

    #[test]
    fn test_resolve_connection_uses_saved_when_no_cli_args() {
        let config = Config {
            version: String::new(),
            connection: Some(make_connection("http://saved:8888", "saved_token")),
        };
        let result = config.resolve_connection(None, None).unwrap();
        assert_eq!(
            result,
            Some(("http://saved:8888".to_string(), "saved_token".to_string()))
        );
    }

    #[test]
    fn test_resolve_connection_returns_none_when_no_config_no_args() {
        let config = Config::default();
        let result = config.resolve_connection(None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_serde_roundtrip_with_connection() {
        let original = Config {
            version: "1".to_string(),
            connection: Some(JupyterConnection {
                server_url: "http://127.0.0.1:8888".to_string(),
                token: "abc123".to_string(),
                connected_at: Utc::now(),
                working_dir: Some("/home/user".to_string()),
                last_validated: None,
                env_manager: Some("uv".to_string()),
                project_root: Some("/projects/myproject".to_string()),
            }),
        };
        let json = serde_json::to_string(&original).unwrap();
        let roundtripped: Config = serde_json::from_str(&json).unwrap();
        let conn = roundtripped.connection.unwrap();
        assert_eq!(conn.server_url, "http://127.0.0.1:8888");
        assert_eq!(conn.token, "abc123");
        assert_eq!(conn.working_dir, Some("/home/user".to_string()));
        assert_eq!(conn.env_manager, Some("uv".to_string()));
    }

    #[test]
    fn test_serde_roundtrip_no_connection() {
        let original = Config::default();
        let json = serde_json::to_string(&original).unwrap();
        let roundtripped: Config = serde_json::from_str(&json).unwrap();
        assert!(roundtripped.connection.is_none());
    }

    #[test]
    fn test_serde_env_manager_omitted_when_none() {
        // env_manager has #[serde(skip_serializing_if = "Option::is_none")].
        // When None, the key must be absent from the serialized JSON — not present as null.
        // This matters for forward compatibility: old server configs written without
        // env_manager should continue to deserialize correctly.
        let config = Config {
            version: String::new(),
            connection: Some(make_connection("http://127.0.0.1:8888", "tok")),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("env_manager"),
            "env_manager must be absent when None\nJSON: {}",
            json
        );
        assert!(
            !json.contains("project_root"),
            "project_root must be absent when None\nJSON: {}",
            json
        );
    }
}
