//! Kernel-WS-only output collection (vanilla jupyter_server, no Y.js).

use super::RemoteExecutor;
use crate::execution::output_collector::KernelOutputCollector;
use crate::execution::types::ExecutionResult;
use anyhow::{Context, Result};

pub(super) async fn execute_code_kernel_ws(
    executor: &mut RemoteExecutor,
    code: &str,
    cell_id: Option<&str>,
    on_output: Option<&crate::execution::OutputCallback>,
) -> Result<ExecutionResult> {
    let ws = executor.ws.as_mut().context("WebSocket not connected")?;
    let deadline = tokio::time::Instant::now() + executor.config.timeout;

    let msg_id = ws
        .send_execute_request(code, !executor.config.allow_errors, cell_id)
        .await?;

    let mut collector = KernelOutputCollector::new();

    loop {
        let msg = match tokio::time::timeout_at(deadline, ws.recv_message()).await {
            Ok(Ok(Some(msg))) => msg,
            // A closed socket before idle means we cannot know whether the
            // cell completed; reporting success with empty outputs would
            // make a dropped connection look like a no-output cell.
            Ok(Ok(None)) => {
                anyhow::bail!("Kernel WebSocket closed before execution completed")
            }
            Ok(Err(e)) => return Err(e).context("WebSocket error"),
            Err(_) => anyhow::bail!(
                "Cell execution timed out after {:?}",
                executor.config.timeout
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::server::websocket::KernelWebSocket;
    use crate::execution::types::ExecutionConfig;

    /// A server that completes the v1-subprotocol handshake and then goes
    /// silent must trip the per-cell deadline, not hang.
    #[tokio::test]
    async fn execute_times_out_when_kernel_never_responds() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let _ws = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |_req: &Request, mut resp: Response| {
                        resp.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            "v1.kernel.websocket.jupyter.org".parse().unwrap(),
                        );
                        Ok(resp)
                    },
                )
                .await;
                // Hold the connection open without ever sending a message
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });

        let ws = KernelWebSocket::connect(&format!("ws://{}/api/kernels/k/channels", addr))
            .await
            .unwrap();

        let mut executor = RemoteExecutor {
            config: ExecutionConfig {
                timeout: std::time::Duration::from_millis(300),
                ..Default::default()
            },
            server_url: format!("http://{}", addr),
            token: "t".to_string(),
            client: None,
            session: None,
            ws: Some(ws),
            ydoc: None,
            created_session: false,
        };

        let started = tokio::time::Instant::now();
        let err = match execute_code_kernel_ws(&mut executor, "1 + 1", None, None).await {
            Ok(_) => panic!("execution must time out when the kernel never responds"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {}",
            err
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "timeout must fire promptly, took {:?}",
            started.elapsed()
        );
    }

    /// A connection dropped before idle must surface as an error, not as a
    /// successful execution with empty outputs.
    #[tokio::test]
    async fn execute_errors_when_connection_drops_before_idle() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |_req: &Request, mut resp: Response| {
                        resp.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            "v1.kernel.websocket.jupyter.org".parse().unwrap(),
                        );
                        Ok(resp)
                    },
                )
                .await;
                drop(ws);
            }
        });

        let ws = KernelWebSocket::connect(&format!("ws://{}/api/kernels/k/channels", addr))
            .await
            .unwrap();

        let mut executor = RemoteExecutor {
            config: ExecutionConfig {
                timeout: std::time::Duration::from_secs(10),
                ..Default::default()
            },
            server_url: format!("http://{}", addr),
            token: "t".to_string(),
            client: None,
            session: None,
            ws: Some(ws),
            ydoc: None,
            created_session: false,
        };

        let err = match execute_code_kernel_ws(&mut executor, "1 + 1", None, None).await {
            Ok(r) => panic!(
                "dropped connection must not report success (got success={}, outputs={})",
                r.success,
                r.outputs.len()
            ),
            Err(e) => e,
        };
        // Three legitimate error paths depending on when the peer reset is
        // observed: detected on recv (closed-before-completed), surfaced as a
        // read error, or hit during the send itself.
        assert!(
            err.to_string()
                .contains("closed before execution completed")
                || err.to_string().contains("WebSocket error")
                || err.to_string().contains("Failed to send execute request"),
            "unexpected error: {}",
            err
        );
    }
}
