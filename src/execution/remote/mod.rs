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

    /// Y.js-based output collection (collaboration server)
    async fn execute_code_ydoc(
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

        let msg_id = ws
            .send_execute_request(code, !self.config.allow_errors, cell_id)
            .await?;

        let mut outputs: Vec<nbformat::v4::Output> = Vec::new();
        let mut fetched_urls: HashSet<String> = HashSet::new();
        let mut seen_indices: HashSet<usize> = HashSet::new();
        let mut idle_received = false;
        let mut expected_ec: Option<i64> = None;
        let deadline = tokio::time::Instant::now() + self.config.timeout;

        loop {
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

            if idle_received {
                match tokio::time::timeout_at(deadline, ydoc.recv_update()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => return Err(e).context("Y.js update error"),
                    // Kernel already reported idle, so execution finished;
                    // only output sync timed out. Return what we collected
                    // instead of erroring (unlike the pre-idle timeout below).
                    Err(_) => break,
                }
            } else {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => {
                        anyhow::bail!(
                            "Cell execution timed out after {:?}",
                            self.config.timeout
                        );
                    }
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

    /// Kernel-WS-only output collection (vanilla jupyter_server, no Y.js)
    async fn execute_code_kernel_ws(
        &mut self,
        code: &str,
        cell_id: Option<&str>,
        on_output: Option<&crate::execution::OutputCallback>,
    ) -> Result<ExecutionResult> {
        let ws = self.ws.as_mut().context("WebSocket not connected")?;
        let deadline = tokio::time::Instant::now() + self.config.timeout;

        let msg_id = ws
            .send_execute_request(code, !self.config.allow_errors, cell_id)
            .await?;

        let mut collector = KernelOutputCollector::new();

        loop {
            let msg = match tokio::time::timeout_at(deadline, ws.recv_message()).await {
                Ok(Ok(Some(msg))) => msg,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => return Err(e).context("WebSocket error"),
                Err(_) => anyhow::bail!("Cell execution timed out after {:?}", self.config.timeout),
            };

            let is_ours = msg
                .parent_header
                .as_ref()
                .map(|h| h.msg_id == msg_id)
                .unwrap_or(false);

            if !is_ours {
                continue;
            }

            if collector.handle(msg.content, on_output) {
                break;
            }
        }

        Ok(collector.into_result())
    }
}

/// Accumulates nbformat outputs from a kernel iopub message stream, applying
/// the same persistence semantics as nbclient: consecutive same-name stream
/// outputs are coalesced into one entry, clear_output is applied immediately,
/// and clear_output(wait=True) is deferred until the next output arrives so
/// that outputs are kept when nothing follows.
struct KernelOutputCollector {
    outputs: Vec<nbformat::v4::Output>,
    execution_count: Option<i64>,
    error_info: Option<ExecutionError>,
    clear_pending: bool,
}

impl KernelOutputCollector {
    fn new() -> Self {
        Self {
            outputs: Vec::new(),
            execution_count: None,
            error_info: None,
            clear_pending: false,
        }
    }

    /// Process one kernel message belonging to the tracked execution.
    /// Returns true when the kernel reports idle, completing the collection.
    fn handle(
        &mut self,
        content: JupyterMessageContent,
        on_output: Option<&crate::execution::OutputCallback>,
    ) -> bool {
        // Apply a deferred clear_output(wait=True) when the next output arrives
        if self.clear_pending
            && matches!(
                content,
                JupyterMessageContent::StreamContent(_)
                    | JupyterMessageContent::ExecuteResult(_)
                    | JupyterMessageContent::DisplayData(_)
                    | JupyterMessageContent::ErrorOutput(_)
            )
        {
            self.outputs.clear();
            self.clear_pending = false;
        }

        match content {
            JupyterMessageContent::Status(status)
                if matches!(
                    status.execution_state,
                    jupyter_protocol::ExecutionState::Idle
                ) =>
            {
                return true;
            }
            JupyterMessageContent::StreamContent(stream) => {
                let name = match stream.name {
                    jupyter_protocol::Stdio::Stdout => "stdout".to_string(),
                    jupyter_protocol::Stdio::Stderr => "stderr".to_string(),
                };
                if let Some(cb) = &on_output {
                    let chunk = nbformat::v4::Output::Stream {
                        name: name.clone(),
                        text: nbformat::v4::MultilineString(stream.text.clone()),
                    };
                    cb(&chunk);
                }
                // Merge consecutive same-name stream outputs into one entry
                let coalesced = if let Some(nbformat::v4::Output::Stream {
                    name: ref last_name,
                    text: ref mut last_text,
                }) = self.outputs.last_mut()
                {
                    if *last_name == name {
                        last_text.0.push_str(&stream.text);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !coalesced {
                    self.outputs.push(nbformat::v4::Output::Stream {
                        name,
                        text: nbformat::v4::MultilineString(stream.text),
                    });
                }
            }
            JupyterMessageContent::ExecuteResult(result) => {
                self.execution_count = Some(result.execution_count.value() as i64);
                let json = serde_json::json!({
                    "output_type": "execute_result",
                    "execution_count": result.execution_count.value(),
                    "data": result.data,
                    "metadata": result.metadata
                });
                if let Ok(output) = serde_json::from_value::<nbformat::v4::Output>(json) {
                    if let Some(cb) = &on_output {
                        cb(&output);
                    }
                    self.outputs.push(output);
                }
            }
            JupyterMessageContent::DisplayData(display) => {
                let json = serde_json::json!({
                    "output_type": "display_data",
                    "data": display.data,
                    "metadata": display.metadata
                });
                if let Ok(output) = serde_json::from_value::<nbformat::v4::Output>(json) {
                    if let Some(cb) = &on_output {
                        cb(&output);
                    }
                    self.outputs.push(output);
                }
            }
            JupyterMessageContent::ErrorOutput(error) => {
                self.error_info = Some(ExecutionError {
                    ename: error.ename.clone(),
                    evalue: error.evalue.clone(),
                    traceback: error.traceback.clone(),
                });
                let output = nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
                    ename: error.ename,
                    evalue: error.evalue,
                    traceback: error.traceback,
                });
                if let Some(cb) = &on_output {
                    cb(&output);
                }
                self.outputs.push(output);
            }
            JupyterMessageContent::ClearOutput(clear) => {
                if clear.wait {
                    self.clear_pending = true;
                } else {
                    self.outputs.clear();
                }
            }
            JupyterMessageContent::ExecuteInput(input) => {
                self.execution_count = Some(input.execution_count.0 as i64);
            }
            _ => {}
        }

        false
    }

    fn into_result(self) -> ExecutionResult {
        if let Some(error) = self.error_info {
            ExecutionResult::error(self.outputs, self.execution_count, error)
        } else {
            ExecutionResult::success(self.outputs, self.execution_count)
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

        // Connect Y.js client for observing outputs during execution (skip for vanilla servers)
        let skip_ydoc = self.config.ydoc_available == Some(false);
        if !skip_ydoc {
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
                    Err(e) => {
                        if self.config.ydoc_available.is_none() {
                            eprintln!("Y.js not available, using direct kernel output: {}", e);
                        } else {
                            return Err(e)
                                .context("Failed to connect Y.js client for output observation");
                        }
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
            self.execute_code_ydoc(code, cell_id, cell_index, on_output)
                .await
        } else {
            self.execute_code_kernel_ws(code, cell_id, on_output).await
        }
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

#[cfg(test)]
mod kernel_output_collector_tests {
    use super::*;
    use jupyter_protocol::{
        ClearOutput, ExecutionCount, ExecutionState, Status, Stdio, StreamContent,
    };

    fn stream(name: Stdio, text: &str) -> JupyterMessageContent {
        JupyterMessageContent::StreamContent(StreamContent {
            name,
            text: text.to_string(),
        })
    }

    fn clear(wait: bool) -> JupyterMessageContent {
        JupyterMessageContent::ClearOutput(ClearOutput { wait })
    }

    fn status(execution_state: ExecutionState) -> JupyterMessageContent {
        JupyterMessageContent::Status(Status { execution_state })
    }

    fn execute_result(ec: usize, plain_text: &str) -> JupyterMessageContent {
        JupyterMessageContent::ExecuteResult(jupyter_protocol::ExecuteResult {
            execution_count: ExecutionCount::new(ec),
            data: serde_json::from_value(serde_json::json!({ "text/plain": plain_text })).unwrap(),
            metadata: serde_json::Map::new(),
            transient: None,
        })
    }

    fn display_data(plain_text: &str) -> JupyterMessageContent {
        JupyterMessageContent::DisplayData(jupyter_protocol::DisplayData {
            data: serde_json::from_value(serde_json::json!({ "text/plain": plain_text })).unwrap(),
            metadata: serde_json::Map::new(),
            transient: None,
        })
    }

    fn error(ename: &str, evalue: &str) -> JupyterMessageContent {
        JupyterMessageContent::ErrorOutput(jupyter_protocol::ErrorOutput {
            ename: ename.to_string(),
            evalue: evalue.to_string(),
            traceback: vec![format!("{}: {}", ename, evalue)],
        })
    }

    /// Feed a message sequence and return the final result, asserting that
    /// only a trailing Idle completes the collection.
    fn collect(messages: Vec<JupyterMessageContent>) -> ExecutionResult {
        assert!(!messages.is_empty(), "test sequence must not be empty");
        let mut collector = KernelOutputCollector::new();
        let last = messages.len() - 1;
        for (i, msg) in messages.into_iter().enumerate() {
            let is_idle = matches!(
                &msg,
                JupyterMessageContent::Status(s) if s.execution_state == ExecutionState::Idle
            );
            let done = collector.handle(msg, None);
            assert_eq!(done, is_idle, "only Idle should complete (message {})", i);
            if done {
                assert_eq!(
                    i, last,
                    "Idle must be the last message in the test sequence"
                );
            }
        }
        collector.into_result()
    }

    fn stream_texts(result: &ExecutionResult) -> Vec<(String, String)> {
        result
            .outputs
            .iter()
            .map(|o| match o {
                nbformat::v4::Output::Stream { name, text } => (name.clone(), text.0.clone()),
                other => panic!("expected stream output, got {:?}", other),
            })
            .collect()
    }

    #[test]
    fn coalesces_consecutive_same_name_streams() {
        let result = collect(vec![
            stream(Stdio::Stdout, "a\n"),
            stream(Stdio::Stdout, "b\n"),
            stream(Stdio::Stdout, "c\n"),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![("stdout".to_string(), "a\nb\nc\n".to_string())]
        );
    }

    #[test]
    fn streams_with_different_names_stay_separate() {
        let result = collect(vec![
            stream(Stdio::Stdout, "out1"),
            stream(Stdio::Stderr, "err"),
            stream(Stdio::Stdout, "out2"),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![
                ("stdout".to_string(), "out1".to_string()),
                ("stderr".to_string(), "err".to_string()),
                ("stdout".to_string(), "out2".to_string()),
            ]
        );
    }

    #[test]
    fn immediate_clear_drops_prior_outputs() {
        let result = collect(vec![
            stream(Stdio::Stdout, "before"),
            clear(false),
            stream(Stdio::Stdout, "after"),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![("stdout".to_string(), "after".to_string())]
        );
    }

    #[test]
    fn deferred_clear_applies_at_next_output() {
        let result = collect(vec![
            stream(Stdio::Stdout, "frame 0"),
            clear(true),
            stream(Stdio::Stdout, "frame 1"),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![("stdout".to_string(), "frame 1".to_string())]
        );
    }

    #[test]
    fn trailing_deferred_clear_keeps_outputs() {
        let result = collect(vec![
            stream(Stdio::Stdout, "kept"),
            clear(true),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![("stdout".to_string(), "kept".to_string())]
        );
    }

    #[test]
    fn error_produces_error_result_with_output() {
        let result = collect(vec![
            error("ZeroDivisionError", "division by zero"),
            status(ExecutionState::Idle),
        ]);
        assert!(!result.success);
        let err = result.error.expect("error info should be set");
        assert_eq!(err.ename, "ZeroDivisionError");
        assert!(matches!(
            result.outputs.as_slice(),
            [nbformat::v4::Output::Error(e)] if e.ename == "ZeroDivisionError"
        ));
    }

    #[test]
    fn execute_result_sets_execution_count_and_output() {
        let result = collect(vec![execute_result(3, "42"), status(ExecutionState::Idle)]);
        assert!(result.success);
        assert_eq!(result.execution_count, Some(3));
        assert!(matches!(
            result.outputs.as_slice(),
            [nbformat::v4::Output::ExecuteResult(er)] if er.execution_count.0 == 3
        ));
    }

    #[test]
    fn busy_status_and_unrelated_messages_are_ignored() {
        let result = collect(vec![
            status(ExecutionState::Busy),
            JupyterMessageContent::ExecuteInput(jupyter_protocol::ExecuteInput {
                code: "1 + 1".to_string(),
                execution_count: ExecutionCount::new(7),
            }),
            status(ExecutionState::Idle),
        ]);
        assert!(result.success);
        assert_eq!(result.execution_count, Some(7));
        assert!(result.outputs.is_empty());
    }

    #[test]
    fn clear_starts_a_new_coalescing_run() {
        let result = collect(vec![
            stream(Stdio::Stdout, "old"),
            clear(false),
            stream(Stdio::Stdout, "new1 "),
            stream(Stdio::Stdout, "new2"),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(
            stream_texts(&result),
            vec![("stdout".to_string(), "new1 new2".to_string())]
        );
    }

    #[test]
    fn deferred_clear_flushed_by_each_output_kind() {
        type IsExpected = fn(&nbformat::v4::Output) -> bool;
        let cases: [(JupyterMessageContent, &str, IsExpected); 3] = [
            (execute_result(1, "42"), "execute_result", |o| {
                matches!(o, nbformat::v4::Output::ExecuteResult(_))
            }),
            (display_data("img"), "display_data", |o| {
                matches!(o, nbformat::v4::Output::DisplayData(_))
            }),
            (error("E", "v"), "error", |o| {
                matches!(o, nbformat::v4::Output::Error(_))
            }),
        ];
        for (flusher, label, is_expected) in cases {
            let result = collect(vec![
                stream(Stdio::Stdout, "stale"),
                clear(true),
                flusher,
                status(ExecutionState::Idle),
            ]);
            assert_eq!(
                result.outputs.len(),
                1,
                "{}: stale output should be dropped",
                label
            );
            assert!(
                is_expected(&result.outputs[0]),
                "{}: remaining output should be the flushing output, got {:?}",
                label,
                result.outputs[0]
            );
        }
    }

    #[test]
    fn display_data_becomes_nbformat_output() {
        let result = collect(vec![display_data("chart"), status(ExecutionState::Idle)]);
        assert!(matches!(
            result.outputs.as_slice(),
            [nbformat::v4::Output::DisplayData(_)]
        ));
    }

    #[test]
    fn on_output_callback_receives_each_chunk_even_when_coalesced() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = Arc::clone(&seen);
        let cb: crate::execution::OutputCallback = Box::new(move |o| {
            let label = match o {
                nbformat::v4::Output::Stream { text, .. } => format!("stream:{}", text.0),
                nbformat::v4::Output::ExecuteResult(_) => "result".to_string(),
                other => format!("{:?}", other),
            };
            seen_clone.lock().unwrap().push(label);
        });

        let mut collector = KernelOutputCollector::new();
        for msg in [
            stream(Stdio::Stdout, "a"),
            stream(Stdio::Stdout, "b"),
            execute_result(1, "42"),
        ] {
            assert!(!collector.handle(msg, Some(&cb)));
        }
        let result = collector.into_result();

        // Callback sees every chunk as it arrives...
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["stream:a", "stream:b", "result"]
        );
        // ...while persisted outputs coalesce the stream chunks.
        assert_eq!(result.outputs.len(), 2);
    }
}
