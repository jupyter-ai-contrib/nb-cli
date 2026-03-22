use super::discovery::find_kernel;
use crate::execution::types::{ExecutionConfig, ExecutionError, ExecutionResult};
use crate::execution::ExecutionBackend;
use anyhow::{Context, Result};
use jupyter_protocol::{
    ConnectionInfo, ExecuteRequest, ExecutionState, JupyterKernelspec, JupyterMessage,
    JupyterMessageContent,
};
use std::path::PathBuf;

/// Local execution backend using runtimelib
///
/// This implementation uses runtimelib to communicate directly with Jupyter kernels
/// over ZeroMQ. It manages the kernel lifecycle and maintains state across cells.
pub struct LocalExecutor {
    config: ExecutionConfig,
    kernel_name: String,
    kernel_spec: Option<runtimelib::KernelspecDir>,

    // Kernel process and connections
    kernel_process: Option<tokio::process::Child>,
    connection_info: Option<ConnectionInfo>,
    shell_socket: Option<runtimelib::ClientShellConnection>,
    iopub_socket: Option<runtimelib::ClientIoPubConnection>,
    session_id: String,

    // Working directory for kernel execution (notebook directory)
    cwd: Option<PathBuf>,
}

impl LocalExecutor {
    /// Create a new local executor
    pub fn new(config: ExecutionConfig) -> Result<Self> {
        // Extract working directory from config if notebook_path is set
        let cwd = config
            .notebook_path
            .as_ref()
            .and_then(|path| std::path::Path::new(path).parent().map(|p| p.to_path_buf()));

        Ok(Self {
            config,
            kernel_name: String::new(),
            kernel_spec: None,
            kernel_process: None,
            connection_info: None,
            shell_socket: None,
            iopub_socket: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            cwd,
        })
    }

    /// Execute a single code cell and collect results
    async fn execute_cell(&mut self, code: &str) -> Result<ExecutionResult> {
        let shell_socket = self
            .shell_socket
            .as_mut()
            .context("Shell socket not initialized")?;
        let iopub_socket = self
            .iopub_socket
            .as_mut()
            .context("IOPub socket not initialized")?;

        // Create execute request
        let execute_request = ExecuteRequest::new(code.to_string());
        let execute_request: JupyterMessage = execute_request.into();
        let request_id = execute_request.header.msg_id.clone();

        // Send execute request
        shell_socket
            .send(execute_request)
            .await
            .context("Failed to send execute request")?;

        // Collect outputs from IOPub
        let mut outputs = Vec::new();
        let mut execution_count = None;
        let mut error_info: Option<ExecutionError> = None;

        let timeout = self.config.timeout;
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            // Check timeout
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!("Execution timeout after {:?}", timeout);
            }

