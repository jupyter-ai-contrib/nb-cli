//! Y.js operations for notebook manipulation (add cell, update cell, etc.)

use anyhow::{bail, Context, Result};
use nbformat::v4::Cell;
use yrs::types::ToJson;
use yrs::{Any, Array, ArrayPrelim, ArrayRef, Map, MapPrelim, MapRef, Text, TextPrelim, Transact};

use super::ydoc::YDocClient;

/// Selector for which cells to clear outputs from
pub enum ClearCellSelector {
    /// Clear all code cells
    All,
    /// Clear a specific cell by ID
    ById(String),
    /// Clear a specific cell by index (supports negative indexing)
    ByIndex(i32),
}

/// Add a cell to the Y.js document
fn add_cell_to_ydoc(doc: &yrs::Doc, cell: &Cell, index: usize) -> Result<()> {
    let cells_array = doc.get_or_insert_array("cells");
    let mut txn = doc.transact_mut();

    // Insert an empty map to create the cell in the array
    let empty_map = MapPrelim::default();
    cells_array.insert(&mut txn, index as u32, empty_map);

    // Get a reference to the newly created map
    let cell_value = cells_array
        .get(&txn, index as u32)
        .context("Failed to get inserted cell")?;
    let cell_map: MapRef = cell_value
        .cast()
        .map_err(|_| anyhow::anyhow!("Failed to cast cell to MapRef"))?;

    match cell {
        Cell::Code {
            id,
            metadata,
            execution_count,
            source,
            outputs: _,
        } => {
            cell_map.insert(&mut txn, "cell_type", "code");
            cell_map.insert(&mut txn, "id", id.as_str());

            if let Some(count) = execution_count {
                cell_map.insert(&mut txn, "execution_count", *count);
            } else {
                cell_map.insert(&mut txn, "execution_count", Any::Null);
            }
            cell_map.insert(&mut txn, "execution_state", "idle");

            let metadata_prelim = if let Some(trusted) =
                metadata.additional.get("trusted").and_then(|v| v.as_bool())
            {
                MapPrelim::from([("trusted", Any::Bool(trusted))])
            } else {
                MapPrelim::default()
            };
            cell_map.insert(&mut txn, "metadata", metadata_prelim);

            let source_str = source_to_string(source);
            cell_map.insert(&mut txn, "source", TextPrelim::new(&source_str));

            cell_map.insert(&mut txn, "outputs", ArrayPrelim::default());
        }
        Cell::Markdown {
            id,
            metadata,
            source,
            ..
        } => {
            cell_map.insert(&mut txn, "cell_type", "markdown");
            cell_map.insert(&mut txn, "id", id.as_str());
            cell_map.insert(&mut txn, "execution_state", "idle");

            let metadata_prelim = if let Some(trusted) =
                metadata.additional.get("trusted").and_then(|v| v.as_bool())
            {
                MapPrelim::from([("trusted", Any::Bool(trusted))])
            } else {
                MapPrelim::default()
            };
            cell_map.insert(&mut txn, "metadata", metadata_prelim);

            let source_str = source_to_string(source);
            cell_map.insert(&mut txn, "source", TextPrelim::new(&source_str));
        }
        Cell::Raw {
            id,
            metadata,
            source,
        } => {
            cell_map.insert(&mut txn, "cell_type", "raw");
            cell_map.insert(&mut txn, "id", id.as_str());
            cell_map.insert(&mut txn, "execution_state", "idle");

            let metadata_prelim = if let Some(trusted) =
                metadata.additional.get("trusted").and_then(|v| v.as_bool())
            {
                MapPrelim::from([("trusted", Any::Bool(trusted))])
            } else {
                MapPrelim::default()
            };
            cell_map.insert(&mut txn, "metadata", metadata_prelim);

            let source_str = source_to_string(source);
            cell_map.insert(&mut txn, "source", TextPrelim::new(&source_str));
        }
    }

    Ok(())
}

/// Convert source Vec<String> to a single string
fn source_to_string(source: &[String]) -> String {
    source.join("")
}

