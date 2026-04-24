//! Remote execution backend using Jupyter Server API

pub mod client;
pub mod output_conversion;
pub mod session_check;
pub mod websocket;
pub mod ydoc;
pub mod ydoc_notebook_ops;

use crate::execution::types::{ExecutionConfig, ExecutionError, ExecutionResult};
use crate::execution::ExecutionBackend;
use anyhow::{Context, Result};
use client::{JupyterClient, SessionInfo};
use jupyter_protocol::messaging::JupyterMessageContent;
use std::collections::HashSet;
use websocket::KernelWebSocket;
use ydoc::YDocClient;

/// Remote execution backend using Jupyter Server
pub struct RemoteExecutor {
    config: ExecutionConfig,
    server_url: String,
    token: String,
    client: Option<JupyterClient>,
    session: Option<SessionInfo>,
    ws: Option<KernelWebSocket>,
    ydoc: Option<YDocClient>,
    /// Track if we created the session (true) or reused existing (false)
    created_session: bool,
}

impl RemoteExecutor {
    pub fn new(config: ExecutionConfig, server_url: String, token: String) -> Result<Self> {
        Ok(Self {
            config,
            server_url,
            token,
            client: None,
            session: None,
            ws: None,
            ydoc: None,
            created_session: false,
        })
    }

