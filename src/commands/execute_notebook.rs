use crate::commands::common::{self, OutputFormat};
use crate::commands::markdown_renderer;
use crate::execution::{create_backend, types::ExecutionConfig, types::ExecutionMode};
use crate::notebook::{read_notebook, write_notebook_atomic};
use anyhow::{Context, Result};
use clap::Args;
use nbformat::v4::Cell;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Args)]
pub struct ExecuteNotebookArgs {
    /// Path to the notebook file
    pub file: String,

    /// Kernel to use (overrides notebook metadata)
    #[arg(short, long)]
    pub kernel: Option<String>,

    /// Timeout in seconds per cell (default: 30)
    #[arg(short, long, default_value = "30")]
    pub timeout: u64,

    /// Continue despite errors
    #[arg(long)]
    pub allow_errors: bool,

    /// Execute specific cell by ID (stable identifier)
    #[arg(short = 'c', long, conflicts_with_all = ["cell_index", "start", "end"])]
    pub cell: Option<String>,

    /// Execute specific cell by index (supports negative indexing)
    #[arg(short = 'i', long = "cell-index", allow_negative_numbers = true, conflicts_with_all = ["cell", "start", "end"])]
    pub cell_index: Option<i32>,

    /// Start cell index (inclusive)
    #[arg(long, allow_negative_numbers = true, conflicts_with_all = ["cell", "cell_index"])]
    pub start: Option<i32>,

    /// End cell index (inclusive)
    #[arg(long, allow_negative_numbers = true, conflicts_with_all = ["cell", "cell_index"])]
    pub end: Option<i32>,

    /// Remote server URL (enables remote mode)
    #[arg(long)]
    pub server: Option<String>,

    /// Authentication token for remote server
    #[arg(long)]
    pub token: Option<String>,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,

    /// Use uv to discover kernels in local mode
    #[arg(long, conflicts_with = "pixi")]
    pub uv: bool,

    /// Use pixi to discover kernels in local mode
    #[arg(long, conflicts_with = "uv")]
    pub pixi: bool,
}

#[derive(Serialize)]
pub struct ExecuteNotebookResult {
    success: bool,
    total_cells: usize,
    executed_cells: usize,
    failed_cells: usize,
    cells: Vec<serde_json::Value>,
}

pub fn execute(args: ExecuteNotebookArgs) -> Result<()> {
    // Create Tokio runtime for async execution
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(execute_async(args))
}

