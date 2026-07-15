use crate::commands::common::{self, OutputFormat};
use crate::execution::server::ydoc_notebook_ops::{self, ClearCellSelector};
use crate::notebook::session::{resolve_backend, run_mutation, CellMutator};
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::{Cell, Notebook};
use serde::Serialize;

#[derive(Parser, Clone)]
pub struct ClearOutputsArgs {
    /// Path to notebook file
    pub file: String,

    /// Clear specific cell by ID (stable identifier)
    #[arg(
        short = 'c',
        long = "cell",
        value_name = "ID",
        conflicts_with = "cell_index"
    )]
    pub cell: Option<String>,

    /// Clear specific cell by index (supports negative indexing)
    #[arg(
        short = 'i',
        long = "cell-index",
        value_name = "INDEX",
        allow_negative_numbers = true,
        conflicts_with = "cell"
    )]
    pub cell_index: Option<i32>,

    /// Preserve execution_count (default: clear it too)
    #[arg(long = "keep-execution-count")]
    pub keep_execution_count: bool,

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
struct ClearOutputsResult {
    file: String,
    cells_cleared: usize,
    execution_counts_cleared: bool,
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

/// Clear outputs from the cell(s) selected by `--cell`/`--cell-index`, or all
/// code cells when neither is given. Returns the number of cells cleared.
fn clear_selected_outputs(args: &ClearOutputsArgs, notebook: &mut Notebook) -> Result<usize> {
    if let Some(ref cell_id) = args.cell {
        let (_, cell) = common::find_cell_by_id_mut(&mut notebook.cells, cell_id)?;
        clear_cell_output(cell, args.keep_execution_count)?;
        Ok(1)
    } else if let Some(cell_index) = args.cell_index {
        let index = common::normalize_index(cell_index, notebook.cells.len())?;
        clear_cell_output(&mut notebook.cells[index], args.keep_execution_count)?;
        Ok(1)
    } else {
        let mut count = 0;
        for cell in &mut notebook.cells {
            if let Cell::Code { .. } = cell {
                clear_cell_output(cell, args.keep_execution_count)?;
                count += 1;
            }
        }
        Ok(count)
    }
}

struct ClearOutputsMutator<'a> {
    args: &'a ClearOutputsArgs,
}

#[async_trait::async_trait]
impl CellMutator for ClearOutputsMutator<'_> {
    /// (cells_cleared, execution_counts_cleared)
    type Output = (usize, bool);

    fn mutate_notebook(&self, notebook: &mut Notebook, _file_path: &str) -> Result<Self::Output> {
        let cells_cleared = clear_selected_outputs(self.args, notebook)?;
        Ok((cells_cleared, !self.args.keep_execution_count))
    }

    async fn mutate_realtime(
        &self,
        server_url: &str,
        token: &str,
        server_path: &str,
        _file_path: &str,
    ) -> Result<Self::Output> {
        let selector = if let Some(ref cell_id) = self.args.cell {
            ClearCellSelector::ById(cell_id.clone())
        } else if let Some(cell_index) = self.args.cell_index {
            ClearCellSelector::ByIndex(cell_index)
        } else {
            ClearCellSelector::All
        };

        let cells_cleared =
            ydoc_notebook_ops::ydoc_clear_outputs(server_url, token, server_path, selector)
                .await
                .context("Error clearing outputs")?;
        // ydoc_clear_outputs always clears execution_count regardless of
        // --keep-execution-count (the Y.js path has no way to preserve it
        // today) — report that faithfully rather than the requested flag.
        Ok((cells_cleared, true))
    }
}

pub fn execute(args: ClearOutputsArgs) -> Result<()> {
    let backend = resolve_backend(&args.file, args.server.clone(), args.token.clone())?;
    let mutator = ClearOutputsMutator { args: &args };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (file_path, (cells_cleared, execution_counts_cleared)) =
        runtime.block_on(run_mutation(backend, &mutator))?;

    let result = ClearOutputsResult {
        file: file_path,
        cells_cleared,
        execution_counts_cleared,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)
}

fn output_result(result: &ClearOutputsResult, format: &OutputFormat) -> Result<()> {
    common::print_result(result, format, |result| {
        println!(
            "Cleared outputs from {} cell(s) in: {}",
            result.cells_cleared, result.file
        );
        if result.execution_counts_cleared {
            println!("Execution counts were also cleared");
        } else {
            println!("Execution counts were preserved");
        }
    })
}