/// Add cells to the notebook via Y.js in a single connection
pub async fn ydoc_add_cells(
    server_url: &str,
    token: &str,
    notebook_path: &str,
    cells: &[Cell],
    start_index: usize,
) -> Result<()> {
    // Connect to Y.js document
    let mut ydoc_client = YDocClient::connect(
        server_url.to_string(),
        token.to_string(),
        notebook_path.to_string(),
    )
    .await?;

    // Add all cells to the Y.js document consecutively
    for (i, cell) in cells.iter().enumerate() {
        add_cell_to_ydoc(ydoc_client.get_doc(), cell, start_index + i)
            .context("Failed to add cell to Y.js document")?;
    }

    // Sync changes
    ydoc_client.sync().await.context("Failed to sync changes")?;

    // Close connection
    ydoc_client.close().await?;

    Ok(())
}

/// Delete cells from the notebook via Y.js
pub async fn ydoc_delete_cells(
    server_url: &str,
    token: &str,
    notebook_path: &str,
    indices: &[usize],
) -> Result<()> {
    // Connect to Y.js document
    let mut ydoc_client = YDocClient::connect(
        server_url.to_string(),
        token.to_string(),
        notebook_path.to_string(),
    )
    .await?;

    // Delete cells from the Y.js document (in reverse order to maintain indices)
    delete_cells_from_ydoc(ydoc_client.get_doc(), indices)
        .context("Failed to delete cells from Y.js document")?;

    // Sync changes
    ydoc_client.sync().await.context("Failed to sync changes")?;

    // Close connection
    ydoc_client.close().await?;

    Ok(())
}

/// Delete cells from the Y.js document (expects indices in descending order)
fn delete_cells_from_ydoc(doc: &yrs::Doc, indices: &[usize]) -> Result<()> {
    let cells_array = doc.get_or_insert_array("cells");
    let mut txn = doc.transact_mut();

    // Delete cells in reverse order to avoid index shifting issues
    for &index in indices {
        cells_array.remove(&mut txn, index as u32);
    }

    Ok(())
}

/// Update an existing cell in the notebook via Y.js
pub async fn ydoc_update_cell(
    server_url: &str,
    token: &str,
    notebook_path: &str,
    cell_index: usize,
    new_source: Option<&str>,
    append_source: Option<&str>,
) -> Result<()> {
    // Connect to Y.js document
    let mut ydoc_client = YDocClient::connect(
        server_url.to_string(),
        token.to_string(),
        notebook_path.to_string(),
    )
    .await?;

    // Update the cell in the Y.js document
    update_cell_source_in_ydoc(ydoc_client.get_doc(), cell_index, new_source, append_source)
        .context("Failed to update cell in Y.js document")?;

    // Sync changes
    ydoc_client.sync().await.context("Failed to sync changes")?;

    // Close connection
    ydoc_client.close().await?;

    Ok(())
}

/// Update a cell's source in the Y.js document
fn update_cell_source_in_ydoc(
    doc: &yrs::Doc,
    cell_index: usize,
    new_source: Option<&str>,
    append_source: Option<&str>,
) -> Result<()> {
    let cells_array = doc.get_or_insert_array("cells");
    let mut txn = doc.transact_mut();

    // Get the cell at the specified index
    let cell_value = cells_array
        .get(&txn, cell_index as u32)
        .context(format!("Cell at index {} not found", cell_index))?;

    let cell_map: MapRef = cell_value
        .cast()
        .map_err(|_| anyhow::anyhow!("Cell at index {} is not a Map", cell_index))?;

    // Get the source field (should be a Y.Text)
    let source_value = cell_map
        .get(&txn, "source")
        .context("Cell does not have a source field")?;

    let source_text: yrs::TextRef = source_value
        .cast()
        .map_err(|_| anyhow::anyhow!("Source field is not a Y.Text"))?;

    // Update the source
    if let Some(new_text) = new_source {
        let current_len = source_text.len(&txn);
        if current_len > 0 {
            source_text.remove_range(&mut txn, 0, current_len);
        }
        source_text.insert(&mut txn, 0, new_text);
    } else if let Some(append_text) = append_source {
        let current_len = source_text.len(&txn);
        source_text.insert(&mut txn, current_len, append_text);
    }

    // Reset execution_count when updating source
    cell_map.insert(&mut txn, "execution_count", Any::Null);

    Ok(())
}

