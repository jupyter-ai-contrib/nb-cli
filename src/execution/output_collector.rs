//! Shared Jupyter-message-to-`nbformat::v4::Output` conversion, used by all
//! three execution backends (local ZMQ, kernel gateway WS, and the
//! kernel-WS-only leg of the Jupyter Server backend).

use crate::execution::types::ExecutionError;
use jupyter_protocol::messaging::JupyterMessageContent;

/// Convert a `StreamContent` message into a stream `Output`.
pub fn stream_to_output(stream: jupyter_protocol::StreamContent) -> nbformat::v4::Output {
    let name = match stream.name {
        jupyter_protocol::Stdio::Stdout => "stdout".to_string(),
        jupyter_protocol::Stdio::Stderr => "stderr".to_string(),
    };
    nbformat::v4::Output::Stream {
        name,
        text: nbformat::v4::MultilineString(stream.text),
    }
}

/// Convert an `ExecuteResult` message into an `execute_result` `Output`.
/// Returns `None` if the message can't be represented as an nbformat output
/// (never expected in practice; mirrors the pre-existing `.ok()` handling).
pub fn execute_result_to_output(
    result: &jupyter_protocol::ExecuteResult,
) -> Option<nbformat::v4::Output> {
    let json = serde_json::json!({
        "output_type": "execute_result",
        "execution_count": result.execution_count.value(),
        "data": result.data,
        "metadata": result.metadata
    });
    serde_json::from_value(json).ok()
}

/// Convert a `DisplayData` message into a `display_data` `Output`.
pub fn display_data_to_output(
    display: &jupyter_protocol::DisplayData,
) -> Option<nbformat::v4::Output> {
    let json = serde_json::json!({
        "output_type": "display_data",
        "data": display.data,
        "metadata": display.metadata
    });
    serde_json::from_value(json).ok()
}

/// Convert an `ErrorOutput` message into an error `Output`, alongside the
/// separately-tracked `ExecutionError` used for `ExecutionResult::error`.
pub fn error_to_output(
    error: jupyter_protocol::ErrorOutput,
) -> (nbformat::v4::Output, ExecutionError) {
    let execution_error = ExecutionError {
        ename: error.ename.clone(),
        evalue: error.evalue.clone(),
        traceback: error.traceback.clone(),
    };
    let output = nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
        ename: error.ename,
        evalue: error.evalue,
        traceback: error.traceback,
    });
    (output, execution_error)
}

/// Accumulates nbformat outputs from a kernel iopub message stream, applying
/// the same persistence semantics as nbclient: consecutive same-name stream
/// outputs are coalesced into one entry, clear_output is applied immediately,
/// and clear_output(wait=True) is deferred until the next output arrives so
/// that outputs are kept when nothing follows.
///
/// Completion requires a `Busy` status before the completing `Idle` (a kernel
/// that reports Idle without ever having gone Busy has not actually run the
/// request yet — this matches the Jupyter messaging spec and the Kernel
/// Gateway backend's pre-existing, stricter behavior).
pub struct KernelOutputCollector {
    outputs: Vec<nbformat::v4::Output>,
    execution_count: Option<i64>,
    error_info: Option<ExecutionError>,
    clear_pending: bool,
    saw_busy: bool,
}

impl Default for KernelOutputCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelOutputCollector {
    pub fn new() -> Self {
        Self {
            outputs: Vec::new(),
            execution_count: None,
            error_info: None,
            clear_pending: false,
            saw_busy: false,
        }
    }

