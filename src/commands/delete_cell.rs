use crate::commands::common;
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Text,
}

#[derive(Parser)]
pub struct DeleteCellArgs {
    /// Path to notebook file
    pub file: String,

    /// Cell index(es) to delete (supports negative)
    #[arg(short = 'c', long = "cell", value_name = "INDEX", conflicts_with_all = ["cell_id", "range"])]
    pub cell: Vec<i32>,

    /// Cell ID(s) to delete
    #[arg(short = 'i', long = "cell-id", value_name = "ID", conflicts_with_all = ["cell", "range"])]
    pub cell_id: Vec<String>,

    /// Delete range [start, end) (exclusive end)
    #[arg(short = 'r', long = "range", value_name = "START:END", conflicts_with_all = ["cell", "cell_id"])]
    pub range: Option<String>,

    /// Output format
    #[arg(short = 'f', long = "format", default_value = "json", value_name = "FORMAT")]
    pub format: OutputFormat,
}

#[derive(Serialize)]
struct DeleteCellResult {
    file: String,
    cells_deleted: usize,
    remaining_cells: usize,
}

pub fn execute(args: DeleteCellArgs) -> Result<()> {
    // Read notebook
    let mut notebook = notebook::read_notebook(&args.file)
        .context("Failed to read notebook")?;

    // Collect indices to delete
    let mut indices_to_delete: HashSet<usize> = HashSet::new();

    if !args.cell.is_empty() {
        // Delete by indices
        for idx in &args.cell {
            let normalized = common::normalize_index(*idx, notebook.cells.len())?;
            indices_to_delete.insert(normalized);
        }
    } else if !args.cell_id.is_empty() {
        // Delete by IDs
        for id in &args.cell_id {
            let (index, _) = common::find_cell_by_id(&notebook.cells, id)?;
            indices_to_delete.insert(index);
        }
    } else if let Some(ref range_str) = args.range {
        // Delete by range
        let (start, end) = parse_range(range_str, notebook.cells.len())?;
        for i in start..end {
            indices_to_delete.insert(i);
        }
    } else {
        bail!("Must specify --cell, --cell-id, or --range");
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
    notebook::write_notebook_atomic(&args.file, &notebook)
        .context("Failed to write notebook")?;

    // Output result
    let result = DeleteCellResult {
        file: args.file.clone(),
        cells_deleted,
        remaining_cells: notebook.cells.len(),
    };

    output_result(&result, &args.format)?;

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
        let start_i32: i32 = start_str
            .parse()
            .context("Invalid start index in range")?;
        common::normalize_index(start_i32, max_len)?
    };

    // Parse end index (can be one past the end for exclusive range)
    let end = if end_str.is_empty() {
        max_len
    } else {
        let end_i32: i32 = end_str
            .parse()
            .context("Invalid end index in range")?;
        if end_i32 < 0 {
            let abs_idx = end_i32.abs() as usize;
            if abs_idx > max_len {
                bail!("Negative end index {} out of range (notebook has {} cells)", end_i32, max_len);
            }
            max_len - abs_idx
        } else {
            let end_usize = end_i32 as usize;
            if end_usize > max_len {
                bail!("End index {} out of range (notebook has {} cells, max end is {})", end_i32, max_len, max_len);
            }
            end_usize
        }
    };

    // Validate range
    if start >= end {
        bail!("Invalid range: start ({}) must be less than end ({})", start, end);
    }

    Ok((start, end))
}

fn output_result(result: &DeleteCellResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text => {
            println!("Deleted {} cell(s) from: {}", result.cells_deleted, result.file);
            println!("Remaining cells: {}", result.remaining_cells);
        }
    }
    Ok(())
}