/// Clear outputs and execution_count for cells via Y.js
pub async fn ydoc_clear_outputs(
    server_url: &str,
    token: &str,
    notebook_path: &str,
    selector: ClearCellSelector,
) -> Result<usize> {
    let mut ydoc_client = YDocClient::connect(
        server_url.to_string(),
        token.to_string(),
        notebook_path.to_string(),
    )
    .await?;

    let cells_cleared = clear_outputs_in_ydoc(ydoc_client.get_doc(), selector)
        .context("Failed to clear outputs in Y.js document")?;

    ydoc_client.sync().await.context("Failed to sync changes")?;
    ydoc_client.close().await?;

    Ok(cells_cleared)
}

/// Clear outputs and execution_count for cells in the Y.js document
fn clear_outputs_in_ydoc(doc: &yrs::Doc, selector: ClearCellSelector) -> Result<usize> {
    let cells_array = doc.get_or_insert_array("cells");
    let mut txn = doc.transact_mut();
    let num_cells = cells_array.len(&txn) as usize;

    // Determine which indices to clear
    let indices: Vec<usize> = match selector {
        ClearCellSelector::All => (0..num_cells)
            .filter(|&i| {
                cell_type_at(&cells_array, &txn, i)
                    .map(|t| t == "code")
                    .unwrap_or(false)
            })
            .collect(),
        ClearCellSelector::ById(ref id) => {
            let idx = (0..num_cells)
                .find(|&i| {
                    cell_id_at(&cells_array, &txn, i)
                        .map(|cid| cid == *id)
                        .unwrap_or(false)
                })
                .ok_or_else(|| anyhow::anyhow!("Cell with ID '{}' not found in notebook", id))?;
            let ct = cell_type_at(&cells_array, &txn, idx).unwrap_or_default();
            if ct != "code" {
                bail!("Can only clear outputs from code cells");
            }
            vec![idx]
        }
        ClearCellSelector::ByIndex(raw_idx) => {
            let idx = normalize_ydoc_index(raw_idx, num_cells)?;
            let ct = cell_type_at(&cells_array, &txn, idx).unwrap_or_default();
            if ct != "code" {
                bail!("Can only clear outputs from code cells");
            }
            vec![idx]
        }
    };

    // Clear outputs and execution_count for each target cell
    for &i in &indices {
        let cell_value = cells_array
            .get(&txn, i as u32)
            .context("Cell index out of bounds")?;
        let cell_map: MapRef = cell_value
            .cast()
            .map_err(|_| anyhow::anyhow!("Cell is not a Map"))?;

        // Clear outputs array
        if let Some(outputs_val) = cell_map.get(&txn, "outputs") {
            if let Ok(arr) = outputs_val.cast::<ArrayRef>() {
                let len = arr.len(&txn);
                if len > 0 {
                    arr.remove_range(&mut txn, 0, len);
                }
            }
        }

        // Set execution_count to null
        cell_map.insert(&mut txn, "execution_count", Any::Null);
    }

    Ok(indices.len())
}

/// Read cell_type string from a cell in the Y.js cells array
fn cell_type_at(cells_array: &ArrayRef, txn: &yrs::TransactionMut, index: usize) -> Option<String> {
    let cell_value = cells_array.get(txn, index as u32)?;
    let cell_map: MapRef = cell_value.cast().ok()?;
    let val = cell_map.get(txn, "cell_type")?;
    match val.to_json(txn) {
        Any::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Read cell id string from a cell in the Y.js cells array
fn cell_id_at(cells_array: &ArrayRef, txn: &yrs::TransactionMut, index: usize) -> Option<String> {
    let cell_value = cells_array.get(txn, index as u32)?;
    let cell_map: MapRef = cell_value.cast().ok()?;
    let val = cell_map.get(txn, "id")?;
    match val.to_json(txn) {
        Any::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Normalize a potentially negative index against a cell count
fn normalize_ydoc_index(index: i32, len: usize) -> Result<usize> {
    if index < 0 {
        let abs = index.unsigned_abs() as usize;
        if abs > len {
            bail!(
                "Cell index {} out of range (notebook has {} cells)",
                index,
                len
            );
        }
        Ok(len - abs)
    } else {
        let idx = index as usize;
        if idx >= len {
            bail!(
                "Cell index {} out of range (notebook has {} cells)",
                index,
                len
            );
        }
        Ok(idx)
    }
}
