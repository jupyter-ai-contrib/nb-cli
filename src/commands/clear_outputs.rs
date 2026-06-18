use crate::commands::common::{self, OutputFormat};
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::Cell;
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

pub fn execute(args: ClearOutputsArgs) -> Result<()> {
    use crate::execution::types::ExecutionMode;
    let mode = common::resolve_execution_mode(args.server.clone(), args.token.clone())?;

    match &mode {
        ExecutionMode::Local => execute_file_based(args),
        ExecutionMode::Remote { .. } => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(execute_remote(args, mode))
        }
    }
}

/// Remote dispatch: cached Some(false) goes straight to the Contents API;
/// otherwise try the Y.js realtime path and fall back on the definitive
/// backend-absent signal. Transient errors propagate so a flaky
/// collaboration server is never silently downgraded.
async fn execute_remote(
    args: ClearOutputsArgs,
    mode: crate::execution::types::ExecutionMode,
) -> Result<()> {
    let cached = common::resolve_ydoc_available(&args.server, &args.token);
    if cached == Some(false) {
        return execute_with_contents_api(args, mode).await;
    }
    match execute_with_realtime(args.clone(), mode.clone()).await {
        Err(e) if crate::execution::remote::ydoc::is_yjs_unavailable(&e) => {
            common::warn_stale_collab_cache(cached);
            execute_with_contents_api(args, mode).await
        }
        result => result,
    }
}

async fn execute_with_realtime(
    args: ClearOutputsArgs,
    mode: crate::execution::types::ExecutionMode,
) -> Result<()> {
    use crate::execution::remote::ydoc_notebook_ops::{self, ClearCellSelector};

    let (server_url, token) = match &mode {
        crate::execution::types::ExecutionMode::Remote { server_url, token } => {
            (server_url.clone(), token.clone())
        }
        _ => bail!("Expected remote execution mode"),
    };

    let file_path = common::normalize_notebook_path(&args.file);
    let server_root = common::resolve_server_root();
    let notebook_server_path = common::notebook_path_for_server(&file_path, server_root.as_deref());

    let selector = if let Some(ref cell_id) = args.cell {
        ClearCellSelector::ById(cell_id.clone())
    } else if let Some(cell_index) = args.cell_index {
        ClearCellSelector::ByIndex(cell_index)
    } else {
        ClearCellSelector::All
    };

    let cells_cleared =
        ydoc_notebook_ops::ydoc_clear_outputs(&server_url, &token, &notebook_server_path, selector)
            .await
            .context("Error clearing outputs")?;

    let result = ClearOutputsResult {
        file: file_path,
        cells_cleared,
        execution_counts_cleared: true,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

async fn execute_with_contents_api(
    args: ClearOutputsArgs,
    mode: crate::execution::types::ExecutionMode,
) -> Result<()> {
    let file = args.file.clone();
    let result = common::with_contents_api(&file, &mode, |notebook, file_path| {
        let cells_cleared = if let Some(ref cell_id) = args.cell {
            let (_, cell) = common::find_cell_by_id_mut(&mut notebook.cells, cell_id)?;
            clear_cell_output(cell, args.keep_execution_count)?;
            1
        } else if let Some(cell_index) = args.cell_index {
            let index = common::normalize_index(cell_index, notebook.cells.len())?;
            clear_cell_output(&mut notebook.cells[index], args.keep_execution_count)?;
            1
        } else {
            let mut count = 0;
            for cell in &mut notebook.cells {
                if let Cell::Code { .. } = cell {
                    clear_cell_output(cell, args.keep_execution_count)?;
                    count += 1;
                }
            }
            count
        };

        Ok(ClearOutputsResult {
            file: file_path.to_string(),
            cells_cleared,
            execution_counts_cleared: !args.keep_execution_count,
        })
    })
    .await?;

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)
}

fn execute_file_based(args: ClearOutputsArgs) -> Result<()> {
    let file_path = common::normalize_notebook_path(&args.file);
    let mut notebook = notebook::read_notebook(&file_path).context("Failed to read notebook")?;

    let cells_cleared = if let Some(ref cell_id) = args.cell {
        let (_, cell) = common::find_cell_by_id_mut(&mut notebook.cells, cell_id)?;
        clear_cell_output(cell, args.keep_execution_count)?;
        1
    } else if let Some(cell_index) = args.cell_index {
        let index = common::normalize_index(cell_index, notebook.cells.len())?;
        clear_cell_output(&mut notebook.cells[index], args.keep_execution_count)?;
        1
    } else {
        let mut count = 0;
        for cell in &mut notebook.cells {
            if let Cell::Code { .. } = cell {
                clear_cell_output(cell, args.keep_execution_count)?;
                count += 1;
            }
        }
        count
    };

    notebook::write_notebook_atomic(&file_path, &notebook).context("Failed to write notebook")?;

    let result = ClearOutputsResult {
        file: file_path,
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
