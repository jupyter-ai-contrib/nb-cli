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
        let url = format!("{}/api", self.base_url);
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

    fn contents_url(&self, path: &str) -> Result<String> {
        let mut url = url::Url::parse(&self.base_url).context("Invalid server URL")?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Server URL cannot be a base"))?;
            segments.push("api").push("contents");
            for part in path.split('/') {
                if !part.is_empty() {
                    segments.push(part);
                }
            }
        }
        Ok(url.to_string())
    }

    /// Read a notebook from the server via the Contents API.
    pub async fn get_notebook(&self, path: &str) -> Result<nbformat::v4::Notebook> {
        let url = self.contents_url(path)?;

        let response = self
            .client
            .get(&url)
            .query(&[
                ("token", self.token.as_str()),
                ("content", "1"),
                ("type", "notebook"),
            ])
            .send()
            .await
            .context("Failed to fetch notebook from server")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to fetch notebook: HTTP {} - {}", status, text);
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Contents API response")?;

        let content = body
            .get("content")
            .cloned()
            .context("Contents API response missing 'content' field")?;

        serde_json::from_value::<nbformat::v4::Notebook>(content)
            .context("Failed to parse notebook from server (expected nbformat v4)")
    }

    /// Save a notebook to the server
    pub async fn save_notebook(&self, path: &str, notebook: &nbformat::v4::Notebook) -> Result<()> {
        let url = self.contents_url(path)?;

        let payload = serde_json::json!({
            "type": "notebook",
            "format": "json",
            "content": notebook
        });

        let response = self
            .client
            .put(&url)
            .query(&[("token", self.token.as_str())])
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

    /// Probe whether the Y.js backend (jupyter-server-documents) is available.
    /// Uses POST /api/fileid/index, which only jupyter-server-documents registers.
    /// GET /api/fileid/id cannot be used: it returns 404 for unindexed paths
    /// even when the fileid extension is installed.
    ///
    /// This probe checks only fileid/index, so jupyter-collaboration servers
    /// (which 404 it) are cached Some(false) and served via the Contents API
    /// path. ydoc.rs `get_file_id` keeps a separate collaboration/session
    /// fallback that #95 extends for full jupyter-collaboration support.
    ///
    /// Probes with the server root path, which always exists, so the index
    /// call is idempotent and creates no record for a fake path. Verified
    /// against jupyter-server-documents 0.2.2 with both ArbitraryFileIdManager
    /// and LocalFileIdManager: both return 200 for the root path.
    pub async fn probe_ydoc(&self) -> Result<bool> {
        let url = format!("{}/api/fileid/index", self.base_url);
        let response = self
            .client
            .post(&url)
            .query(&[("token", self.token.as_str()), ("path", "")])
            .send()
            .await
            .context("Failed to probe FileID API")?;

        let status = response.status();
        if status.as_u16() == 404 {
            Ok(false)
        } else if status.is_success() || status.is_redirection() {
            Ok(true)
        } else {
            anyhow::bail!("FileID probe returned unexpected status {}", status)
        }
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
