use crate::commands::common::{self, OutputFormat};
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::Cell;
use serde::Serialize;

#[derive(Parser)]
pub struct ClearOutputsArgs {
    /// Path to notebook file
    pub file: String,

    /// Clear specific cell by ID (stable identifier)
    #[arg(short = 'c', long = "cell", value_name = "ID", conflicts_with_all = ["cell_index", "all"])]
    pub cell: Option<String>,

    /// Clear specific cell by index (supports negative indexing)
    #[arg(short = 'i', long = "cell-index", value_name = "INDEX", allow_negative_numbers = true, conflicts_with_all = ["cell", "all"])]
    pub cell_index: Option<i32>,

    /// Clear all code cell outputs (default if no options)
    #[arg(short = 'a', long = "all", conflicts_with_all = ["cell", "cell_index"])]
    pub all: bool,

    /// Preserve execution_count (default: clear it too)
    #[arg(long = "keep-execution-count")]
    pub keep_execution_count: bool,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct ClearOutputsResult {
    file: String,
    cells_cleared: usize,
    execution_counts_cleared: bool,
}

pub fn execute(args: ClearOutputsArgs) -> Result<()> {
    // Read notebook
    let mut notebook = notebook::read_notebook(&args.file).context("Failed to read notebook")?;

    let cells_cleared = if let Some(ref cell_id) = args.cell {
        // Clear specific cell by ID
        let (_, cell) = common::find_cell_by_id_mut(&mut notebook.cells, cell_id)?;
        clear_cell_output(cell, args.keep_execution_count)?;
        1
    } else if let Some(cell_index) = args.cell_index {
        // Clear specific cell by index
        let index = common::normalize_index(cell_index, notebook.cells.len())?;
        clear_cell_output(&mut notebook.cells[index], args.keep_execution_count)?;
        1
    } else {
        // Clear all code cell outputs (default behavior)
        let mut count = 0;
        for cell in &mut notebook.cells {
            if let Cell::Code { .. } = cell {
                clear_cell_output(cell, args.keep_execution_count)?;
                count += 1;
            }
        }
        count
    };

    // Write notebook atomically
    notebook::write_notebook_atomic(&args.file, &notebook).context("Failed to write notebook")?;

    // Output result
    let result = ClearOutputsResult {
        file: args.file.clone(),
        cells_cleared,
        execution_counts_cleared: !args.keep_execution_count,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn clear_cell_output(cell: &mut Cell, keep_execution_count: bool) -> Result<()> {
    match cell {
        Cell::Code {
            outputs,
            execution_count,
            ..
        } => {
            outputs.clear();
            if !keep_execution_count {
                *execution_count = None;
            }
            Ok(())
        }
        _ => bail!("Can only clear outputs from code cells"),
    }
}

fn output_result(result: &ClearOutputsResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!(
                "Cleared outputs from {} cell(s) in: {}",
                result.cells_cleared, result.file
            );
            if result.execution_counts_cleared {
                println!("Execution counts were also cleared");
            } else {
                println!("Execution counts were preserved");
            }
        }
    }
    Ok(())
}
