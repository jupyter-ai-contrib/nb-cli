//! HTTP client for Jupyter Server REST API

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Jupyter Server REST API client
pub struct JupyterClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelInfo {
    pub id: String,
    pub name: String,
    pub last_activity: String,
    pub execution_state: String,
    pub connections: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub r#type: String,
    pub kernel: KernelInfo,
}

#[derive(Debug, Serialize)]
struct CreateSessionRequest {
    path: String,
    name: String,
    r#type: String,
    kernel: KernelSpec,
}

#[derive(Debug, Serialize)]
struct KernelSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    name: String,
}

impl JupyterClient {
    /// Create a new Jupyter Server client
    pub fn new(server_url: String, token: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            base_url: server_url.trim_end_matches('/').to_string(),
            token,
            client,
        })
    }

    /// Test connection to the server
    pub async fn test_connection(&self) -> Result<()> {
        let url = format!("{}/api/contents", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to connect to Jupyter Server")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to connect to Jupyter Server: HTTP {}",
                response.status()
            );
        }

        Ok(())
    }

    /// Get kernel info by ID
    pub async fn get_kernel(&self, kernel_id: &str) -> Result<KernelInfo> {
        let url = format!("{}/api/kernels/{}", self.base_url, kernel_id);
        let response = self
            .client
            .get(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to get kernel info")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to get kernel info: HTTP {}", response.status());
        }

        response
            .json()
            .await
            .context("Failed to parse kernel info response")
    }

    /// List all running kernels
    #[allow(dead_code)]
    pub async fn list_kernels(&self) -> Result<Vec<KernelInfo>> {
        let url = format!("{}/api/kernels", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to list kernels")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to list kernels: HTTP {}", response.status());
        }

        let kernels = response
            .json()
            .await
            .context("Failed to parse kernels response")?;

        Ok(kernels)
    }

    /// Start a new kernel
    #[allow(dead_code)]
    pub async fn start_kernel(&self, kernel_name: &str) -> Result<KernelInfo> {
        let url = format!("{}/api/kernels", self.base_url);
        let payload = serde_json::json!({
            "name": kernel_name
        });

        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .json(&payload)
            .send()
            .await
            .context("Failed to start kernel")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to start kernel: HTTP {}", response.status());
        }

        let kernel = response
            .json()
            .await
            .context("Failed to parse kernel response")?;

        Ok(kernel)
    }

    /// List all sessions
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let url = format!("{}/api/sessions", self.base_url);
        let response = self
            .client
            .get(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to list sessions")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to list sessions: HTTP {}", response.status());
        }

        let sessions = response
            .json()
            .await
            .context("Failed to parse sessions response")?;

        Ok(sessions)
    }

    /// Create a new session with an existing kernel
    #[allow(dead_code)]
    pub async fn create_session_with_kernel(
        &self,
        notebook_path: &str,
        kernel_id: &str,
        kernel_name: &str,
    ) -> Result<SessionInfo> {
        let url = format!("{}/api/sessions", self.base_url);
        let payload = CreateSessionRequest {
            path: notebook_path.to_string(),
            name: filename_from_path(notebook_path),
            r#type: "notebook".to_string(),
            kernel: KernelSpec {
                id: Some(kernel_id.to_string()),
                name: kernel_name.to_string(),
            },
        };

        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .json(&payload)
            .send()
            .await
            .context("Failed to create session with existing kernel")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to create session with existing kernel: HTTP {} - {}",
                status,
                text
            );
        }

        let session = response
            .json()
            .await
            .context("Failed to parse session response")?;

        Ok(session)
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        notebook_path: &str,
        kernel_name: &str,
    ) -> Result<SessionInfo> {
        let url = format!("{}/api/sessions", self.base_url);
        let payload = CreateSessionRequest {
            path: notebook_path.to_string(),
            name: filename_from_path(notebook_path),
            r#type: "notebook".to_string(),
            kernel: KernelSpec {
                id: None,
                name: kernel_name.to_string(),
            },
        };

        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .json(&payload)
            .send()
            .await
            .context("Failed to create session")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to create session: HTTP {} - {}", status, text);
        }

        let session = response
            .json()
            .await
            .context("Failed to parse session response")?;

        Ok(session)
    }

    /// Restart a kernel
    pub async fn restart_kernel(&self, kernel_id: &str) -> Result<KernelInfo> {
        let url = format!("{}/api/kernels/{}/restart", self.base_url, kernel_id);
        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to restart kernel")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to restart kernel: HTTP {}", response.status());
        }

        response
            .json()
            .await
            .context("Failed to parse kernel restart response")
    }

    /// Delete a session
    #[allow(dead_code)]
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let url = format!("{}/api/sessions/{}", self.base_url, session_id);
        let response = self
            .client
            .delete(&url)
            .query(&[("token", &self.token)])
            .send()
            .await
            .context("Failed to delete session")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to delete session: HTTP {}", response.status());
        }

        Ok(())
    }

    /// Save a notebook to the server
    #[allow(dead_code)]
    pub async fn save_notebook(&self, path: &str, notebook: &nbformat::v4::Notebook) -> Result<()> {
        let url = format!("{}/api/contents/{}", self.base_url, path);

        let payload = serde_json::json!({
            "type": "notebook",
            "format": "json",
            "content": notebook
        });

        let response = self
            .client
            .put(&url)
            .query(&[("token", &self.token)])
            .json(&payload)
            .send()
            .await
            .context("Failed to save notebook")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to save notebook: HTTP {} - {}", status, text);
        }

        Ok(())
    }

    /// Get the WebSocket URL for a kernel
    pub fn get_ws_url(&self, kernel_id: &str, session_id: Option<&str>) -> String {
        let ws_base = self
            .base_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        if let Some(sid) = session_id {
            format!(
                "{}/api/kernels/{}/channels?session_id={}&token={}",
                ws_base, kernel_id, sid, self.token
            )
        } else {
            format!(
                "{}/api/kernels/{}/channels?token={}",
                ws_base, kernel_id, self.token
            )
        }
    }
}

fn filename_from_path(notebook_path: &str) -> String {
    Path::new(notebook_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(notebook_path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(base_url: &str) -> JupyterClient {
        JupyterClient::new(base_url.to_string(), "tok".to_string()).unwrap()
    }

    #[test]
    fn test_get_ws_url_formats() {
        let c = client("http://127.0.0.1:8888");

        let url = c.get_ws_url("kid1", None);
        assert_eq!(
            url,
            "ws://127.0.0.1:8888/api/kernels/kid1/channels?token=tok"
        );

        let url = c.get_ws_url("kid2", Some("sid42"));
        assert_eq!(
            url,
            "ws://127.0.0.1:8888/api/kernels/kid2/channels?session_id=sid42&token=tok"
        );

        let c_https = client("https://example.com");
        let url = c_https.get_ws_url("kid3", None);
        assert!(url.starts_with("wss://"), "https must become wss");
    }

    #[test]
    fn test_new_trims_trailing_slash() {
        let c = client("http://host:8888/");
        let url = c.get_ws_url("k", None);
        assert!(
            !url.contains("//api"),
            "trailing slash must not produce double-slash in URL: {url}"
        );
        assert!(url.contains("/api/kernels/k/channels"));
    }

    #[test]
    fn test_filename_from_path_extracts_name() {
        assert_eq!(
            filename_from_path("path/to/notebook.ipynb"),
            "notebook.ipynb"
        );
        assert_eq!(filename_from_path("notebook.ipynb"), "notebook.ipynb");
        // Empty string: Path::new("").file_name() returns None → falls back to input unchanged
        assert_eq!(filename_from_path(""), "");
    }
}
