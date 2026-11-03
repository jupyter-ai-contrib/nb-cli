//! HTTP client for Jupyter Server REST API

use super::TokenAuth;
use crate::execution::server_url;
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
    /// Create a new Jupyter Server client. The URL is validated and
    /// normalized eagerly so a bad URL fails here, not at first use.
    pub fn new(server_url: String, token: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            base_url: server_url::normalize(&server_url)?,
            token,
            client,
        })
    }

    /// Validate access to a protected API, not the anonymous `/api` endpoint.
    pub async fn test_connection(&self) -> Result<()> {
        self.list_kernels()
            .await
            .context("Failed to validate Jupyter Server connection")?;
        Ok(())
    }

    /// Get kernel info by ID
    pub async fn get_kernel(&self, kernel_id: &str) -> Result<KernelInfo> {
        let url = server_url::endpoint(&self.base_url, &["api", "kernels", kernel_id])?;
        let response = self
            .client
            .get(url)
            .token_auth(&self.token)
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
    pub async fn list_kernels(&self) -> Result<Vec<KernelInfo>> {
        let url = server_url::endpoint(&self.base_url, &["api", "kernels"])?;
        let response = self
            .client
            .get(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "kernels"])?;
        let payload = serde_json::json!({
            "name": kernel_name
        });

        let response = self
            .client
            .post(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "sessions"])?;
        let response = self
            .client
            .get(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "sessions"])?;
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
            .post(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "sessions"])?;
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
            .post(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "kernels", kernel_id, "restart"])?;
        let response = self
            .client
            .post(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "sessions", session_id])?;
        let response = self
            .client
            .delete(url)
            .token_auth(&self.token)
            .send()
            .await
            .context("Failed to delete session")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to delete session: HTTP {}", response.status());
        }

        Ok(())
    }

    fn contents_url(&self, path: &str) -> Result<String> {
        Ok(server_url::endpoint_with_path(&self.base_url, &["api", "contents"], path)?.to_string())
    }

    /// Read a notebook from the server via the Contents API.
    pub async fn get_notebook(&self, path: &str) -> Result<nbformat::v4::Notebook> {
        let url = self.contents_url(path)?;

        let response = self
            .client
            .get(&url)
            .token_auth(&self.token)
            .query(&[("content", "1"), ("type", "notebook")])
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
            .token_auth(&self.token)
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
        let mut url =
            server_url::ws_endpoint(&self.base_url, &["api", "kernels", kernel_id, "channels"])
                .expect("server URL was validated at construction");
        if let Some(sid) = session_id {
            url.query_pairs_mut().append_pair("session_id", sid);
        }
        url.to_string()
    }

    /// Probe whether the server supports `POST /api/kernels/{id}/execute`.
    /// Sends a minimal invalid request (empty cells) and checks the status:
    /// - 400 or 200 means the endpoint exists (server understands the route)
    /// - 404 or 405 means the endpoint is absent (fall back to kernel WS)
    pub async fn probe_execute_api(&self, kernel_id: &str) -> Result<bool> {
        let url = server_url::endpoint(&self.base_url, &["api", "kernels", kernel_id, "execute"])?;
        let payload = serde_json::json!({
            "document_id": "",
            "cells": []
        });

        let response = self
            .client
            .post(url)
            .token_auth(&self.token)
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
        let url = server_url::endpoint(&self.base_url, &["api", "kernels", kernel_id, "execute"])?;

        let response = self
            .client
            .post(url)
            .token_auth(&self.token)
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

    async fn serve_response(listener: tokio::net::TcpListener, status: &str, body: &str) -> String {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut request = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            request.push_str(&line);
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
        request
    }

    #[tokio::test]
    async fn get_notebook_authenticates_without_url_credentials() {
        use std::time::Duration;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let c = client(&format!("http://{addr}"));
        let body = r#"{"content":{"cells":[],"metadata":{},"nbformat":4,"nbformat_minor":5}}"#;
        let server = serve_response(listener, "200 OK", body);

        let (request, notebook) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(server, c.get_notebook("example.ipynb"))
        })
        .await
        .expect("Contents API request timed out");
        assert!(notebook.unwrap().cells.is_empty());

        let mut lines = request.lines();
        let request_line = lines.next().unwrap();
        let mut parts = request_line.split_whitespace();
        assert_eq!(parts.next(), Some("GET"));
        let url = url::Url::parse(&format!("http://{addr}{}", parts.next().unwrap())).unwrap();
        assert_eq!(url.path(), "/api/contents/example.ipynb");
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert!(!query.contains_key("token"));
        assert_eq!(query.get("content").map(|v| v.as_ref()), Some("1"));
        assert_eq!(query.get("type").map(|v| v.as_ref()), Some("notebook"));
        let authorization = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.trim());
        assert_eq!(authorization, Some("token tok"));
    }

    #[tokio::test]
    async fn test_connection_validates_protected_api_response() {
        use std::time::Duration;

        for (token, status, body, expected_error) in [
            ("tok", "200 OK", "[]", None),
            ("invalid", "403 Forbidden", "{}", Some("HTTP 403")),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let c = JupyterClient::new(format!("http://{addr}"), token.into()).unwrap();
            let server = serve_response(listener, status, body);
            let (request, result) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(server, c.test_connection())
            })
            .await
            .expect("Connection validation timed out");

            match expected_error {
                Some(message) => {
                    let error = format!("{:#}", result.unwrap_err());
                    assert!(error.contains(message), "{error}");
                }
                None => result.unwrap(),
            }
            assert!(request.starts_with("GET /api/kernels HTTP/1.1\r\n"));
        }
    }

    #[test]
    fn test_get_ws_url_formats() {
        let c = client("http://127.0.0.1:8888");

        let url = c.get_ws_url("kid1", None);
        assert_eq!(url, "ws://127.0.0.1:8888/api/kernels/kid1/channels");

        let url = c.get_ws_url("kid2", Some("sid42"));
        assert_eq!(
            url,
            "ws://127.0.0.1:8888/api/kernels/kid2/channels?session_id=sid42"
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
    fn test_get_ws_url_preserves_base_url_prefix() {
        // Server mounted under a base_url (e.g. /jupyter) — the ws URL must
        // keep the prefix, not rebuild from host/port.
        let c = client("https://host/jupyter");
        let url = c.get_ws_url("kid1", None);
        assert_eq!(url, "wss://host/jupyter/api/kernels/kid1/channels");

        let c = client("http://host:8888/foo/bar/");
        let url = c.get_ws_url("kid2", Some("sid"));
        assert_eq!(
            url,
            "ws://host:8888/foo/bar/api/kernels/kid2/channels?session_id=sid"
        );
    }

    #[test]
    fn test_new_rejects_invalid_server_url() {
        assert!(JupyterClient::new("not a url".to_string(), "t".to_string()).is_err());
        assert!(JupyterClient::new("host:8888".to_string(), "t".to_string()).is_err());
        assert!(JupyterClient::new("ftp://host".to_string(), "t".to_string()).is_err());
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