    /// Fetch a single externalized output from the outputs REST API.
    /// Waits 100ms before the first attempt, then uses exponential backoff.
    async fn fetch_output(
        http: &reqwest::Client,
        server_url: &str,
        token: &str,
        url_path: &str,
    ) -> Option<nbformat::v4::Output> {
        let url = format!("{}{}", server_url, url_path);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut backoff_ms = 100u64; // initial delay before first fetch

        // Wait before first attempt to let the server populate the output
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;

        loop {
            if let Ok(resp) = http.get(&url).query(&[("token", token)]).send().await {
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        if let Ok(output) = serde_json::from_str::<nbformat::v4::Output>(&text) {
                            return Some(output);
                        }
                    }
                }
            }
            if tokio::time::Instant::now() > deadline {
                return None;
            }
            backoff_ms = (backoff_ms * 2).min(1000);
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        }
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for RemoteExecutor {
    async fn start(&mut self) -> Result<()> {
        // Create HTTP client
        let client = JupyterClient::new(self.server_url.clone(), self.token.clone())?;

        // Test connection
        client
            .test_connection()
            .await
            .context("Failed to connect to Jupyter Server")?;

        // Determine kernel name
        let kernel_name = self.config.kernel_name.as_deref().unwrap_or("python3");

        // Try to find an existing session first
        let sessions = client.list_sessions().await?;

        // Try to find and reuse existing session by notebook path
        let (session, created) = if let Some(ref notebook_path) = self.config.notebook_path {
            if let Some(existing) = sessions.iter().find(|s| s.path == *notebook_path) {
                // Restart kernel if requested
                if self.config.restart_kernel {
                    client
                        .restart_kernel(&existing.kernel.id)
                        .await
                        .context("Failed to restart kernel")?;
                    // Wait for kernel to be ready using short-interval polling with backoff
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                    let mut poll_ms = 200u64;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
                        let info = client.get_kernel(&existing.kernel.id).await?;
                        if info.execution_state == "idle" {
                            break;
                        }
                        if tokio::time::Instant::now() > deadline {
                            anyhow::bail!(
                                "Timeout waiting for kernel to become ready after restart"
                            );
                        }
                        poll_ms = (poll_ms * 2).min(5_000);
                    }
                }
                // Return the existing session directly - no new session/kernel creation
                (existing.clone(), false)
            } else {
                if self.config.restart_kernel {
                    eprintln!("No existing session found; new kernel will start clean.");
                }
                let s = client
                    .create_session(notebook_path, kernel_name)
                    .await
                    .context("Failed to create session")?;
                (s, true)
            }
        } else {
            let s = client
                .create_session("notebook", kernel_name)
                .await
                .context("Failed to create session")?;
            (s, true)
        };

        self.created_session = created;

        // Connect to kernel via WebSocket with session_id
        let ws_url = client.get_ws_url(&session.kernel.id, Some(&session.id));
        let ws = KernelWebSocket::connect(&ws_url)
            .await
            .context("Failed to connect to kernel WebSocket")?;

        self.client = Some(client);
        self.session = Some(session);
        self.ws = Some(ws);

        // Connect Y.js client for observing outputs during execution
        if let Some(ref notebook_path) = self.config.notebook_path {
            let ydoc = YDocClient::connect(
                self.server_url.clone(),
                self.token.clone(),
                notebook_path.clone(),
            )
            .await
            .context("Failed to connect Y.js client for output observation")?;
            self.ydoc = Some(ydoc);
        }

        Ok(())
    }

    async fn execute_code(
        &mut self,
        code: &str,
        cell_id: Option<&str>,
        cell_index: Option<usize>,
        on_output: Option<&crate::execution::OutputCallback>,
    ) -> Result<ExecutionResult> {
        let ws = self.ws.as_mut().context("WebSocket not connected")?;
        let cell_idx = cell_index.context("cell_index required for remote execution")?;
        let ydoc = self.ydoc.as_mut().context("Y.js client not connected")?;
        let http = reqwest::Client::new();

        // 1. Fire execute request
        let msg_id = ws
            .send_execute_request(code, !self.config.allow_errors, cell_id)
            .await?;

        // 2. Watch for changes on the ydoc for this cell
        let mut outputs: Vec<nbformat::v4::Output> = Vec::new();
        let mut fetched_urls: HashSet<String> = HashSet::new();
        let mut seen_indices: HashSet<usize> = HashSet::new();
        let mut idle_received = false;
        let mut expected_ec: Option<i64> = None;
        let deadline = tokio::time::Instant::now() + self.config.timeout;

        loop {
            // 3. Check ydoc state before blocking — the update may already
            //    have been applied in a previous iteration.
            let cell_data = ydoc.read_cell_outputs(cell_idx).ok();
            let ec = cell_data.as_ref().and_then(|d| d.execution_count);
            let ec_ready = expected_ec.is_some() && ec == expected_ec;

            if ec_ready {
                if let Some(ref cell_data) = cell_data {
                    for (idx, url_path) in &cell_data.externalized_urls {
                        if fetched_urls.insert(url_path.clone()) {
                            seen_indices.insert(*idx);
                            if let Some(output) =
                                Self::fetch_output(&http, &self.server_url, &self.token, url_path)
                                    .await
                            {
                                if let Some(cb) = &on_output {
                                    cb(&output);
                                }
                                outputs.push(output);
                            }
                        }
                    }
                    for (idx, output) in &cell_data.inline_outputs {
                        if seen_indices.insert(*idx) {
                            if let Some(cb) = &on_output {
                                cb(output);
                            }
                            outputs.push(output.clone());
                        }
                    }
                }

                if idle_received {
                    let has_error = outputs
                        .iter()
                        .any(|o| matches!(o, nbformat::v4::Output::Error(_)));
                    let error_info = outputs.iter().find_map(|o| {
                        if let nbformat::v4::Output::Error(err) = o {
                            Some(ExecutionError {
                                ename: err.ename.clone(),
                                evalue: err.evalue.clone(),
                                traceback: err.traceback.clone(),
                            })
                        } else {
                            None
                        }
                    });
                    return if has_error {
                        Ok(ExecutionResult::error(outputs, ec, error_info.unwrap()))
                    } else {
                        Ok(ExecutionResult::success(outputs, ec))
                    };
                }
            }

            // 4. Wait for new messages
            if idle_received {
                match tokio::time::timeout_at(deadline, ydoc.recv_update()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(e).context("Y.js update error"),
                    Err(_) => break,
                }
            } else {
                tokio::select! {
                    kernel_msg = ws.recv_message() => {
                        if let Some(msg) = kernel_msg? {
                            let is_ours = msg.parent_header.as_ref()
                                .map(|h| h.msg_id == msg_id).unwrap_or(false);
                            if is_ours {
                                match &msg.content {
                                    JupyterMessageContent::ExecuteInput(input) => {
                                        expected_ec = Some(input.execution_count.0 as i64);
                                    }
                                    JupyterMessageContent::Status(status) => {
                                        if matches!(status.execution_state,
                                            jupyter_protocol::ExecutionState::Idle) {
                                            idle_received = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    ydoc_result = ydoc.recv_update() => {
                        ydoc_result.context("Y.js update error")?;
                    }
                }
            }
        }

        let ec = ydoc
            .read_cell_outputs(cell_idx)
            .ok()
            .and_then(|c| c.execution_count);
        Ok(ExecutionResult::success(outputs, ec))
    }

    async fn stop(&mut self) -> Result<()> {
        // Close kernel WebSocket
        if let Some(ws) = self.ws.take() {
            let _ = ws.close().await;
        }

        // Close Y.js WebSocket
        if let Some(ydoc) = self.ydoc.take() {
            let _ = ydoc.close().await;
        }

        // Don't delete session - let it persist for reuse in subsequent executions
        // This maintains parity with JupyterLab's behavior where sessions/kernels
        // stay alive across multiple cell executions.

        Ok(())
    }
}
