//! Remote kernel (Jupyter Kernel Gateway) execution backend.

use crate::execution::remote::websocket::KernelWebSocket;
use crate::execution::types::{ExecutionConfig, ExecutionError, ExecutionResult};
use crate::execution::{ExecutionBackend, OutputCallback};
use anyhow::{Context, Result};
use jupyter_protocol::messaging::JupyterMessageContent;
use serde::Deserialize;

/// Minimal subset of the Jupyter Kernel Gateway's `/api/kernels` response.
/// We only need the kernel id; the gateway returns more fields but we ignore them.
#[derive(Deserialize)]
struct GatewayKernel {
    id: String,
}

pub struct RemoteKernelExecutor {
    config: ExecutionConfig,
    gateway_url: String,
    token: String,
    kernel_id: Option<String>,
    auth_scheme: String,
    ws: Option<KernelWebSocket>,
    http_client: reqwest::Client,
}

impl RemoteKernelExecutor {
    pub fn new(
        config: ExecutionConfig,
        gateway_url: String,
        token: String,
        kernel_id: Option<String>,
        auth_scheme: String,
    ) -> Result<Self> {
        Ok(Self {
            config,
            gateway_url,
            token,
            kernel_id,
            auth_scheme,
            ws: None,
            http_client: reqwest::Client::new(),
        })
    }

    fn auth_header(&self) -> String {
        format!("{} {}", self.auth_scheme, self.token)
    }

    async fn discover_kernel_id(&self) -> Result<String> {
        let url = format!("{}/api/kernels", self.gateway_url.trim_end_matches('/'));
        let response = self
            .http_client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to connect to kernel gateway at {}. Check that the URL is reachable.",
                    self.gateway_url
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            match status.as_u16() {
                401 | 403 => anyhow::bail!(
                    "Gateway rejected authentication ({}). Check --gateway-token and --gateway-auth-scheme.",
                    status
                ),
                404 => anyhow::bail!(
                    "Gateway endpoint /api/kernels not found ({}). Check that --gateway points to a Jupyter Kernel Gateway.",
                    status
                ),
                _ => anyhow::bail!("Gateway returned status {} for GET /api/kernels", status),
            }
        }

        let kernels: Vec<GatewayKernel> = response
            .json()
            .await
            .context("Failed to parse kernel list from gateway")?;

        if let Some(kernel) = kernels.into_iter().next() {
            return Ok(kernel.id);
        }