            // Read message with timeout
            let message = match tokio::time::timeout_at(deadline, iopub_socket.read()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    anyhow::bail!("Error reading IOPub message: {}", e);
                }
                Err(_) => {
                    anyhow::bail!("Timeout reading IOPub message");
                }
            };

            // Only process messages related to our request
            let is_our_message = message
                .parent_header
                .as_ref()
                .map(|h| h.msg_id.as_str() == request_id.as_str())
                .unwrap_or(false);

            if !is_our_message {
                continue;
            }

            match message.content {
                JupyterMessageContent::Status(status) => {
                    if status.execution_state == ExecutionState::Idle {
                        // Execution complete on IOPub
                        break;
                    }
                }
                JupyterMessageContent::StreamContent(stream) => {
                    // Convert to nbformat output
                    let name = match stream.name {
                        jupyter_protocol::Stdio::Stdout => "stdout".to_string(),
                        jupyter_protocol::Stdio::Stderr => "stderr".to_string(),
                    };
                    let output = nbformat::v4::Output::Stream {
                        name,
                        text: nbformat::v4::MultilineString(stream.text),
                    };
                    outputs.push(output);
                }
                JupyterMessageContent::ExecuteResult(result) => {
                    execution_count = Some(result.execution_count.value() as i64);
                    // Convert to nbformat output
                    let json = serde_json::json!({
                        "output_type": "execute_result",
                        "execution_count": result.execution_count.value(),
                        "data": result.data,
                        "metadata": result.metadata
                    });
                    if let Ok(output) = serde_json::from_value(json) {
                        outputs.push(output);
                    }
                }
                JupyterMessageContent::DisplayData(display) => {
                    // Convert to nbformat output
                    let json = serde_json::json!({
                        "output_type": "display_data",
                        "data": display.data,
                        "metadata": display.metadata
                    });
                    if let Ok(output) = serde_json::from_value(json) {
                        outputs.push(output);
                    }
                }
                JupyterMessageContent::ErrorOutput(error) => {
                    // Store error info
                    error_info = Some(ExecutionError {
                        ename: error.ename.clone(),
                        evalue: error.evalue.clone(),
                        traceback: error.traceback.clone(),
                    });
                    // Also add as output
                    let output = nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
                        ename: error.ename,
                        evalue: error.evalue,
                        traceback: error.traceback,
                    });
                    outputs.push(output);
                }
                _ => {
                    // Ignore other message types
                }
            }
        }

        // Read execute_reply from shell channel to get execution_count
        // This is important for cells that don't produce output (no ExecuteResult)
        match tokio::time::timeout_at(deadline, shell_socket.read()).await {
            Ok(Ok(reply)) => {
                if let JupyterMessageContent::ExecuteReply(reply_content) = reply.content {
                    // Use execution_count from reply if we don't have one yet
                    if execution_count.is_none() {
                        execution_count = Some(reply_content.execution_count.value() as i64);
                    }
                }
            }
            Ok(Err(e)) => {
                eprintln!("Warning: Failed to read execute_reply: {}", e);
            }
            Err(_) => {
                eprintln!("Warning: Timeout reading execute_reply");
            }
        }

        // Build result
        if let Some(error) = error_info {
            Ok(ExecutionResult::error(outputs, execution_count, error))
        } else {
            Ok(ExecutionResult::success(outputs, execution_count))
        }
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for LocalExecutor {
    async fn start(&mut self) -> Result<()> {
        // Find kernel
        let (kernel_name, kernel_spec_path) = find_kernel(
            self.config.kernel_name.as_deref(),
            None, // Notebook kernel will be passed from command
        )?;
        self.kernel_name = kernel_name.clone();

        // Read kernelspec from the found path
        let kernel_json_path = kernel_spec_path.join("kernel.json");
        let kernel_json_content =
            tokio::fs::read_to_string(&kernel_json_path)
                .await
                .context(format!(
                    "Failed to read kernel spec from {}",
                    kernel_json_path.display()
                ))?;
        let kernelspec: JupyterKernelspec =
            serde_json::from_str(&kernel_json_content).context("Failed to parse kernel.json")?;

        let kernel_spec = runtimelib::KernelspecDir {
            kernel_name: kernel_name.clone(),
            path: kernel_spec_path,
            kernelspec,
        };

        self.kernel_spec = Some(kernel_spec.clone());

        // Allocate ports for ZeroMQ
        let ip = std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));
        let ports = runtimelib::peek_ports(ip, 5)
            .await
            .context("Failed to allocate ports")?;

        if ports.len() != 5 {
            anyhow::bail!("Failed to allocate 5 ports, got {}", ports.len());
        }

        // Create connection info
        let connection_info = ConnectionInfo {
            transport: jupyter_protocol::connection_info::Transport::TCP,
            ip: ip.to_string(),
            stdin_port: ports[0],
            control_port: ports[1],
            hb_port: ports[2],
            shell_port: ports[3],
            iopub_port: ports[4],
            signature_scheme: "hmac-sha256".to_string(),
            key: uuid::Uuid::new_v4().to_string(),
            kernel_name: Some(kernel_name.clone()),
        };

        // Write connection file
        let runtime_dir = runtimelib::dirs::runtime_dir();
        tokio::fs::create_dir_all(&runtime_dir)
            .await
            .context("Failed to create runtime directory")?;

        let connection_path = runtime_dir.join(format!("kernel-nb-cli-{}.json", self.session_id));
        let content = serde_json::to_string(&connection_info)
            .context("Failed to serialize connection info")?;
        tokio::fs::write(&connection_path, content)
            .await
            .context("Failed to write connection file")?;

        // Determine working directory
        let working_dir = self
            .cwd
            .as_ref()
            .map(|p| p.as_path())
            .unwrap_or_else(|| std::path::Path::new("."));

        // Launch kernel process
        let mut process = kernel_spec
            .command(&connection_path, None, None)
            .context("Failed to create kernel command")?
            .current_dir(working_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn()
            .context("Failed to spawn kernel process")?;

        // Wait a bit for kernel to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Check if kernel is still running
        if let Ok(Some(status)) = process.try_wait() {
            anyhow::bail!("Kernel process exited immediately with status: {}", status);
        }

        self.kernel_process = Some(process);
        self.connection_info = Some(connection_info.clone());

        // Create ZeroMQ connections
        let iopub_socket =
            runtimelib::create_client_iopub_connection(&connection_info, "", &self.session_id)
                .await
                .context("Failed to create IOPub socket")?;

        let identity = runtimelib::peer_identity_for_session(&self.session_id)
            .context("Failed to create peer identity")?;
        let shell_socket = runtimelib::create_client_shell_connection_with_identity(
            &connection_info,
            &self.session_id,
            identity,
        )
        .await
        .context("Failed to create shell socket")?;

        self.iopub_socket = Some(iopub_socket);
        self.shell_socket = Some(shell_socket);

        Ok(())
    }

    async fn execute_code(
        &mut self,
        code: &str,
        _cell_id: Option<&str>,
    ) -> Result<ExecutionResult> {
        self.execute_cell(code).await
    }

    async fn stop(&mut self) -> Result<()> {
        // Close sockets
        self.shell_socket = None;
        self.iopub_socket = None;

        // Terminate kernel process
        if let Some(mut process) = self.kernel_process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }

        // Clean up connection file
        if self.connection_info.is_some() {
            let runtime_dir = runtimelib::dirs::runtime_dir();
            let connection_path =
                runtime_dir.join(format!("kernel-nb-cli-{}.json", self.session_id));
            let _ = tokio::fs::remove_file(&connection_path).await;
        }

        Ok(())
    }
}
