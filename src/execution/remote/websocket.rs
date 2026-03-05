//! WebSocket client for kernel channels

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use jupyter_protocol::messaging::{JupyterMessage, JupyterMessageContent};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

/// WebSocket connection to a Jupyter kernel
pub struct KernelWebSocket {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        Message,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
}

impl KernelWebSocket {
    /// Connect to a kernel via WebSocket
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .context("Failed to connect to kernel WebSocket")?;

        let (write, read) = ws_stream.split();

        Ok(Self { write, read })
    }

    /// Send an execute request
    pub async fn send_execute_request(&mut self, code: &str, stop_on_error: bool) -> Result<String> {
        let mut execute_request = JupyterMessage::new(
            JupyterMessageContent::ExecuteRequest(jupyter_protocol::ExecuteRequest {
                code: code.to_string(),
                silent: false,
                store_history: true,
                user_expressions: None,
                allow_stdin: false,
                stop_on_error,
            }),
            None,
        );

        // Save message ID for correlation
        let msg_id = execute_request.header.msg_id.clone();

        let json = serde_json::to_string(&execute_request)
            .context("Failed to serialize execute request")?;

        self.write
            .send(Message::Text(json))
            .await
            .context("Failed to send execute request")?;

        Ok(msg_id)
    }

    /// Receive the next message
    pub async fn recv_message(&mut self) -> Result<Option<JupyterMessage>> {
        loop {
            match self.read.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: JupyterMessage = serde_json::from_str(&text)
                        .with_context(|| format!("Failed to parse text message"))?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Binary(data))) => {
                    // Jupyter Server WebSocket sends messages as multi-part format
                    // Try parsing as simple JSON first (some servers send this way)
                    let text = String::from_utf8_lossy(&data);
                    if let Ok(msg) = serde_json::from_str::<JupyterMessage>(&text) {
                        return Ok(Some(msg));
                    }

                    // Otherwise, parse the wire protocol format
                    // Split by newlines to get individual frames
                    let frames: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();

                    if frames.len() < 6 {
                        continue;
                    }

                    // Find the delimiter frame <IDS|MSG>
                    let delimiter_idx = frames.iter().position(|f| f.starts_with(b"<IDS|MSG>"));

                    if let Some(del_idx) = delimiter_idx {
                        // After delimiter: hmac (1), header (2), parent_header (3), metadata (4), content (5)
                        if frames.len() > del_idx + 5 {
                            let header_data = frames[del_idx + 2];
                            let parent_header_data = frames[del_idx + 3];
                            let metadata_data = frames[del_idx + 4];
                            let content_data = frames[del_idx + 5];

                            // Try to parse and construct message
                            if let (Ok(header_val), Ok(parent_header_val), Ok(metadata_val), Ok(content_val)) = (
                                serde_json::from_slice::<serde_json::Value>(header_data),
                                serde_json::from_slice::<serde_json::Value>(parent_header_data),
                                serde_json::from_slice::<serde_json::Value>(metadata_data),
                                serde_json::from_slice::<serde_json::Value>(content_data),
                            ) {
                                let full_msg_json = serde_json::json!({
                                    "header": header_val,
                                    "parent_header": parent_header_val,
                                    "metadata": metadata_val,
                                    "content": content_val,
                                });

                                if let Ok(msg) = serde_json::from_value::<JupyterMessage>(full_msg_json) {
                                    return Ok(Some(msg));
                                }
                            }
                        }
                    }

                    // Skip unparseable messages
                    continue;
                }
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(_)) => {
                    // Ignore other message types (ping, pong, etc.) and continue loop
                    continue;
                }
                Some(Err(e)) => return Err(e).context("WebSocket error"),
                None => return Ok(None),
            }
        }
    }

    /// Close the WebSocket connection
    pub async fn close(mut self) -> Result<()> {
        self.write
            .close()
            .await
            .context("Failed to close WebSocket")?;
        Ok(())
    }
}