    /// Process one kernel message belonging to the tracked execution.
    /// Returns true when the kernel reports idle (after having been busy),
    /// completing the collection.
    pub fn handle(
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
            JupyterMessageContent::Status(status) => match status.execution_state {
                jupyter_protocol::ExecutionState::Busy => {
                    self.saw_busy = true;
                }
                jupyter_protocol::ExecutionState::Idle if self.saw_busy => {
                    return true;
                }
                _ => {}
            },
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
                    self.outputs.push(stream_to_output(stream));
                }
            }
            JupyterMessageContent::ExecuteResult(result) => {
                self.execution_count = Some(result.execution_count.value() as i64);
                if let Some(output) = execute_result_to_output(&result) {
                    if let Some(cb) = &on_output {
                        cb(&output);
                    }
                    self.outputs.push(output);
                }
            }
            JupyterMessageContent::DisplayData(display) => {
                if let Some(output) = display_data_to_output(&display) {
                    if let Some(cb) = &on_output {
                        cb(&output);
                    }
                    self.outputs.push(output);
                }
            }
            JupyterMessageContent::ErrorOutput(error) => {
                let (output, execution_error) = error_to_output(error);
                self.error_info = Some(execution_error);
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
            JupyterMessageContent::ExecuteReply(reply) if self.execution_count.is_none() => {
                self.execution_count = Some(reply.execution_count.value() as i64);
            }
            _ => {}
        }

        false
    }

    pub fn into_result(self) -> crate::execution::types::ExecutionResult {
        if let Some(error) = self.error_info {
            crate::execution::types::ExecutionResult::error(
                self.outputs,
                self.execution_count,
                error,
            )
        } else {
            crate::execution::types::ExecutionResult::success(self.outputs, self.execution_count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::types::ExecutionResult;
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

    /// Feed a message sequence (which must include a leading Busy so Idle
    /// completes) and return the final result.
    fn collect(messages: Vec<JupyterMessageContent>) -> ExecutionResult {
        assert!(!messages.is_empty(), "test sequence must not be empty");
        let mut collector = KernelOutputCollector::new();
        let last = messages.len() - 1;
        for (i, msg) in messages.into_iter().enumerate() {
            let is_completing_idle = i == last
                && matches!(
                    &msg,
                    JupyterMessageContent::Status(s) if s.execution_state == ExecutionState::Idle
                );
            let done = collector.handle(msg, None);
            assert_eq!(
                done, is_completing_idle,
                "only the trailing Idle should complete (message {})",
                i
            );
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
            status(ExecutionState::Busy),
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
            status(ExecutionState::Busy),
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
            status(ExecutionState::Busy),
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
            status(ExecutionState::Busy),
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
            status(ExecutionState::Busy),
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
            status(ExecutionState::Busy),
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
        let result = collect(vec![
            status(ExecutionState::Busy),
            execute_result(3, "42"),
            status(ExecutionState::Idle),
        ]);
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
    fn idle_without_prior_busy_does_not_complete() {
        // A kernel that reports Idle without ever having gone Busy has not
        // actually run anything yet — this is the flagged behavior change
        // from the old local/executor.rs and kernel_ws policy (any-idle),
        // now unified onto the stricter busy-then-idle policy.
        let mut collector = KernelOutputCollector::new();
        let done = collector.handle(status(ExecutionState::Idle), None);
        assert!(!done, "Idle with no preceding Busy must not complete");
    }

    #[test]
    fn clear_starts_a_new_coalescing_run() {
        let result = collect(vec![
            status(ExecutionState::Busy),
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
                status(ExecutionState::Busy),
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
        let result = collect(vec![
            status(ExecutionState::Busy),
            display_data("chart"),
            status(ExecutionState::Idle),
        ]);
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
            status(ExecutionState::Busy),
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

    #[test]
    fn execute_reply_sets_execution_count_when_no_result_seen() {
        // Mirrors local/executor.rs's and gateway/executor.rs's fallback:
        // cells with no ExecuteResult (e.g. only side effects) still get
        // their execution_count from the shell-channel ExecuteReply.
        let result = collect(vec![
            status(ExecutionState::Busy),
            JupyterMessageContent::ExecuteReply(jupyter_protocol::ExecuteReply {
                execution_count: ExecutionCount::new(5),
                ..Default::default()
            }),
            status(ExecutionState::Idle),
        ]);
        assert_eq!(result.execution_count, Some(5));
    }
}
