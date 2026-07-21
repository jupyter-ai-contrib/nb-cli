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

    /// Probe whether the server supports `POST /api/kernels/{id}/execute`.
    /// Sends a minimal invalid request (empty cells) and checks the status:
    /// - 400 or 200 means the endpoint exists (server understands the route)
    /// - 404 or 405 means the endpoint is absent (fall back to kernel WS)
    pub async fn probe_execute_api(&self, kernel_id: &str) -> Result<bool> {
        let url = format!("{}/api/kernels/{}/execute", self.base_url, kernel_id);
        let payload = serde_json::json!({
            "document_id": "",
            "cells": []
        });

        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .json(&payload)
            .send()
            .await
            .context("Failed to probe execute API")?;

        let status = response.status().as_u16();
        // 400 = endpoint exists but rejects our empty probe
        // 200 = endpoint exists and accepted (unlikely with empty cells)
        // 404/405 = endpoint not registered on this server
        Ok(status == 400 || status == 200)
    }

    /// Trigger cell execution via the server-driven execute API
    /// (`POST /api/kernels/{kernel_id}/execute`).
    ///
    /// Returns `Ok(())` on 200 (accepted, fire-and-forget).
    /// Returns typed errors for 409 (source mismatch) and 408 (predecessor timeout).
    pub async fn execute_cells(
        &self,
        kernel_id: &str,
        request: &ExecuteCellsRequest,
    ) -> Result<ExecuteCellsResponse> {
        let url = format!("{}/api/kernels/{}/execute", self.base_url, kernel_id);

        let response = self
            .client
            .post(&url)
            .query(&[("token", &self.token)])
            .json(request)
            .send()
            .await
            .context("Failed to call execute API")?;

        let status = response.status().as_u16();
        match status {
            200 => Ok(ExecuteCellsResponse::Accepted),
            400 => {
                let text = response.text().await.unwrap_or_default();
                anyhow::bail!("Execute API bad request: {}", text);
            }
            408 => Ok(ExecuteCellsResponse::PredecessorTimeout),
            409 => {
                let body: serde_json::Value =
                    response.json().await.unwrap_or(serde_json::Value::Null);
                let cell_id = body
                    .get("cell_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(ExecuteCellsResponse::SourceMismatch { cell_id })
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Execute API returned unexpected status {}: {}",
                    status,
                    text
                );
            }
        }
    }
}

/// A cell to execute via the server-driven execute API.
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteCellSpec {
    pub cell_id: String,
    pub source_hash: String,
}

/// Request body for `POST /api/kernels/{kernel_id}/execute`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteCellsRequest {
    pub document_id: String,
    pub cells: Vec<ExecuteCellSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_request_id: Option<String>,
}

/// Response from the execute API.
#[derive(Debug, Clone)]
pub enum ExecuteCellsResponse {
    /// 200 — cells enqueued for execution (fire-and-forget)
    Accepted,
    /// 408 — predecessor request timed out
    PredecessorTimeout,
    /// 409 — cell source has diverged from the provided hash
    SourceMismatch { cell_id: String },
}

/// Compute the MurmurHash2 (seed=0) of cell source as a decimal string,
/// matching jsd's `_source_hash()` implementation. This is the hash the
/// server uses to verify cell source hasn't diverged between request time
/// and execution time.
pub fn compute_source_hash(source: &str) -> String {
    let data = source.as_bytes();
    let m: u32 = 0x5BD1_E995;
    let len = data.len();
    let _h: u32 = (len as u32).wrapping_mul(1); // seed=0, so h = 0 ^ len = len
                                                // Actually: h = (seed ^ len) & 0xFFFFFFFF, seed=0 => h = len as u32
    let mut h: u32 = len as u32;
    let mut i = 0;
    let mut remaining = len;

    while remaining >= 4 {
        let k = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let k = k.wrapping_mul(m);
        let k = k ^ (k >> 24);
        let k = k.wrapping_mul(m);
        h = h.wrapping_mul(m) ^ k;
        remaining -= 4;
        i += 4;
    }

    if remaining == 3 {
        h ^= (data[i + 2] as u32 & 0xFF) << 16;
    }
    if remaining >= 2 {
        h ^= (data[i + 1] as u32 & 0xFF) << 8;
    }
    if remaining >= 1 {
        h ^= data[i] as u32 & 0xFF;
        h = h.wrapping_mul(m);
    }

    h ^= h >> 13;
    h = h.wrapping_mul(m);
    h ^= h >> 15;

    h.to_string()
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
    fn test_compute_source_hash() {
        // MurmurHash2 (seed=0) as decimal string — verified against jsd's Python _source_hash()
        assert_eq!(compute_source_hash(""), "0");
        assert_eq!(compute_source_hash("print('hello')"), "3975440051");
        assert_eq!(compute_source_hash("x = 1\ny = 2"), "749973748");
        assert_eq!(
            compute_source_hash("persistent_var = 3 * 333"),
            "3952812721"
        );
        // Same input must produce same output
        assert_eq!(
            compute_source_hash("print('hello')"),
            compute_source_hash("print('hello')")
        );
        // Different input must produce different output
        assert_ne!(
            compute_source_hash("print('hello')"),
            compute_source_hash("print('world')")
        );
    }

    #[test]
    fn test_execute_cells_request_serialization() {
        let request = ExecuteCellsRequest {
            document_id: "json:notebook:file-123".to_string(),
            cells: vec![ExecuteCellSpec {
                cell_id: "cell-1".to_string(),
                source_hash: "abc123".to_string(),
            }],
            client_id: None,
            request_id: Some("req-1".to_string()),
            previous_request_id: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["document_id"], "json:notebook:file-123");
        assert_eq!(json["cells"][0]["cell_id"], "cell-1");
        assert_eq!(json["cells"][0]["source_hash"], "abc123");
        assert_eq!(json["request_id"], "req-1");
        // Optional None fields should be absent
        assert!(json.get("client_id").is_none());
        assert!(json.get("previous_request_id").is_none());
    }
}