async fn execute_async(args: ExecuteNotebookArgs) -> Result<()> {
    use crate::commands::env_manager::EnvConfig;

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    // Create environment configuration for local mode
    let env_config = if args.uv || args.pixi {
        Some(EnvConfig::from_flags(args.uv, args.pixi)?)
    } else {
        None
    };

    // Read notebook
    let file_path = common::normalize_notebook_path(&args.file);
    let mut notebook = read_notebook(&file_path).context("Failed to read notebook")?;

    // Determine cell range
    let (start_idx, end_idx) = if let Some(ref cell_id) = args.cell {
        // Execute specific cell by ID
        let (idx, _) = crate::commands::common::find_cell_by_id(&notebook.cells, cell_id)?;
        (idx, idx)
    } else if let Some(cell_index) = args.cell_index {
        // Execute specific cell by index
        let idx = crate::commands::common::normalize_index(cell_index, notebook.cells.len())?;
        (idx, idx)
    } else {
        // Execute range or all cells
        let start = if let Some(start) = args.start {
            crate::commands::common::normalize_index(start, notebook.cells.len())?
        } else {
            0
        };

        let end = if let Some(end) = args.end {
            crate::commands::common::normalize_index(end, notebook.cells.len())?
        } else {
            notebook.cells.len().saturating_sub(1)
        };

        (start, end)
    };

    if start_idx > end_idx {
        anyhow::bail!(
            "Start index {} is greater than end index {}",
            start_idx,
            end_idx
        );
    }

    // Determine execution mode
    let mode =
        crate::commands::common::resolve_execution_mode(args.server.clone(), args.token.clone())?;

    // Get kernel from notebook metadata if not specified
    let notebook_kernel = notebook
        .metadata
        .kernelspec
        .as_ref()
        .map(|ks| ks.name.as_str());

    // Get absolute path to notebook for working directory determination
    let notebook_path_abs =
        std::fs::canonicalize(&file_path).context("Failed to resolve notebook path")?;
    let notebook_path_str = notebook_path_abs
        .to_str()
        .context("Notebook path contains invalid UTF-8")?
        .to_string();

    // For remote mode, compute path relative to server root so that
    // notebooks with the same name in different directories get distinct sessions.
    let notebook_identifier =
        if matches!(mode, crate::execution::types::ExecutionMode::Remote { .. }) {
            let server_root = common::resolve_server_root();
            common::notebook_path_for_server(&file_path, server_root.as_deref())
        } else {
            // For local mode, use full absolute path
            notebook_path_str.clone()
        };

    // Create execution config
    let config = ExecutionConfig {
        mode: mode.clone(),
        timeout: Duration::from_secs(args.timeout),
        kernel_name: args.kernel.or_else(|| notebook_kernel.map(String::from)),
        allow_errors: args.allow_errors,
        notebook_path: Some(notebook_identifier.clone()),
        env_config: env_config.clone(),
    };

    // Create and start backend (reuse kernel for all cells)
    let mut backend = create_backend(config)?;
    backend.start().await?;

    // Execute cells in range and collect results
    let mut executed_count = 0;
    let mut failed_count = 0;
    let _total_cells = notebook.cells.len();
    let mut execution_results: HashMap<usize, crate::execution::types::ExecutionResult> =
        HashMap::new();

    let mut code_cell_num = 0;

    for (i, cell) in notebook.cells.iter().enumerate() {
        // Skip cells outside range
        if i < start_idx || i > end_idx {
            continue;
        }

        // Skip non-code cells
        if !matches!(cell, Cell::Code { .. }) {
            continue;
        }

        code_cell_num += 1;

        // Get cell source and cell_id
        let source = crate::commands::common::cell_to_string(cell);
        let cell_id = crate::commands::common::cell_id_to_string(cell);

        // Execute cell
        match backend.execute_code(&source, Some(&cell_id)).await {
            Ok(result) => {
                let success = result.success;

                // Store result for later processing
                execution_results.insert(i, result);
                executed_count += 1;

                if !success {
                    failed_count += 1;

                    if !matches!(format, OutputFormat::Json) {
                        eprintln!("  ✗ Cell {} completed with error", code_cell_num);
                        if let Some(error) =
                            execution_results.get(&i).and_then(|r| r.error.as_ref())
                        {
                            eprintln!("    Error: {}: {}", error.ename, error.evalue);
                        }
                    }

                    // Stop on error unless --allow-errors
                    if !args.allow_errors {
                        break;
                    }
                } else if !matches!(format, OutputFormat::Json) {
                    eprintln!("  ✓ Cell {} completed", code_cell_num);
                }
            }
            Err(e) => {
                backend.stop().await?;
                return Err(e).context(format!("Failed to execute cell {}", i));
            }
        }
    }

    // Stop backend (just closes WebSocket, session persists)
    backend.stop().await?;

    // Update notebook cells with execution results
    for (i, result) in &execution_results {
        if let Cell::Code {
            ref mut outputs,
            ref mut execution_count,
            ..
        } = notebook.cells[*i]
        {
            *outputs = result.outputs.clone();
            *execution_count = result.execution_count.map(|c| c as i32);
        }
    }

    // Persist changes based on mode
    match mode {
        ExecutionMode::Local => {
            // Write notebook to file
            write_notebook_atomic(&file_path, &notebook).context("Failed to write notebook")?;
        }
        ExecutionMode::Remote { .. } => {
            // In remote mode, Jupyter Server automatically updates the Y.js document
            // when it receives kernel execution messages. The outputs are already
            // processed by jupyter-server-documents and will be auto-saved to disk
            // within ~1 second (configured save_delay). No need to wait as nb-cli
            // already has all outputs from the kernel execution.
        }
    }

    // Compute result summary
    let success = failed_count == 0;
    let total_cells = end_idx - start_idx + 1;

    // Output summary to stderr (diagnostics)
    eprintln!("\n{}", "=".repeat(50));
    if success {
        eprintln!("✓ Notebook executed successfully");
    } else {
        eprintln!("✗ Notebook execution completed with errors");
    }
    eprintln!("Total cells in range: {}", total_cells);
    eprintln!("Executed: {}", executed_count);
    eprintln!("Failed: {}", failed_count);

    if matches!(mode, ExecutionMode::Local) {
        eprintln!("\nNotebook updated: {}", file_path);
    } else {
        eprintln!("\n(Executed via Jupyter Server)");
    }

    // Output notebook content to stdout (data)
    match format {
        OutputFormat::Json => {
            let cells = common::serialize_cells_json(&notebook.cells, true);
            let output_result = ExecuteNotebookResult {
                success,
                total_cells,
                executed_cells: executed_count,
                failed_cells: failed_count,
                cells,
            };
            println!("{}", serde_json::to_string_pretty(&output_result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            let output_dir = markdown_renderer::notebook_output_dir(&file_path);
            std::fs::create_dir_all(&output_dir)?;
            let markdown = markdown_renderer::render_notebook_markdown(
                &notebook,
                true,
                Some(&output_dir),
                common::DEFAULT_INLINE_LIMIT,
            )?;
            print!("{}", markdown);
        }
    }

    if !success {
        std::process::exit(1);
    }

    Ok(())
}
