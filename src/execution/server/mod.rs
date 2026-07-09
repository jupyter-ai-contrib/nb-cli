//! Remote execution backend using Jupyter Server API

pub mod client;
mod kernel_ws_execution;
pub mod output_conversion;
pub mod websocket;
pub mod ydoc;
mod ydoc_execution;
pub mod ydoc_notebook_ops;

use crate::execution::types::{ExecutionConfig, ExecutionResult};
use crate::execution::ExecutionBackend;
use anyhow::{Context, Result};
use client::{JupyterClient, SessionInfo};
use websocket::KernelWebSocket;
use ydoc::YDocClient;

/// Remote execution backend using Jupyter Server
pub struct RemoteExecutor {
    pub(super) config: ExecutionConfig,
    pub(super) server_url: String,
    pub(super) token: String,
    pub(super) client: Option<JupyterClient>,
    pub(super) session: Option<SessionInfo>,
    pub(super) ws: Option<KernelWebSocket>,
    pub(super) ydoc: Option<YDocClient>,
    /// Track if we created the session (true) or reused existing (false)
    pub(super) created_session: bool,
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

        // Connect Y.js client for observing outputs during execution.
        // The cached ydoc_available is a routing hint, not a gate: Some(false)
        // skips the attempt, anything else tries Y.js and falls back to the
        // kernel-WS path on the definitive backend-absent signal. Transient
        // errors stay hard so a flaky collaboration server is never silently
        // downgraded.
        if self.config.ydoc_available != Some(false) {
            if let Some(ref notebook_path) = self.config.notebook_path {
                match YDocClient::connect(
                    self.server_url.clone(),
                    self.token.clone(),
                    notebook_path.clone(),
                )
                .await
                {
                    Ok(ydoc) => {
                        self.ydoc = Some(ydoc);
                    }
                    Err(e) if ydoc::is_yjs_unavailable(&e) => {
                        if self.config.ydoc_available == Some(true) {
                            eprintln!(
                                "Collaboration backend no longer found on server; \
                                 using direct kernel output. Run 'nb connect' to refresh."
                            );
                        }
                    }
                    Err(e) => {
                        return Err(e)
                            .context("Failed to connect Y.js client for output observation");
                    }
                }
            }
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
        if self.ydoc.is_some() {
            ydoc_execution::execute_code_ydoc(self, code, cell_id, cell_index, on_output).await
        } else {
            kernel_ws_execution::execute_code_kernel_ws(self, code, cell_id, on_output).await
        }
    }

    fn server_persists_outputs(&self) -> bool {
        // With a Y.js room attached, the server observes and saves outputs
        // itself; without one the caller must persist via the Contents API.
        self.ydoc.is_some()
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
