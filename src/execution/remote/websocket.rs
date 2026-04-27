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

        // We need num_buffers offsets (each 8 bytes) plus the header.
        // Use checked arithmetic to guard against overflow on adversarial input.
        let header_size = (8usize).checked_add(num_buffers.checked_mul(8)?)?;
        if data.len() < header_size {
            return None;
        }

        // Read offsets
        let mut offsets = Vec::new();
        for i in 0..num_buffers {
            let offset_start = (8usize).checked_add(i.checked_mul(8)?)?;
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
        // Empty buffers (0 bytes) are valid in the Jupyter protocol (e.g., empty parent_header
        // or metadata). Default to empty JSON object to avoid dropping the entire message.
        let header: serde_json::Value = serde_json::from_slice(buffers[1]).ok()?;
        let parent_header: serde_json::Value = if buffers[2].is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(buffers[2]).ok()?
        };
        let metadata: serde_json::Value = if buffers[3].is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(buffers[3]).ok()?
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use jupyter_protocol::messaging::{JupyterMessage, JupyterMessageContent};

    fn make_execute_request_msg() -> JupyterMessage {
        JupyterMessage::new(
            JupyterMessageContent::ExecuteRequest(jupyter_protocol::ExecuteRequest {
                code: "x = 1".to_string(),
                silent: false,
                store_history: true,
                user_expressions: None,
                allow_stdin: false,
                stop_on_error: true,
            }),
            None,
        )
    }

    fn make_status_idle_msg() -> JupyterMessage {
        JupyterMessage::new(
            JupyterMessageContent::Status(jupyter_protocol::Status {
                execution_state: jupyter_protocol::ExecutionState::Idle,
            }),
            None,
        )
    }

    #[test]
    fn test_roundtrip_two_message_types() {
        // ExecuteRequest round-trip
        let req = make_execute_request_msg();
        let bytes = KernelWebSocket::serialize_to_binary(&req, "shell").unwrap();
        let parsed = KernelWebSocket::parse_binary_message(&bytes).unwrap();
        assert_eq!(parsed.header.msg_type, req.header.msg_type);
        assert_eq!(parsed.header.msg_id, req.header.msg_id);

        // Status(idle) round-trip
        let status = make_status_idle_msg();
        let bytes = KernelWebSocket::serialize_to_binary(&status, "iopub").unwrap();
        let parsed = KernelWebSocket::parse_binary_message(&bytes).unwrap();
        assert_eq!(parsed.header.msg_type, status.header.msg_type);
        assert_eq!(parsed.header.msg_id, status.header.msg_id);
    }

    #[test]
    fn test_parse_too_short_or_bad_buffer_count_returns_none() {
        // Fewer than 8 bytes → None
        assert!(KernelWebSocket::parse_binary_message(&[0u8; 7]).is_none());

        // num_buffers = 3 → fewer than 5 buffers → None
        // Build a blob: [3u64 LE][3 offsets of 0u64][no data]
        let mut data = Vec::new();
        data.extend_from_slice(&3u64.to_le_bytes());
        for _ in 0..3 {
            data.extend_from_slice(&0u64.to_le_bytes());
        }
        assert!(KernelWebSocket::parse_binary_message(&data).is_none());

        // Valid structure but content (buffer 4) contains invalid JSON → None
        let msg = make_execute_request_msg();
        let mut bytes = KernelWebSocket::serialize_to_binary(&msg, "shell").unwrap();
        // Corrupt the content buffer: write garbage starting at the content offset.
        // Read offset[4] from bytes to find content start.
        let offset_start = 8 + 4 * 8;
        let content_offset =
            u64::from_le_bytes(bytes[offset_start..offset_start + 8].try_into().unwrap()) as usize;
        let end_offset = u64::from_le_bytes(
            bytes[offset_start + 8..offset_start + 16]
                .try_into()
                .unwrap(),
        ) as usize;
        // Overwrite content with invalid JSON
        for b in bytes[content_offset..end_offset].iter_mut() {
            *b = b'!';
        }
        assert!(KernelWebSocket::parse_binary_message(&bytes).is_none());
    }

    #[test]
    fn test_parse_empty_parent_header_succeeds() {
        // The Jupyter protocol allows empty parent_header (buffer[2]).
        // The code handles this with a fallback to {} (lines 91-95 in source).
        let msg = make_execute_request_msg();
        let bytes = KernelWebSocket::serialize_to_binary(&msg, "shell").unwrap();

        // Zero out the parent_header buffer (buffer index 2).
        // offset[1] = start of header, offset[2] = end of header / start of parent_header
        // offset[3] = end of parent_header
        let offset2_start = 8 + 2 * 8;
        let offset3_start = 8 + 3 * 8;
        let ph_start =
            u64::from_le_bytes(bytes[offset2_start..offset2_start + 8].try_into().unwrap())
                as usize;
        let ph_end = u64::from_le_bytes(bytes[offset3_start..offset3_start + 8].try_into().unwrap())
            as usize;

        // Replace parent_header bytes with zeros (empty = 0 bytes won't work directly
        // because offset table still marks the range; fill with a 0-byte slice by
        // making offset[2] == offset[3]).
        // Instead, verify the current path: a non-empty parent_header parses fine.
        // Then test the empty case by constructing a minimal blob with empty buffer[2].
        assert!(ph_start < ph_end); // Parent header is non-empty in a normal message
        let _ = bytes; // suppress unused warning

        // Build a custom blob where parent_header (buffer 2) is empty (offset[2] == offset[3]).
        let channel_bytes = b"shell";
        let header_bytes = serde_json::to_vec(&msg.header).unwrap();
        let parent_header_bytes: &[u8] = &[]; // intentionally empty
        let metadata_bytes = serde_json::to_vec(&msg.metadata).unwrap();
        let content_bytes = serde_json::to_vec(&msg.content).unwrap();

        let offset_count = 6u64;
        let header_size = 8 + offset_count * 8;
        let mut offsets = Vec::new();
        let mut offset = header_size;
        offsets.push(offset);
        offset += channel_bytes.len() as u64;
        offsets.push(offset);
        offset += header_bytes.len() as u64;
        offsets.push(offset);
        offset += parent_header_bytes.len() as u64; // 0 bytes
        offsets.push(offset);
        offset += metadata_bytes.len() as u64;
        offsets.push(offset);
        offset += content_bytes.len() as u64;
        offsets.push(offset);

        let mut blob = Vec::new();
        blob.extend_from_slice(&offset_count.to_le_bytes());
        for off in &offsets {
            blob.extend_from_slice(&off.to_le_bytes());
        }
        blob.extend_from_slice(channel_bytes);
        blob.extend_from_slice(&header_bytes);
        // parent_header: 0 bytes
        blob.extend_from_slice(&metadata_bytes);
        blob.extend_from_slice(&content_bytes);

        // Must parse successfully even with empty parent_header
        let parsed = KernelWebSocket::parse_binary_message(&blob);
        assert!(
            parsed.is_some(),
            "Message with empty parent_header should parse successfully"
        );
    }

    #[test]
    fn test_serialize_offset_math_is_consistent() {
        let msg = make_execute_request_msg();
        let bytes = KernelWebSocket::serialize_to_binary(&msg, "shell").unwrap();

        // Read offset_count from the first 8 bytes
        let offset_count = u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize;
        assert_eq!(
            offset_count, 6,
            "Expected 6 offsets (channel + 4 frames + end)"
        );

        // Read all offsets
        let offsets: Vec<usize> = (0..offset_count)
            .map(|i| {
                let start = 8 + i * 8;
                u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap()) as usize
            })
            .collect();

        // The 5 data sections are: channel, header, parent_header, metadata, content
        // offset[5] (end marker) must equal blob length
        assert_eq!(
            offsets[offset_count - 1],
            bytes.len(),
            "Final offset must equal total blob length"
        );

        // Each section length must equal the difference between consecutive offsets
        let section_lengths: Vec<usize> = (0..offset_count - 1)
            .map(|i| offsets[i + 1] - offsets[i])
            .collect();

        let channel_bytes = b"shell";
        let header_bytes = serde_json::to_vec(&msg.header).unwrap();
        let parent_header_bytes = serde_json::to_vec(&msg.parent_header).unwrap();
        let metadata_bytes = serde_json::to_vec(&msg.metadata).unwrap();
        let content_bytes = serde_json::to_vec(&msg.content).unwrap();

        assert_eq!(
            section_lengths[0],
            channel_bytes.len(),
            "channel length mismatch"
        );
        assert_eq!(
            section_lengths[1],
            header_bytes.len(),
            "header length mismatch"
        );
        assert_eq!(
            section_lengths[2],
            parent_header_bytes.len(),
            "parent_header length mismatch"
        );
        assert_eq!(
            section_lengths[3],
            metadata_bytes.len(),
            "metadata length mismatch"
        );
        assert_eq!(
            section_lengths[4],
            content_bytes.len(),
            "content length mismatch"
        );
    }

    proptest::proptest! {
        /// parse_binary_message must never panic on arbitrary byte input.
        #[test]
        fn prop_parse_random_bytes_never_panics(data in proptest::collection::vec(0u8..=255u8, 0..512)) {
            let _ = KernelWebSocket::parse_binary_message(&data);
        }
    }
}
