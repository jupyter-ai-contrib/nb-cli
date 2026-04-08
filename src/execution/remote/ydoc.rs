//! Y.js WebSocket client for real-time notebook document synchronization

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use nbformat::v4::Output;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use url::Url;
use yrs::encoding::varint::VarInt;
use yrs::encoding::write::Write;
use yrs::types::ToJson;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Array, ArrayRef, Doc, Map, ReadTxn, StateVector, Transact, Update};

use super::output_conversion::{update_cell_execution_count, update_cell_outputs};

/// Outputs and execution_count read from a Y.js cell
pub struct YDocCellOutputs {
    pub execution_count: Option<i64>,
    /// (output_index, url) for outputs externalized by jupyter-server-documents
    pub externalized_urls: Vec<(usize, String)>,
}

/// Convert yrs::Any to serde_json::Value for JSON round-trip deserialization
fn any_to_json(any: &yrs::Any) -> serde_json::Value {
    match any {
        yrs::Any::Null | yrs::Any::Undefined => serde_json::Value::Null,
        yrs::Any::Bool(b) => serde_json::Value::Bool(*b),
        yrs::Any::Number(n) => serde_json::json!(*n),
        yrs::Any::BigInt(n) => serde_json::json!(*n),
        yrs::Any::String(s) => serde_json::Value::String(s.to_string()),
        yrs::Any::Array(arr) => serde_json::Value::Array(arr.iter().map(any_to_json).collect()),
        yrs::Any::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.to_string(), any_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        yrs::Any::Buffer(_) => serde_json::Value::Null,
    }
}

#[derive(Debug, Deserialize)]
struct FileIdResponse {
    id: String,
}

/// Y.js document client for syncing notebook changes with Jupyter Server
pub struct YDocClient {
    doc: Doc,
    ws: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    #[allow(dead_code)]
    file_id: String,
    /// Track the document state when we last synced, so we only send changes
    last_state: StateVector,
}

impl YDocClient {
    /// Connect to Y.js room for the given notebook
    pub async fn connect(server_url: String, token: String, notebook_path: String) -> Result<Self> {
        // Step 1: Get file ID from FileID API
        let file_id = Self::get_file_id(&server_url, &token, &notebook_path).await?;

        // Step 2: Connect to room WebSocket
        let ws_url = Self::build_room_ws_url(&server_url, &file_id, &token)?;

        let (ws_stream, _) = connect_async(&ws_url)
            .await
            .context("Failed to connect to Y.js room WebSocket")?;

        // Step 3: Initialize Y.js document
        let doc = Doc::new();

        let mut client = Self {
            doc,
            ws: ws_stream,
            file_id,
            last_state: StateVector::default(),
        };

        // Step 4: Perform Y.js sync handshake with timeout
        match tokio::time::timeout(std::time::Duration::from_secs(3), client.sync_handshake()).await
        {
            Ok(Ok(_)) => Ok(client),
            Ok(Err(e)) => Err(e).context("Failed to perform Y.js sync handshake"),
            Err(_) => Err(anyhow::anyhow!(
                "Y.js sync handshake timed out after 3 seconds"
            )),
        }
    }

