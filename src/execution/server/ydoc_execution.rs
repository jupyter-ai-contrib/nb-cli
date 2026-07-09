//! Y.js-observed execution path: used when the Jupyter Server has a
//! collaboration backend (jupyter-server-documents or jupyter-collaboration)
//! attached, so outputs can be observed via the Y.js document instead of
//! (or in addition to) the kernel WebSocket directly.

use super::RemoteExecutor;
use crate::execution::types::{ExecutionError, ExecutionResult};
use anyhow::{Context, Result};
use jupyter_protocol::messaging::JupyterMessageContent;
use std::collections::HashSet;

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

/// Convert a kernel iopub message to an nbformat output, for the
/// jupyter-collaboration path where outputs must be collected from the
/// kernel WS directly (the server won't write them to Y.js itself).
fn kernel_msg_to_output(content: &JupyterMessageContent) -> Option<nbformat::v4::Output> {
    match content {
        JupyterMessageContent::StreamContent(stream) => Some(
            crate::execution::output_collector::stream_to_output(stream.clone()),
        ),
        JupyterMessageContent::ExecuteResult(result) => {
            crate::execution::output_collector::execute_result_to_output(result)
        }
        JupyterMessageContent::DisplayData(display) => {
            crate::execution::output_collector::display_data_to_output(display)
        }
        JupyterMessageContent::ErrorOutput(error) => {
            Some(crate::execution::output_collector::error_to_output(error.clone()).0)
        }
        _ => None,
    }
}

/// Y.js-based output collection (collaboration server)
pub(super) async fn execute_code_ydoc(
    executor: &mut RemoteExecutor,
    code: &str,
    cell_id: Option<&str>,
    cell_index: Option<usize>,
    on_output: Option<&crate::execution::OutputCallback>,
) -> Result<ExecutionResult> {
    let ws = executor.ws.as_mut().context("WebSocket not connected")?;
    let cell_idx = cell_index.context("cell_index required for remote execution")?;
    let ydoc = executor
        .ydoc
        .as_mut()
        .context("Y.js client not connected")?;
    let http = reqwest::Client::new();
    // jupyter-collaboration doesn't write outputs to Y.js itself, so once
    // the kernel goes idle we write our own kernel-WS-collected outputs
    // and let the read loop below pick them back up from the doc.
    let client_writes_outputs = !ydoc.server_writes_outputs();

    let msg_id = ws
        .send_execute_request(code, !executor.config.allow_errors, cell_id)
        .await?;

    let mut outputs: Vec<nbformat::v4::Output> = Vec::new();
    let mut kernel_outputs: Vec<nbformat::v4::Output> = Vec::new();
    let mut fetched_urls: HashSet<String> = HashSet::new();
    let mut seen_indices: HashSet<usize> = HashSet::new();
    let mut idle_received = false;
    let mut expected_ec: Option<i64> = None;
    let deadline = tokio::time::Instant::now() + executor.config.timeout;

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
                            fetch_output(&http, &executor.server_url, &executor.token, url_path)
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

        // jupyter-collaboration never writes outputs or execution_count to
        // Y.js itself, so once the kernel is idle we must do it. This also
        // covers cells with no output: without this branch, a no-output cell
        // would fall through to the ydoc.recv_update() wait below and block
        // for the entire execution timeout (120s+), during which the server's
        // 30s WebSocket ping kills the kernel connection for subsequent cells.
        if client_writes_outputs && idle_received && !ec_ready && expected_ec.is_some() {
            ydoc.update_cell_outputs(cell_idx, kernel_outputs.clone())?;
            ydoc.update_cell_execution_count(cell_idx, expected_ec)?;
            ydoc.sync().await?;
            continue;
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
                        executor.config.timeout
                    );
                }
                kernel_msg = ws.recv_message() => {
                    match kernel_msg? {
                        Some(msg) => {
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
                                    _ => {
                                        if client_writes_outputs {
                                            if let Some(output) =
                                                kernel_msg_to_output(&msg.content)
                                            {
                                                kernel_outputs.push(output);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // A closed socket yields None on every recv; without
                        // this arm the select busy-spins until the deadline
                        // and misreports the drop as a timeout.
                        None => anyhow::bail!(
                            "Kernel WebSocket closed before execution completed"
                        ),
                    }
                }
                ydoc_result = ydoc.recv_update() => {
                    ydoc_result.context("Y.js update error")?;
                }
            }
        }
    }

    // Fallback: the Y.js write/sync above may not have round-tripped
    // through the read loop before the post-idle timeout broke us out;
    // return what the kernel WS collected rather than losing output.
    if client_writes_outputs && !kernel_outputs.is_empty() {
        let ec = expected_ec;
        let has_error = kernel_outputs
            .iter()
            .any(|o| matches!(o, nbformat::v4::Output::Error(_)));
        let error_info = kernel_outputs.iter().find_map(|o| {
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
            Ok(ExecutionResult::error(
                kernel_outputs,
                ec,
                error_info.unwrap(),
            ))
        } else {
            Ok(ExecutionResult::success(kernel_outputs, ec))
        };
    }

    let ec = ydoc
        .read_cell_outputs(cell_idx)
        .ok()
        .and_then(|c| c.execution_count);
    Ok(ExecutionResult::success(outputs, ec))
}
