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

fn result_from_outputs(
    outputs: Vec<nbformat::v4::Output>,
    execution_count: Option<i64>,
) -> ExecutionResult {
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

    if let Some(error_info) = error_info {
        ExecutionResult::error(outputs, execution_count, error_info)
    } else {
        ExecutionResult::success(outputs, execution_count)
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
    let initial_execution_count = ydoc
        .read_cell_outputs(cell_idx)
        .ok()
        .and_then(|d| d.execution_count);
    let http = reqwest::Client::new();
    // jupyter-collaboration doesn't write outputs to Y.js itself, so once
    // the kernel goes idle we write our own kernel-WS-collected outputs
    // and let the read loop below pick them back up from the doc.
    //
    // jupyter-server-documents writes outputs to Y.js, while its kernel
    // WebSocket can intermittently miss idle/status or close between cells.
    // Use whichever completion/output signals arrive, and require a short
    // quiet period before treating JSD's Y.js output stream as complete.
    let client_writes_outputs = !ydoc.server_writes_outputs();

    let msg_id = ws
        .send_execute_request(code, !executor.config.allow_errors, cell_id)
        .await?;

    let mut outputs: Vec<nbformat::v4::Output> = Vec::new();
    let mut kernel_outputs: Vec<nbformat::v4::Output> = Vec::new();
    let mut fetched_urls: HashSet<String> = HashSet::new();
    let mut seen_indices: HashSet<usize> = HashSet::new();
    let mut idle_received = false;
    let mut shell_reply_received = false;
    let mut expected_ec: Option<i64> = None;
    let deadline = tokio::time::Instant::now() + executor.config.timeout;
    let mut post_completion_output_deadline: Option<tokio::time::Instant> = None;
    let mut post_idle_ydoc_deadline: Option<tokio::time::Instant> = None;
    let mut observed_output_count = 0usize;
    let debug_exec = std::env::var_os("NB_DEBUG_EXEC").is_some();
    let debug_started = tokio::time::Instant::now();
    let mut next_debug_tick = debug_started + std::time::Duration::from_secs(5);
    let mut last_debug_ec: Option<i64> = None;
    let mut last_debug_externalized_count = 0usize;
    let mut last_debug_inline_count = 0usize;

    if debug_exec {
        eprintln!(
            "[nb-debug] start ydoc execute cell_idx={cell_idx} cell_id={:?} msg_id={} client_writes_outputs={} initial_ec={:?} timeout={:?}",
            cell_id,
            msg_id,
            client_writes_outputs,
            initial_execution_count,
            executor.config.timeout
        );
    }

    loop {
        let cell_data = ydoc.read_cell_outputs(cell_idx).ok();
        let ec = cell_data.as_ref().and_then(|d| d.execution_count);
        let externalized_count = cell_data
            .as_ref()
            .map(|d| d.externalized_urls.len())
            .unwrap_or(0);
        let inline_count = cell_data
            .as_ref()
            .map(|d| d.inline_outputs.len())
            .unwrap_or(0);
        let ec_ready = expected_ec.is_some() && ec == expected_ec;
        let ydoc_execution_advanced = !client_writes_outputs
            && ec
                .zip(initial_execution_count)
                .map(|(current, initial)| current > initial)
                .unwrap_or_else(|| ec.is_some() && initial_execution_count.is_none());
        let execution_complete = idle_received || ydoc_execution_advanced || shell_reply_received;

        if debug_exec && tokio::time::Instant::now() >= next_debug_tick {
            eprintln!(
                "[nb-debug] tick elapsed={:?} cell_idx={cell_idx} ec={:?} expected_ec={:?} ec_ready={} ydoc_advanced={} idle={} shell_reply={} externalized={} inline={} outputs={} kernel_outputs={}",
                debug_started.elapsed(),
                ec,
                expected_ec,
                ec_ready,
                ydoc_execution_advanced,
                idle_received,
                shell_reply_received,
                externalized_count,
                inline_count,
                outputs.len(),
                kernel_outputs.len()
            );
            next_debug_tick += std::time::Duration::from_secs(5);
        }

        if debug_exec
            && (ec != last_debug_ec
                || externalized_count != last_debug_externalized_count
                || inline_count != last_debug_inline_count)
        {
            eprintln!(
                "[nb-debug] ydoc state elapsed={:?} cell_idx={cell_idx} ec={:?} externalized={} inline={} ydoc_advanced={}",
                debug_started.elapsed(),
                ec,
                externalized_count,
                inline_count,
                ydoc_execution_advanced
            );
            last_debug_ec = ec;
            last_debug_externalized_count = externalized_count;
            last_debug_inline_count = inline_count;
        }

        if ec_ready || ydoc_execution_advanced || (!client_writes_outputs && shell_reply_received) {
            if let Some(ref cell_data) = cell_data {
                for (idx, url_path) in &cell_data.externalized_urls {
                    if fetched_urls.insert(url_path.clone()) {
                        seen_indices.insert(*idx);
                        if debug_exec {
                            eprintln!(
                                "[nb-debug] output-url elapsed={:?} cell_idx={cell_idx} idx={} url={}",
                                debug_started.elapsed(),
                                idx,
                                url_path
                            );
                        }
                        if let Some(output) =
                            fetch_output(&http, &executor.server_url, &executor.token, url_path)
                                .await
                        {
                            if client_writes_outputs || kernel_outputs.is_empty() {
                                if let Some(cb) = &on_output {
                                    cb(&output);
                                }
                            }
                            if client_writes_outputs || kernel_outputs.is_empty() {
                                outputs.push(output);
                            }
                        }
                    }
                }
                for (idx, output) in &cell_data.inline_outputs {
                    if seen_indices.insert(*idx) {
                        if client_writes_outputs || kernel_outputs.is_empty() {
                            if let Some(cb) = &on_output {
                                cb(&output);
                            }
                        }
                        if client_writes_outputs || kernel_outputs.is_empty() {
                            outputs.push(output.clone());
                        }
                    }
                }
            }

            if execution_complete {
                let have_outputs = !outputs.is_empty() || !kernel_outputs.is_empty();
                let output_count = outputs.len() + kernel_outputs.len();
                if output_count > observed_output_count {
                    observed_output_count = output_count;
                    post_completion_output_deadline = None;
                }

                if !client_writes_outputs && !idle_received {
                    let quiet_period = if have_outputs {
                        std::time::Duration::from_millis(2500)
                    } else if shell_reply_received {
                        std::time::Duration::from_secs(1)
                    } else {
                        std::time::Duration::from_secs(3)
                    };
                    let output_deadline = *post_completion_output_deadline
                        .get_or_insert_with(|| tokio::time::Instant::now() + quiet_period);
                    if tokio::time::Instant::now() < output_deadline {
                        match tokio::time::timeout_at(output_deadline, ydoc.recv_update()).await {
                            Ok(Ok(_)) => continue,
                            Ok(Err(e)) => return Err(e).context("Y.js update error"),
                            Err(_) => {}
                        }
                    }
                }

                return if client_writes_outputs || kernel_outputs.is_empty() {
                    Ok(result_from_outputs(outputs, ec))
                } else {
                    Ok(result_from_outputs(kernel_outputs, expected_ec.or(ec)))
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
            if !client_writes_outputs {
                let ydoc_deadline = *post_idle_ydoc_deadline.get_or_insert_with(|| {
                    tokio::time::Instant::now() + std::time::Duration::from_secs(1)
                });
                if tokio::time::Instant::now() >= ydoc_deadline {
                    if debug_exec {
                        eprintln!(
                            "[nb-debug] post-idle ydoc wait expired elapsed={:?} cell_idx={cell_idx} ec={:?} outputs={} externalized={} inline={}",
                            debug_started.elapsed(),
                            ec,
                            outputs.len(),
                            externalized_count,
                            inline_count
                        );
                    }
                    break;
                }
                match tokio::time::timeout_at(ydoc_deadline, ydoc.recv_update()).await {
                    Ok(Ok(_)) => continue,
                    Ok(Err(e)) => return Err(e).context("Y.js update error"),
                    Err(_) => break,
                }
            }

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
                                        if debug_exec {
                                            eprintln!(
                                                "[nb-debug] execute_input elapsed={:?} cell_idx={cell_idx} ec={:?}",
                                                debug_started.elapsed(),
                                                expected_ec
                                            );
                                        }
                                    }
                                    JupyterMessageContent::ExecuteReply(reply) => {
                                        if expected_ec.is_none() {
                                            expected_ec =
                                                Some(reply.execution_count.value() as i64);
                                        }
                                        shell_reply_received = true;
                                        if debug_exec {
                                            eprintln!(
                                                "[nb-debug] execute_reply elapsed={:?} cell_idx={cell_idx} ec={:?}",
                                                debug_started.elapsed(),
                                                expected_ec
                                            );
                                        }
                                    }
                                    JupyterMessageContent::Status(status) => {
                                        if debug_exec {
                                            eprintln!(
                                                "[nb-debug] status elapsed={:?} cell_idx={cell_idx} state={:?}",
                                                debug_started.elapsed(),
                                                status.execution_state
                                            );
                                        }
                                        if matches!(status.execution_state,
                                            jupyter_protocol::ExecutionState::Idle) {
                                            idle_received = true;
                                            if debug_exec {
                                                eprintln!(
                                                    "[nb-debug] idle elapsed={:?} cell_idx={cell_idx}",
                                                    debug_started.elapsed()
                                                );
                                            }
                                        }
                                    }
                                    _ => {
                                        if let Some(output) = kernel_msg_to_output(&msg.content) {
                                            if !client_writes_outputs {
                                                if let Some(cb) = &on_output {
                                                    cb(&output);
                                                }
                                            }
                                            kernel_outputs.push(output);
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
        return Ok(result_from_outputs(kernel_outputs, ec));
    }

    let ec = ydoc
        .read_cell_outputs(cell_idx)
        .ok()
        .and_then(|c| c.execution_count);
    if !kernel_outputs.is_empty() {
        Ok(result_from_outputs(kernel_outputs, expected_ec.or(ec)))
    } else {
        Ok(result_from_outputs(outputs, ec))
    }
}