        eprintln!("No kernels running on gateway; starting a new one...");
        self.start_kernel().await
    }

    async fn start_kernel(&self) -> Result<String> {
        let url = format!("{}/api/kernels", self.gateway_url.trim_end_matches('/'));
        let mut body = serde_json::Map::new();
        if let Some(name) = self.config.kernel_name.as_ref() {
            body.insert("name".to_string(), serde_json::Value::String(name.clone()));
        }

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to start a kernel on gateway at {}",
                    self.gateway_url
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            match status.as_u16() {
                401 | 403 => anyhow::bail!(
                    "Gateway rejected authentication ({}) when starting a kernel.",
                    status
                ),
                _ => anyhow::bail!(
                    "Gateway returned status {} when starting a kernel. Pass --kernel-id to use an existing kernel. Body: {}",
                    status,
                    body
                ),
            }
        }

        let kernel: GatewayKernel = response
            .json()
            .await
            .context("Failed to parse kernel response from gateway")?;
        Ok(kernel.id)
    }

    fn ws_url_for_kernel(&self, kernel_id: &str) -> String {
        let trimmed = self.gateway_url.trim_end_matches('/');
        let base = if let Some(rest) = trimmed.strip_prefix("https://") {
            format!("wss://{}", rest)
        } else if let Some(rest) = trimmed.strip_prefix("http://") {
            format!("ws://{}", rest)
        } else {
            trimmed.to_string()
        };
        format!("{}/api/kernels/{}/channels", base, kernel_id)
    }

    async fn execute_cell(
        &mut self,
        code: &str,
        on_output: Option<&OutputCallback>,
    ) -> Result<ExecutionResult> {
        let ws = self.ws.as_mut().context("WebSocket not initialized")?;

        let stop_on_error = !self.config.allow_errors;
        let msg_id = ws
            .send_execute_request(code, stop_on_error, None)
            .await
            .context("Failed to send execute request")?;

        let mut outputs = Vec::new();
        let mut execution_count: Option<i64> = None;
        let mut error_info: Option<ExecutionError> = None;
        let mut saw_busy = false;

        let timeout = self.config.timeout;
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("Execution timeout after {:?}", timeout);
            }

            let recv_result = tokio::time::timeout_at(deadline, ws.recv_message()).await;
            let message = match recv_result {
                Ok(Ok(Some(msg))) => msg,
                Ok(Ok(None)) => anyhow::bail!("WebSocket connection closed unexpectedly"),
                Ok(Err(e)) => return Err(e).context("Error reading WebSocket message"),
                Err(_) => anyhow::bail!("Timeout reading WebSocket message"),
            };

            let is_our_message = message
                .parent_header
                .as_ref()
                .map(|h| h.msg_id == msg_id)
                .unwrap_or(false);

            if !is_our_message {
                continue;
            }

            match &message.content {
                JupyterMessageContent::Status(status) => match status.execution_state {
                    jupyter_protocol::ExecutionState::Busy => {
                        saw_busy = true;
                    }
                    jupyter_protocol::ExecutionState::Idle if saw_busy => {
                        break;
                    }
                    _ => {}
                },
                JupyterMessageContent::StreamContent(stream) => {
                    let name = match stream.name {
                        jupyter_protocol::Stdio::Stdout => "stdout".to_string(),
                        jupyter_protocol::Stdio::Stderr => "stderr".to_string(),
                    };
                    let output = nbformat::v4::Output::Stream {
                        name,
                        text: nbformat::v4::MultilineString(stream.text.clone()),
                    };
                    if let Some(cb) = on_output {
                        cb(&output);
                    }
                    outputs.push(output);
                }
                JupyterMessageContent::ExecuteResult(result) => {
                    execution_count = Some(result.execution_count.value() as i64);
                    let json = serde_json::json!({
                        "output_type": "execute_result",
                        "execution_count": result.execution_count.value(),
                        "data": result.data,
                        "metadata": result.metadata
                    });
                    if let Ok(output) = serde_json::from_value::<nbformat::v4::Output>(json) {
                        if let Some(cb) = on_output {
                            cb(&output);
                        }
                        outputs.push(output);
                    }
                }
                JupyterMessageContent::DisplayData(display) => {
                    let json = serde_json::json!({
                        "output_type": "display_data",
                        "data": display.data,
                        "metadata": display.metadata
                    });
                    if let Ok(output) = serde_json::from_value::<nbformat::v4::Output>(json) {
                        if let Some(cb) = on_output {
                            cb(&output);
                        }
                        outputs.push(output);
                    }
                }
                JupyterMessageContent::ErrorOutput(error) => {
                    error_info = Some(ExecutionError {
                        ename: error.ename.clone(),
                        evalue: error.evalue.clone(),
                        traceback: error.traceback.clone(),
                    });
                    let output = nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
                        ename: error.ename.clone(),
                        evalue: error.evalue.clone(),
                        traceback: error.traceback.clone(),
                    });
                    if let Some(cb) = on_output {
                        cb(&output);
                    }
                    outputs.push(output);
                }
                JupyterMessageContent::ExecuteReply(reply) if execution_count.is_none() => {
                    execution_count = Some(reply.execution_count.value() as i64);
                }
                _ => {}
            }
        }

        if let Some(error) = error_info {
            Ok(ExecutionResult::error(outputs, execution_count, error))
        } else {
            Ok(ExecutionResult::success(outputs, execution_count))
        }
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for RemoteKernelExecutor {
    async fn start(&mut self) -> Result<()> {
        let kernel_id = match self.kernel_id.take() {
            Some(id) => id,
            None => self
                .discover_kernel_id()
                .await
                .context("Failed to discover kernel from gateway")?,
        };

        let ws_url = self.ws_url_for_kernel(&kernel_id);
        let auth_value = self.auth_header();
        let ws = KernelWebSocket::connect_with_auth(&ws_url, &auth_value)
            .await
            .with_context(|| format!("Failed to connect to kernel {}", kernel_id))?;

        self.ws = Some(ws);
        self.kernel_id = Some(kernel_id);

        Ok(())
    }

    async fn execute_code(
        &mut self,
        code: &str,
        _cell_id: Option<&str>,
        _cell_index: Option<usize>,
        on_output: Option<&OutputCallback>,
    ) -> Result<ExecutionResult> {
        self.execute_cell(code, on_output).await
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(ws) = self.ws.take() {
            let _ = ws.close().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executor(gateway_url: &str, token: &str, auth_scheme: &str) -> RemoteKernelExecutor {
        RemoteKernelExecutor::new(
            ExecutionConfig::default(),
            gateway_url.to_string(),
            token.to_string(),
            None,
            auth_scheme.to_string(),
        )
        .expect("constructor should not fail")
    }

    #[test]
    fn auth_header_uses_configured_scheme() {
        let exec = executor("https://gw.example.com", "abc", "token");
        assert_eq!(exec.auth_header(), "token abc");

        let exec = executor("https://gw.example.com", "xyz", "Bearer");
        assert_eq!(exec.auth_header(), "Bearer xyz");
    }

    #[test]
    fn ws_url_swaps_scheme_and_appends_channels_path() {
        let exec = executor("http://host:8888", "t", "token");
        assert_eq!(
            exec.ws_url_for_kernel("abc"),
            "ws://host:8888/api/kernels/abc/channels"
        );

        let exec = executor("https://gw.example.com", "t", "token");
        assert_eq!(
            exec.ws_url_for_kernel("abc"),
            "wss://gw.example.com/api/kernels/abc/channels"
        );

        // Trailing slash on the gateway URL must not produce a double slash.
        let exec = executor("https://gw.example.com/", "t", "token");
        assert_eq!(
            exec.ws_url_for_kernel("abc"),
            "wss://gw.example.com/api/kernels/abc/channels"
        );

        // "http" inside a path segment must not be rewritten — only the scheme is.
        let exec = executor("https://gw.example.com/httpapi", "t", "token");
        assert_eq!(
            exec.ws_url_for_kernel("abc"),
            "wss://gw.example.com/httpapi/api/kernels/abc/channels"
        );
    }
}
