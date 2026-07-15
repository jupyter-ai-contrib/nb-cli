use crate::commands::common::{self, OutputFormat};
use crate::execution::server::ydoc_notebook_ops;
use crate::notebook::session::{resolve_backend, run_mutation, CellMutator};
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::Notebook;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Parser, Clone)]
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

/// Resolve the set of cell indices to delete from `--cell`/`--cell-index`/
/// `--range` (mutually exclusive), and validate at least one cell remains.
fn resolve_delete_indices(
    args: &DeleteCellArgs,
    cells: &[nbformat::v4::Cell],
) -> Result<Vec<usize>> {
    let mut indices_to_delete: HashSet<usize> = HashSet::new();

    if !args.cell.is_empty() {
        for id in &args.cell {
            let (index, _) = common::find_cell_by_id(cells, id)?;
            indices_to_delete.insert(index);
        }
    } else if !args.cell_index.is_empty() {
        for idx in &args.cell_index {
            let normalized = common::normalize_index(*idx, cells.len())?;
            indices_to_delete.insert(normalized);
        }
    } else if let Some(ref range_str) = args.range {
        let (start, end) = parse_range(range_str, cells.len())?;
        for i in start..end {
            indices_to_delete.insert(i);
        }
    } else {
        bail!("Must specify --cell, --cell-index, or --range");
    }

    if indices_to_delete.len() >= cells.len() {
        bail!("Cannot delete all cells from notebook (must keep at least 1 cell)");
    }

    // Sort indices in descending order (delete from end to avoid index shifting)
    let mut sorted_indices: Vec<usize> = indices_to_delete.into_iter().collect();
    sorted_indices.sort_by(|a, b| b.cmp(a));
    Ok(sorted_indices)
}

struct DeleteCellMutator<'a> {
    args: &'a DeleteCellArgs,
}

#[async_trait::async_trait]
impl CellMutator for DeleteCellMutator<'_> {
    /// (cells_deleted, remaining_cells)
    type Output = (usize, usize);

    fn mutate_notebook(&self, notebook: &mut Notebook, _file_path: &str) -> Result<Self::Output> {
        let sorted_indices = resolve_delete_indices(self.args, &notebook.cells)?;
        for idx in &sorted_indices {
            notebook.cells.remove(*idx);
        }
        Ok((sorted_indices.len(), notebook.cells.len()))
    }

    async fn mutate_realtime(
        &self,
        server_url: &str,
        token: &str,
        server_path: &str,
        _file_path: &str,
    ) -> Result<Self::Output> {
        let notebook =
            crate::notebook::remote::read_notebook_remote(server_url, token, server_path).await?;

        let sorted_indices = resolve_delete_indices(self.args, &notebook.cells)?;
        let cells_deleted = sorted_indices.len();
        let remaining_cells = notebook.cells.len() - cells_deleted;

        // Delete cells via Y.js (don't write to file - let JupyterLab handle persistence)
        ydoc_notebook_ops::ydoc_delete_cells(server_url, token, server_path, &sorted_indices)
            .await
            .context("Error deleting cells")?;

        Ok((cells_deleted, remaining_cells))
    }
}

pub fn execute(args: DeleteCellArgs) -> Result<()> {
    let backend = resolve_backend(&args.file, args.server.clone(), args.token.clone())?;
    let mutator = DeleteCellMutator { args: &args };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (file_path, (cells_deleted, remaining_cells)) =
        runtime.block_on(run_mutation(backend, &mutator))?;

    let result = DeleteCellResult {
        file: file_path,
        cells_deleted,
        remaining_cells,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)
}

fn output_result(result: &DeleteCellResult, format: &OutputFormat) -> Result<()> {
    common::print_result(result, format, |result| {
        println!(
            "Deleted {} cell(s) from: {}",
            result.cells_deleted, result.file
        );
        println!("Remaining cells: {}", result.remaining_cells);
    })
}
