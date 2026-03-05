//! Remote execution backend using Jupyter Server API

pub mod client;
pub mod websocket;

use crate::execution::types::{ExecutionConfig, ExecutionError, ExecutionResult};
use crate::execution::ExecutionBackend;
use anyhow::{Context, Result};
use client::{JupyterClient, SessionInfo};
use jupyter_protocol::messaging::JupyterMessageContent;
use std::time::Duration;
use tokio::time::timeout;
use websocket::KernelWebSocket;

/// Remote execution backend using Jupyter Server
pub struct RemoteExecutor {
    config: ExecutionConfig,
    server_url: String,
    token: String,
    client: Option<JupyterClient>,
    session: Option<SessionInfo>,
    ws: Option<KernelWebSocket>,
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
        })
    }

    /// Collect outputs from WebSocket messages
    async fn collect_outputs(
        ws: &mut KernelWebSocket,
        msg_id: &str,
        timeout_duration: Duration,
    ) -> Result<(Vec<nbformat::v4::Output>, Option<i64>)> {
        let mut outputs = Vec::new();
        let mut execution_count: Option<i64> = None;
        let mut idle_received = false;
        let mut error_info: Option<ExecutionError> = None;

        // Collect messages until we see idle status
        let collect_result = timeout(timeout_duration, async {
            while !idle_received {
                let msg = match ws.recv_message().await? {
                    Some(m) => m,
                    None => break,
                };

                // Only process messages related to our execution
                if let Some(parent) = &msg.parent_header {
                    if parent.msg_id != msg_id {
                        continue;
                    }
                }

                // Process message content
                match msg.content {
                    JupyterMessageContent::Status(status) => {
                        if matches!(status.execution_state, jupyter_protocol::ExecutionState::Idle) {
                            idle_received = true;
                        }
                    }
                    JupyterMessageContent::StreamContent(stream) => {
                        let output = serde_json::json!({
                            "output_type": "stream",
                            "name": stream.name,
                            "text": stream.text
                        });
                        outputs.push(serde_json::from_value(output)?);
                    }
                    JupyterMessageContent::DisplayData(display) => {
                        let output = serde_json::json!({
                            "output_type": "display_data",
                            "data": display.data,
                            "metadata": display.metadata
                        });
                        outputs.push(serde_json::from_value(output)?);
                    }
                    JupyterMessageContent::ExecuteResult(result) => {
                        execution_count = Some(result.execution_count.0 as i64);
                        let output = serde_json::json!({
                            "output_type": "execute_result",
                            "execution_count": result.execution_count.0 as i64,
                            "data": result.data,
                            "metadata": result.metadata
                        });
                        outputs.push(serde_json::from_value(output)?);
                    }
                    JupyterMessageContent::ErrorOutput(error) => {
                        error_info = Some(ExecutionError {
                            ename: error.ename.clone(),
                            evalue: error.evalue.clone(),
                            traceback: error.traceback.clone(),
                        });
                        let output = nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
                            ename: error.ename,
                            evalue: error.evalue,
                            traceback: error.traceback,
                        });
                        outputs.push(output);
                    }
                    _ => {}
                }
            }

            Ok::<_, anyhow::Error>((outputs, execution_count, error_info))
        })
        .await;

        match collect_result {
            Ok(result) => {
                let (outputs, execution_count, error_info) = result?;
                if let Some(error) = error_info {
                    // Return error result but still include outputs
                    return Ok((outputs, execution_count));
                }
                Ok((outputs, execution_count))
            }
            Err(_) => Err(anyhow::anyhow!(
                "Execution timeout after {:?}",
                timeout_duration
            )),
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
        let kernel_name = self
            .config
            .kernel_name
            .as_deref()
            .unwrap_or("python3");

        // Create a session (this also starts a kernel)
        let session = client
            .create_session("notebook", kernel_name)
            .await
            .context("Failed to create session")?;

        // Connect to kernel via WebSocket
        let ws_url = client.get_ws_url(&session.kernel.id);
        let ws = KernelWebSocket::connect(&ws_url)
            .await
            .context("Failed to connect to kernel WebSocket")?;

        self.client = Some(client);
        self.session = Some(session);
        self.ws = Some(ws);

        Ok(())
    }

    async fn execute_code(&mut self, code: &str) -> Result<ExecutionResult> {
        let ws = self
            .ws
            .as_mut()
            .context("WebSocket not connected")?;

        // Send execute request
        let msg_id = ws
            .send_execute_request(code, !self.config.allow_errors)
            .await?;

        // Collect outputs
        let (outputs, execution_count) =
            Self::collect_outputs(ws, &msg_id, self.config.timeout).await?;

        // Check if any output is an error
        let has_error = outputs.iter().any(|o| matches!(o, nbformat::v4::Output::Error(_)));

        if has_error {
            // Extract error info from error output
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

            Ok(ExecutionResult::error(
                outputs,
                execution_count,
                error_info.unwrap(),
            ))
        } else {
            Ok(ExecutionResult::success(outputs, execution_count))
        }
    }

    async fn stop(&mut self) -> Result<()> {
        // Close WebSocket
        if let Some(ws) = self.ws.take() {
            let _ = ws.close().await;
        }

        // Delete session
        if let (Some(client), Some(session)) = (self.client.as_ref(), self.session.as_ref()) {
            let _ = client.delete_session(&session.id).await;
        }

        Ok(())
    }
}