    /// Get unique file ID for notebook path via FileID API
    async fn get_file_id(server_url: &str, token: &str, notebook_path: &str) -> Result<String> {
        let url = format!("{}/api/fileid/index", server_url);

        let http_client = HttpClient::new();
        let response = http_client
            .post(&url)
            .query(&[("path", notebook_path)])
            .header("Authorization", format!("token {}", token))
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    anyhow::anyhow!(
                        "nb has been configured for remote mode, but no server is running at {}.\n\
                         To disable remote mode, run `nb disconnect` or make sure the server is running.",
                        server_url
                    )
                } else {
                    anyhow::anyhow!("Failed to call FileID API: {}", e)
                }
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "FileID API request failed with status {}: {}. \
                 Make sure jupyter-server-documents is installed: \
                 pip install jupyter-server-documents",
                status,
                error_text
            );
        }

        let file_id_response: FileIdResponse = response
            .json()
            .await
            .context("Failed to parse FileID API response")?;

        Ok(file_id_response.id)
    }

    /// Build WebSocket URL for Y.js room
    fn build_room_ws_url(server_url: &str, file_id: &str, token: &str) -> Result<String> {
        // Parse base URL to extract host and port
        let base_url = Url::parse(server_url).context("Invalid server URL")?;

        let host = base_url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("No host in server URL"))?;

        let port = base_url.port().unwrap_or(if base_url.scheme() == "https" {
            443
        } else {
            8888
        });

        // Build WebSocket URL with json:notebook: prefix
        let ws_scheme = if base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };

        let ws_url = format!(
            "{}://{}:{}/api/collaboration/room/json:notebook:{}?token={}",
            ws_scheme, host, port, file_id, token
        );

        Ok(ws_url)
    }

    /// Perform Y.js sync protocol handshake
    async fn sync_handshake(&mut self) -> Result<()> {
        // Step 1: Send our state vector (SyncStep1)
        let state_vector = self.doc.transact().state_vector();
        let sv_bytes = state_vector.encode_v1();

        // Build message: [SYNC=0, SYNC_STEP1=0, length_varint, state_vector_bytes]
        let mut msg: Vec<u8> = Vec::new();
        msg.write_u8(0); // YMessageType.SYNC
        msg.write_u8(0); // YSyncMessageType.SYNC_STEP1
        (sv_bytes.len() as u32).write(&mut msg);
        msg.extend_from_slice(&sv_bytes);

        self.ws
            .send(Message::Binary(msg))
            .await
            .context("Failed to send SyncStep1")?;

        // Step 2: Receive messages until we get SyncStep2
        let mut received_sync_step2 = false;

        while !received_sync_step2 {
            let msg_result = self.ws.next().await;

            if msg_result.is_none() {
                return Err(anyhow::anyhow!(
                    "WebSocket closed during handshake - connection terminated by server"
                ));
            }

            let msg = msg_result.unwrap()?;

            match msg {
                Message::Binary(data) => {
                    if data.len() < 2 {
                        continue;
                    }

                    let y_msg_type = data[0];
                    let sync_msg_type = data[1];
                    let payload_with_length = &data[2..];

                    // Only handle SYNC messages (type 0)
                    if y_msg_type != 0 {
                        continue;
                    }

                    // Decode the length prefix and get actual payload
                    let mut decoder = yrs::encoding::read::Cursor::new(payload_with_length);
                    let payload_length =
                        u32::read(&mut decoder).context("Failed to read payload length")?;

                    let payload_start = decoder.next;
                    let payload = &payload_with_length
                        [payload_start..payload_start + payload_length as usize];

                    match sync_msg_type {
                        0 => {
                            // SyncStep1 from server - send SyncStep2 in response
                            let server_state = StateVector::decode_v1(payload)
                                .context("Failed to decode server state vector")?;

                            let response = {
                                let txn = self.doc.transact();
                                let update = txn.encode_state_as_update_v1(&server_state);

                                let mut buf: Vec<u8> = Vec::new();
                                buf.write_u8(0);
                                buf.write_u8(1);
                                (update.len() as u32).write(&mut buf);
                                buf.extend_from_slice(&update);
                                buf
                            };

                            self.ws
                                .send(Message::Binary(response))
                                .await
                                .context("Failed to send SyncStep2")?;
                        }
                        1 => {
                            // SyncStep2 from server - apply updates
                            let update =
                                Update::decode_v1(payload).context("Failed to decode update")?;

                            {
                                let mut txn = self.doc.transact_mut();
                                let _ = txn.apply_update(update);
                            }

                            received_sync_step2 = true;
                            self.last_state = self.doc.transact().state_vector();
                        }
                        2 => {
                            // Regular update message - apply it
                            let update =
                                Update::decode_v1(payload).context("Failed to decode update")?;

                            let mut txn = self.doc.transact_mut();
                            let _ = txn.apply_update(update);
                        }
                        _ => {
                            // Unknown sync message type - ignore
                        }
                    }
                }
                Message::Close(_) => {
                    return Err(anyhow::anyhow!(
                        "Server closed WebSocket connection during handshake"
                    ));
                }
                _ => {
                    // Ignore other message types (Text, Ping, Pong, Frame)
                }
            }
        }

        Ok(())
    }

    /// Update cell outputs in the Y.js document
    #[allow(dead_code)]
    pub fn update_cell_outputs(&mut self, cell_index: usize, outputs: Vec<Output>) -> Result<()> {
        let cells_array: ArrayRef = self.doc.get_or_insert_array("cells");
        let mut txn = self.doc.transact_mut();

        update_cell_outputs(&mut txn, &cells_array, cell_index, &outputs)
            .context("Failed to update cell outputs")?;

        Ok(())
    }

    /// Update cell execution_count in the Y.js document
    #[allow(dead_code)]
    pub fn update_cell_execution_count(
        &mut self,
        cell_index: usize,
        execution_count: Option<i64>,
    ) -> Result<()> {
        let cells_array: ArrayRef = self.doc.get_or_insert_array("cells");
        let mut txn = self.doc.transact_mut();

        update_cell_execution_count(&mut txn, &cells_array, cell_index, execution_count)
            .context("Failed to update execution count")?;

        Ok(())
    }

    /// Synchronize changes to the server (broadcast updates)
    pub async fn sync(&mut self) -> Result<()> {
        // Check if the server sent us a SyncStep1 (asking for our updates)
        match tokio::time::timeout(std::time::Duration::from_millis(100), self.ws.next()).await {
            Ok(Some(Ok(Message::Binary(data)))) => {
                if data.len() >= 2 {
                    let y_msg_type = data[0];
                    let sync_msg_type = data[1];

                    if y_msg_type == 0 && sync_msg_type == 0 {
                        // Server sent SyncStep1 - respond with SyncStep2
                        let payload_with_length = &data[2..];
                        let mut decoder = yrs::encoding::read::Cursor::new(payload_with_length);
                        let _payload_length =
                            u32::read(&mut decoder).context("Failed to read payload length")?;

                        let payload_start = decoder.next;
                        let payload = &payload_with_length[payload_start..];

                        let server_state = StateVector::decode_v1(payload)
                            .context("Failed to decode server state vector")?;

                        let response = {
                            let txn = self.doc.transact();
                            let update = txn.encode_state_as_update_v1(&server_state);

                            let mut buf: Vec<u8> = Vec::new();
                            buf.write_u8(0);
                            buf.write_u8(1);
                            (update.len() as u32).write(&mut buf);
                            buf.extend_from_slice(&update);

                            self.last_state = txn.state_vector();
                            buf
                        };

                        self.ws
                            .send(Message::Binary(response))
                            .await
                            .context("Failed to send SyncStep2")?;

                        self.ws.flush().await.context("Failed to flush WebSocket")?;

                        return Ok(());
                    }
                }
            }
            Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => {
                // Ignore other messages or timeout
            }
        }

        // If we didn't receive SyncStep1, send a SYNC_UPDATE proactively
        let (msg, new_state) = {
            let txn = self.doc.transact();
            let update = txn.encode_state_as_update_v1(&self.last_state);

            // Check if there are actually any changes
            if update.is_empty() || update == vec![0, 0] {
                return Ok(());
            }

            let mut buf: Vec<u8> = Vec::new();
            buf.write_u8(0);
            buf.write_u8(2);
            (update.len() as u32).write(&mut buf);
            buf.extend_from_slice(&update);

            (buf, txn.state_vector())
        };

        self.ws
            .send(Message::Binary(msg))
            .await
            .context("Failed to send update to server")?;

        self.last_state = new_state;
        self.ws.flush().await.context("Failed to flush WebSocket")?;

        Ok(())
    }

    /// Get a reference to the Y.js document
    pub fn get_doc(&self) -> &Doc {
        &self.doc
    }

    /// Try to receive a message from the WebSocket (non-blocking)
    /// Returns None if no message is available immediately
    #[allow(dead_code)]
    pub async fn try_receive_message(&mut self) -> Option<Message> {
        match tokio::time::timeout(std::time::Duration::from_millis(100), self.ws.next()).await {
            Ok(Some(Ok(msg))) => Some(msg),
            _ => None,
        }
    }

    /// Receive and apply the next Y.js update from the WebSocket.
    /// Returns Ok(true) if an update was applied, Ok(false) if no data, Err on failure.
    pub async fn recv_update(&mut self) -> Result<bool> {
        let msg = match self.ws.next().await {
            Some(Ok(Message::Binary(data))) => data,
            Some(Ok(Message::Close(_))) | None => return Ok(false),
            Some(Ok(_)) => return Ok(false),
            Some(Err(e)) => return Err(e).context("Y.js WebSocket error"),
        };

        if msg.len() < 2 || msg[0] != 0 {
            return Ok(false);
        }

        let sync_msg_type = msg[1];
        let payload_with_length = &msg[2..];
        let mut decoder = yrs::encoding::read::Cursor::new(payload_with_length);
        let payload_length = u32::read(&mut decoder).context("Failed to read payload length")?;
        let payload_start = decoder.next;
        let payload = &payload_with_length[payload_start..payload_start + payload_length as usize];

        match sync_msg_type {
            0 => {
                // SyncStep1 from server — respond with SyncStep2
                let server_state = StateVector::decode_v1(payload)?;
                let response = {
                    let txn = self.doc.transact();
                    let update = txn.encode_state_as_update_v1(&server_state);
                    let mut buf: Vec<u8> = Vec::new();
                    buf.write_u8(0);
                    buf.write_u8(1);
                    (update.len() as u32).write(&mut buf);
                    buf.extend_from_slice(&update);
                    buf
                };
                self.ws.send(Message::Binary(response)).await?;
                Ok(false)
            }
            1 | 2 => {
                // SyncStep2 or Update — apply to doc
                let update = Update::decode_v1(payload)?;
                {
                    let mut txn = self.doc.transact_mut();
                    let _ = txn.apply_update(update);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Read outputs and execution_count for a cell from the Y.js document.
    /// If outputs are externalized (metadata.url present), fetches actual content from the server.
    pub fn read_cell_outputs(&self, cell_index: usize) -> Result<YDocCellOutputs> {
        let cells_array: ArrayRef = self.doc.get_or_insert_array("cells");
        let txn = self.doc.transact();

        let cell_value = cells_array
            .get(&txn, cell_index as u32)
            .context("Cell index out of bounds in Y.js doc")?;
        let cell_map: yrs::MapRef = cell_value
            .cast()
            .map_err(|_| anyhow::anyhow!("Cell is not a Map"))?;

        // Read execution_count
        let execution_count =
            cell_map
                .get(&txn, "execution_count")
                .and_then(|v| match v.to_json(&txn) {
                    yrs::Any::BigInt(n) => Some(n),
                    yrs::Any::Number(n) => Some(n as i64),
                    _ => None,
                });

        // Read outputs array — collect URLs for externalized outputs
        let mut urls: Vec<(usize, String)> = Vec::new();

        if let Some(outputs_val) = cell_map.get(&txn, "outputs") {
            if let Ok(arr) = outputs_val.cast::<ArrayRef>() {
                let len = arr.len(&txn);
                for i in 0..len {
                    if let Some(item) = arr.get(&txn, i) {
                        let json_val = item.to_json(&txn);
                        let json = any_to_json(&json_val);
                        // Check for externalized output (metadata.url present)
                        if let Some(url) = json
                            .get("metadata")
                            .and_then(|m| m.get("url"))
                            .and_then(|u| u.as_str())
                        {
                            urls.push((i as usize, url.to_string()));
                        }
                    }
                }
            }
        }

        Ok(YDocCellOutputs {
            execution_count,
            externalized_urls: urls,
        })
    }

    /// Close the WebSocket connection
    pub async fn close(mut self) -> Result<()> {
        self.ws
            .close(None)
            .await
            .context("Failed to close WebSocket")?;
        Ok(())
    }
}
