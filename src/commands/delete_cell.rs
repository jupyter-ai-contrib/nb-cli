use crate::commands::common::{self, OutputFormat};
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Parser)]
pub struct DeleteCellArgs {
    /// Path to notebook file
    pub file: String,

    /// Cell ID(s) to delete (stable identifier)
    #[arg(short = 'c', long = "cell", value_name = "ID", conflicts_with_all = ["cell_index", "range"])]
    pub cell: Vec<String>,

    /// Cell index(es) to delete (supports negative indexing)
    #[arg(short = 'i', long = "cell-index", value_name = "INDEX", allow_negative_numbers = true, conflicts_with_all = ["cell", "range"])]
    pub cell_index: Vec<i32>,

    /// Delete range [start, end) (exclusive end)
    #[arg(short = 'r', long = "range", value_name = "START:END", conflicts_with_all = ["cell", "cell_index"])]
    pub range: Option<String>,

    /// Jupyter server URL (for real-time updates if notebook is open)
    #[arg(long)]
    pub server: Option<String>,

    /// Authentication token for Jupyter server
    #[arg(long)]
    pub token: Option<String>,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct DeleteCellResult {
    file: String,
    cells_deleted: usize,
    remaining_cells: usize,
}

pub fn execute(args: DeleteCellArgs) -> Result<()> {
    // Check if we should use real-time Y.js updates by resolving execution mode
    use crate::execution::types::ExecutionMode;
    let mode = common::resolve_execution_mode(args.server.clone(), args.token.clone())?;
    let use_realtime = matches!(mode, ExecutionMode::Remote { .. });

    if use_realtime {
        // Create Tokio runtime for async operations
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        return runtime.block_on(execute_with_realtime(args, mode));
    }

    // Fallback to file-based updates
    execute_file_based(args)
}

async fn execute_with_realtime(
    args: DeleteCellArgs,
    mode: crate::execution::types::ExecutionMode,
) -> Result<()> {
    use crate::execution::remote::ydoc_notebook_ops;

    let (server_url, token) = match mode {
        crate::execution::types::ExecutionMode::Remote { server_url, token } => (server_url, token),
        _ => bail!("Expected remote execution mode"),
    };

    // Normalize notebook path
    let file_path = common::normalize_notebook_path(&args.file);

    // Compute notebook path relative to server root for Y.js connection
    let server_root = common::resolve_server_root();
    let notebook_server_path = common::notebook_path_for_server(&file_path, server_root.as_deref());

    // Read notebook to calculate which cells to delete
    let notebook = notebook::read_notebook(&file_path).context("Failed to read notebook")?;

    // Collect indices to delete
    let mut indices_to_delete: HashSet<usize> = HashSet::new();

    if !args.cell.is_empty() {
        // Delete by IDs
        for id in &args.cell {
            let (index, _) = common::find_cell_by_id(&notebook.cells, id)?;
            indices_to_delete.insert(index);
        }
    } else if !args.cell_index.is_empty() {
        // Delete by indices
        for idx in &args.cell_index {
            let normalized = common::normalize_index(*idx, notebook.cells.len())?;
            indices_to_delete.insert(normalized);
        }
    } else if let Some(ref range_str) = args.range {
        // Delete by range
        let (start, end) = parse_range(range_str, notebook.cells.len())?;
        for i in start..end {
            indices_to_delete.insert(i);
        }
    } else {
        bail!("Must specify --cell, --cell-index, or --range");
    }

    // Validate we're not deleting all cells
    if indices_to_delete.len() >= notebook.cells.len() {
        bail!("Cannot delete all cells from notebook (must keep at least 1 cell)");
    }

    // Sort indices in descending order (delete from end to avoid index shifting)
    let mut sorted_indices: Vec<usize> = indices_to_delete.into_iter().collect();
    sorted_indices.sort_by(|a, b| b.cmp(a)); // Sort in reverse order

    let cells_deleted = sorted_indices.len();
    let remaining_cells = notebook.cells.len() - cells_deleted;

    // Delete cells via Y.js (don't write to file - let JupyterLab handle persistence)
    ydoc_notebook_ops::ydoc_delete_cells(
        &server_url,
        &token,
        &notebook_server_path,
        &sorted_indices,
    )
    .await
    .context("Error deleting cells")?;

    // Output result
    let result = DeleteCellResult {
        file: file_path.clone(),
        cells_deleted,
        remaining_cells,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn execute_file_based(args: DeleteCellArgs) -> Result<()> {
    // Normalize notebook path
    let file_path = common::normalize_notebook_path(&args.file);

    // Read notebook
    let mut notebook = notebook::read_notebook(&file_path).context("Failed to read notebook")?;

    // Collect indices to delete
    let mut indices_to_delete: HashSet<usize> = HashSet::new();

    if !args.cell.is_empty() {
        // Delete by IDs
        for id in &args.cell {
            let (index, _) = common::find_cell_by_id(&notebook.cells, id)?;
            indices_to_delete.insert(index);
        }
    } else if !args.cell_index.is_empty() {
        // Delete by indices
        for idx in &args.cell_index {
            let normalized = common::normalize_index(*idx, notebook.cells.len())?;
            indices_to_delete.insert(normalized);
        }
    } else if let Some(ref range_str) = args.range {
        // Delete by range
        let (start, end) = parse_range(range_str, notebook.cells.len())?;
        for i in start..end {
            indices_to_delete.insert(i);
        }
    } else {
        bail!("Must specify --cell, --cell-index, or --range");
    }

    // Validate we're not deleting all cells
    if indices_to_delete.len() >= notebook.cells.len() {
        bail!("Cannot delete all cells from notebook (must keep at least 1 cell)");
    }

    // Sort indices in descending order (delete from end to avoid index shifting)
    let mut sorted_indices: Vec<usize> = indices_to_delete.into_iter().collect();
    sorted_indices.sort_by(|a, b| b.cmp(a)); // Sort in reverse order

    // Delete cells
    for idx in &sorted_indices {
        notebook.cells.remove(*idx);
    }

    let cells_deleted = sorted_indices.len();

    // Write notebook atomically
    notebook::write_notebook_atomic(&file_path, &notebook).context("Failed to write notebook")?;

    // Output result
    let result = DeleteCellResult {
        file: file_path.clone(),
        cells_deleted,
        remaining_cells: notebook.cells.len(),
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn parse_range(range: &str, max_len: usize) -> Result<(usize, usize)> {
    let parts: Vec<&str> = range.split(':').collect();
    if parts.len() != 2 {
        bail!("Range must be in format START:END (e.g., 2:5)");
    }

    let start_str = parts[0].trim();
    let end_str = parts[1].trim();

    // Parse start index
    let start = if start_str.is_empty() {
        0
    } else {
        let start_i32: i32 = start_str.parse().context("Invalid start index in range")?;
        common::normalize_index(start_i32, max_len)?
    };

    // Parse end index (can be one past the end for exclusive range)
    let end = if end_str.is_empty() {
        max_len
    } else {
        let end_i32: i32 = end_str.parse().context("Invalid end index in range")?;
        if end_i32 < 0 {
            let abs_idx = end_i32.unsigned_abs() as usize;
            if abs_idx > max_len {
                bail!(
                    "Negative end index {} out of range (notebook has {} cells)",
                    end_i32,
                    max_len
                );
            }
            max_len - abs_idx
        } else {
            let end_usize = end_i32 as usize;
            if end_usize > max_len {
                bail!(
                    "End index {} out of range (notebook has {} cells, max end is {})",
                    end_i32,
                    max_len,
                    max_len
                );
            }
            end_usize
        }
    };

    // Validate range
    if start >= end {
        bail!(
            "Invalid range: start ({}) must be less than end ({})",
            start,
            end
        );
    }

    Ok((start, end))
}

fn output_result(result: &DeleteCellResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!(
                "Deleted {} cell(s) from: {}",
                result.cells_deleted, result.file
            );
            println!("Remaining cells: {}", result.remaining_cells);
        }
    }
    Ok(())
}
