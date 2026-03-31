//! WebSocket client for kernel channels

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use jupyter_protocol::messaging::{JupyterMessage, JupyterMessageContent};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// WebSocket connection to a Jupyter kernel
pub struct KernelWebSocket {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
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

    /// Parse Jupyter's binary message format
    fn parse_binary_message(data: &[u8]) -> Option<JupyterMessage> {
        // Read number of buffers (first 8 bytes, little-endian)
        if data.len() < 8 {
            return None;
        }

        let num_buffers = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;

        // We need num_buffers offsets (each 8 bytes) plus the header
        let header_size = 8 + (num_buffers * 8);
        if data.len() < header_size {
            return None;
        }

        // Read offsets
        let mut offsets = Vec::new();
        for i in 0..num_buffers {
            let offset_start = 8 + (i * 8);
            let offset =
                u64::from_le_bytes(data[offset_start..offset_start + 8].try_into().ok()?) as usize;
            offsets.push(offset);
        }

        // Extract buffers using offsets
        let mut buffers = Vec::new();
        for i in 0..num_buffers {
            let start = offsets[i];
            let end = if i + 1 < num_buffers {
                offsets[i + 1]
            } else {
                data.len()
            };

            if start <= end && end <= data.len() {
                let buffer = &data[start..end];
                buffers.push(buffer);
            }
        }

        // Jupyter protocol over WebSocket typically has:
        // buffer 0: channel (e.g., "iopub")
        // buffer 1: header (JSON)
        // buffer 2: parent_header (JSON)
        // buffer 3: metadata (JSON)
        // buffer 4: content (JSON)
        // buffer 5+: extra buffers

        if buffers.len() < 5 {
            return None;
        }

        // Parse the JSON components
        let header: serde_json::Value = serde_json::from_slice(buffers[1]).ok()?;
        let parent_header: serde_json::Value = serde_json::from_slice(buffers[2]).ok()?;
        let metadata: serde_json::Value = serde_json::from_slice(buffers[3]).ok()?;
        let content_json: serde_json::Value = serde_json::from_slice(buffers[4]).ok()?;

        // Construct a full message
        let full_msg = serde_json::json!({
            "header": header,
            "parent_header": parent_header,
            "metadata": metadata,
            "content": content_json,
        });

        serde_json::from_value(full_msg).ok()
    }

    /// Serialize message to Jupyter's WebSocket v1 binary format
    /// Format: [offset_count(u64)] [offset0(u64)] ... [offsetN(u64)] [data...]
    /// Data sections: channel, header, parent_header, metadata, content
    /// Note: Unlike ZMQ, WebSocket format does NOT include HMAC signature or delimiter
    fn serialize_to_binary(msg: &JupyterMessage, channel: &str) -> Result<Vec<u8>> {
        // Serialize each component
        let channel_bytes = channel.as_bytes();
        let header_bytes = serde_json::to_vec(&msg.header)?;
        let parent_header_bytes = serde_json::to_vec(&msg.parent_header)?;
        let metadata_bytes = serde_json::to_vec(&msg.metadata)?;
        let content_bytes = serde_json::to_vec(&msg.content)?;

        // We need 6 offsets for: channel + 4 message frames (header, parent, metadata, content) + end marker
        let offset_count = 6u64;
        let header_size = 8 + (offset_count * 8);

        let mut offsets = Vec::new();
        let mut offset = header_size;

        // Offset for channel start
        offsets.push(offset);
        offset += channel_bytes.len() as u64;

        // Offset for header start (end of channel)
        offsets.push(offset);
        offset += header_bytes.len() as u64;

        // Offset for parent_header start (end of header)
        offsets.push(offset);
        offset += parent_header_bytes.len() as u64;

        // Offset for metadata start (end of parent_header)
        offsets.push(offset);
        offset += metadata_bytes.len() as u64;

        // Offset for content start (end of metadata)
        offsets.push(offset);
        offset += content_bytes.len() as u64;

        // Final offset marking end of content
        offsets.push(offset);

        // Build binary message
        let mut data = Vec::new();

        // Write offset count
        data.extend_from_slice(&offset_count.to_le_bytes());

        // Write all offsets
        for off in &offsets {
            data.extend_from_slice(&off.to_le_bytes());
        }

        // Write data buffers in order
        data.extend_from_slice(channel_bytes);
        data.extend_from_slice(&header_bytes);
        data.extend_from_slice(&parent_header_bytes);
        data.extend_from_slice(&metadata_bytes);
        data.extend_from_slice(&content_bytes);

        Ok(data)
    }

    /// Send an execute request
    pub async fn send_execute_request(
        &mut self,
        code: &str,
        stop_on_error: bool,
        cell_id: Option<&str>,
    ) -> Result<String> {
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

        // Fix username to be empty (JupyterLab uses empty string)
        execute_request.header.username = String::new();

        // Add metadata with cell_id if provided
        if let Some(cell_id) = cell_id {
            execute_request.metadata = serde_json::json!({
                "trusted": true,
                "deletedCells": [],
                "recordTiming": false,
                "cellId": cell_id
            });
        }

        // Save message ID for correlation
        let msg_id = execute_request.header.msg_id.clone();

        // Serialize to WebSocket v1 binary format
        let binary_data = Self::serialize_to_binary(&execute_request, "shell")
            .context("Failed to serialize execute request")?;

        self.write
            .send(Message::Binary(binary_data))
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
                        .with_context(|| "Failed to parse text message".to_string())?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Binary(data))) => {
                    // Jupyter WebSocket protocol uses a length-prefixed binary format
                    // Format: [num_buffers(u64)] [offset1(u64)] [offset2(u64)] ... [offsetN(u64)] [data...]

                    if data.len() < 8 {
                        continue;
                    }

                    // Parse the binary blob format
                    if let Some(msg) = Self::parse_binary_message(&data) {
                        return Ok(Some(msg));
                    }
                }
                Some(Ok(Message::Close(_))) => {
                    return Ok(None);
                }
                Some(Ok(_)) => {
                    // Ignore other message types (ping, pong, etc.) and continue loop
                    continue;
                }
                Some(Err(e)) => {
                    return Err(e).context("WebSocket error");
                }
                None => {
                    return Ok(None);
                }
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
