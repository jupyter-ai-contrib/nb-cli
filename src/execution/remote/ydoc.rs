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
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ArrayRef, Doc, ReadTxn, StateVector, Transact, Update};

use super::output_conversion::{update_cell_execution_count, update_cell_outputs};

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

                            let txn = self.doc.transact();
                            let update = txn.encode_state_as_update_v1(&server_state);

                            // Build response: [SYNC=0, SYNC_STEP2=1, length_varint, update_bytes]
                            let mut response: Vec<u8> = Vec::new();
                            response.write_u8(0);
                            response.write_u8(1);
                            (update.len() as u32).write(&mut response);
                            response.extend_from_slice(&update);

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

                        let txn = self.doc.transact();
                        let update = txn.encode_state_as_update_v1(&server_state);

                        // Build SyncStep2 response
                        let mut response: Vec<u8> = Vec::new();
                        response.write_u8(0);
                        response.write_u8(1);
                        (update.len() as u32).write(&mut response);
                        response.extend_from_slice(&update);

                        self.ws
                            .send(Message::Binary(response))
                            .await
                            .context("Failed to send SyncStep2")?;

                        self.last_state = txn.state_vector();
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
        let txn = self.doc.transact();
        let update = txn.encode_state_as_update_v1(&self.last_state);

        // Check if there are actually any changes
        if update.is_empty() || update == vec![0, 0] {
            return Ok(());
        }

        // Build update message: [SYNC=0, SYNC_UPDATE=2, length_varint, update_bytes]
        let mut msg: Vec<u8> = Vec::new();
        msg.write_u8(0);
        msg.write_u8(2);
        (update.len() as u32).write(&mut msg);
        msg.extend_from_slice(&update);

        self.ws
            .send(Message::Binary(msg))
            .await
            .context("Failed to send update to server")?;

        self.last_state = txn.state_vector();
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

    /// Close the WebSocket connection
    pub async fn close(mut self) -> Result<()> {
        self.ws
            .close(None)
            .await
            .context("Failed to close WebSocket")?;
        Ok(())
    }
}
