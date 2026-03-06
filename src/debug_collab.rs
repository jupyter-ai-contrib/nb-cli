use crate::execution::remote::ydoc::YDocClient;
use anyhow::Result;
use yrs::{types::ToJson, Array, ArrayRef, Doc, GetString, Map, ReadTxn, Transact};

/// Debug the collaboration API connection and sync process
///
/// This connects to a Jupyter server's collaboration API, performs the initial
/// sync, and logs all the details for debugging.
pub async fn debug_collaboration_sync(
    server_url: String,
    token: String,
    notebook_path: String,
) -> Result<()> {
    println!("=== Starting Collaboration API Debug ===");
    println!("Server URL: {}", server_url);
    println!("Token: {}", token);
    println!("Notebook path: {}", notebook_path);
    println!();

    // Step 1: Connect to the Y.js document
    println!("Step 1: Connecting to Y.js document...");
    match YDocClient::connect(server_url.clone(), token.clone(), notebook_path.clone()).await {
        Ok(mut ydoc_client) => {
            println!("✓ Successfully connected to Y.js document");
            println!();

            // Step 2: Check if server sends more messages after handshake
            println!("Step 2: Checking for any server messages after handshake...");
            println!("  (Waiting 1 second to see if server sends SyncStep1 or other messages)");

            // Wait briefly to see if server sends anything
            tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

            if let Some(msg) = ydoc_client.try_receive_message().await {
                println!("  📨 Received message from server after handshake!");
                println!("     Message type: {:?}", msg);
            } else {
                println!("  ℹ No additional messages from server (this is expected)");
            }
            println!();

            // Step 3: Inspect the document state
            println!("Step 3: Document state after initial handshake:");
            inspect_ydoc_details(ydoc_client.get_doc());
            println!();

            // Step 4: Make a local change and see what happens
            println!("Step 4: Making a test change to the document...");
            println!("  (This simulates what would happen when you write to a notebook)");

            // Make a dummy change - just to see the sync behavior
            let test_change_result = ydoc_client.test_make_change();
            println!("  Change made: {:?}", test_change_result);
            println!();

            // Step 5: Call sync() to send the change
            println!("Step 5: Calling sync() to send changes to server...");
            match ydoc_client.sync().await {
                Ok(_) => {
                    println!("✓ Sync completed successfully");
                    println!();
                }
                Err(e) => {
                    println!("✗ Sync failed: {}", e);
                    println!("  Error details: {:?}", e);
                    return Err(e);
                }
            }

            // Step 6: Check if server responds to our update
            println!("Step 6: Checking if server responds to our update...");
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            if let Some(msg) = ydoc_client.try_receive_message().await {
                println!("  📨 Received response from server!");
                println!("     Message type: {:?}", msg);
            } else {
                println!("  ℹ No response from server");
            }
            println!();

            // Step 7: Close connection
            println!("Step 7: Closing connection...");
            match ydoc_client.close().await {
                Ok(_) => {
                    println!("✓ Connection closed successfully");
                }
                Err(e) => {
                    println!("⚠ Warning: Error closing connection: {}", e);
                }
            }
        }
        Err(e) => {
            println!("✗ Failed to connect to Y.js document: {}", e);
            println!("  Error details: {:?}", e);
            println!();
            println!("Possible causes:");
            println!("  - jupyter-collaboration extension not installed");
            println!("  - Notebook not open in JupyterLab");
            println!("  - Incorrect server URL or token");
            println!("  - Network connectivity issues");
            return Err(e);
        }
    }

    println!();
    println!("=== Debug Complete ===");
    Ok(())
}

/// Inspect Y.js document structure in detail - discovering what's there
fn inspect_ydoc_details(doc: &Doc) {
    println!("  === Y.js Document Details ===");

    // Get transaction to read current state
    let txn = doc.transact();
    println!("  State vector: {:?}", txn.state_vector());
    println!();

    // Unfortunately, yrs doesn't provide a way to enumerate all keys in a Doc
    // We can only get specific types if we know their names
    // Let's print the document as a debug string to see if that reveals anything
    println!("  Document debug info: {:?}", doc);
    println!();

    // Try the known "cells" key that Jupyter notebooks use
    drop(txn); // Drop read transaction

    println!("  Attempting to access 'cells' array...");
    let cells_array: ArrayRef = doc.get_or_insert_array("cells");
    let txn2 = doc.transact();
    let cells_len = cells_array.len(&txn2);
    println!("  Found 'cells' array with {} items", cells_len);
    println!();

    if cells_len > 0 {
        inspect_cells_array(&txn2, &cells_array);
    } else {
        println!("  (Array exists but is empty - notebook might not be open in JupyterLab)");
    }

    println!();
    println!("  === End Document Details ===");
}

/// Inspect cells array in detail
fn inspect_cells_array<T: ReadTxn>(txn: &T, cells: &ArrayRef) {
    let cells_len = cells.len(txn);
    println!("  === Cells Array Detail ===");
    println!("  Number of cells: {}", cells_len);
    println!();

    // Inspect first few cells in detail
    let max_cells_to_show = 3.min(cells_len);

    for i in 0..max_cells_to_show {
        println!("  Cell {}:", i);

        // Try to cast to a Map (cells are typically maps)
        if let Some(cell_val) = cells.get(txn, i) {
            if let Ok(cell_map) = cell_val.cast::<yrs::MapRef>() {
                // Get all keys in the cell
                let keys: Vec<_> = cell_map.keys(txn).collect();
                println!("    Keys: {:?}", keys);

                // Show specific interesting fields
                if let Some(cell_type_val) = cell_map.get(txn, "cell_type") {
                    let cell_type = cell_type_val.to_string(txn);
                    println!("    cell_type: '{}'", cell_type);
                }

                if let Some(id_val) = cell_map.get(txn, "id") {
                    let id = id_val.to_string(txn);
                    println!("    id: '{}'", id);
                }

                if let Some(source_val) = cell_map.get(txn, "source") {
                    if let Ok(source_text) = source_val.cast::<yrs::TextRef>() {
                        let source_str = source_text.get_string(txn);
                        let preview = if source_str.len() > 60 {
                            format!("{}...", &source_str[..60].replace('\n', "\\n"))
                        } else {
                            source_str.replace('\n', "\\n")
                        };
                        println!("    source: \"{}\"", preview);
                    }
                }

                if let Some(outputs_val) = cell_map.get(txn, "outputs") {
                    if let Ok(outputs_arr) = outputs_val.cast::<yrs::ArrayRef>() {
                        println!("    outputs: <Array> (length: {})", outputs_arr.len(txn));
                    }
                }

                if let Some(exec_count_val) = cell_map.get(txn, "execution_count") {
                    let json_value = exec_count_val.to_json(txn);
                    println!("    execution_count: {:?}", json_value);
                }
            } else {
                println!("    (Not a Map - unexpected structure)");
            }
        }
        println!();
    }

    if cells_len > max_cells_to_show {
        println!(
            "  ... {} more cells not shown",
            cells_len - max_cells_to_show
        );
    }

    println!("  === End Cells Detail ===");
}
