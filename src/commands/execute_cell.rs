use crate::execution::{create_backend, types::ExecutionConfig, types::ExecutionMode};
use crate::notebook::{read_notebook, write_notebook_atomic};
use anyhow::{bail, Context, Result};
use clap::Args;
use nbformat::v4::Cell;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Args)]
pub struct ExecuteCellArgs {
    /// Path to the notebook file
    pub file: String,

    /// Cell index to execute (supports negative indexing like -1 for last cell)
    #[arg(short, long, group = "cell_selector", allow_negative_numbers = true)]
    pub cell: Option<i32>,

    /// Cell ID to execute
    #[arg(long, group = "cell_selector")]
    pub cell_id: Option<String>,

    /// Kernel to use (overrides notebook metadata)
    #[arg(short, long)]
    pub kernel: Option<String>,

    /// Timeout in seconds (default: 30)
    #[arg(short, long, default_value = "30")]
    pub timeout: u64,

    /// Continue despite errors
    #[arg(long)]
    pub allow_errors: bool,

    /// Don't update the notebook file (dry run)
    #[arg(long)]
    pub dry_run: bool,

    /// Remote server URL (enables remote mode)
    #[arg(long)]
    pub server: Option<String>,

    /// Authentication token for remote server
    #[arg(long)]
    pub token: Option<String>,

    /// Output format: json or text
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Json,
    Text,
}

impl std::str::FromStr for OutputFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "json" => Ok(OutputFormat::Json),
            "text" => Ok(OutputFormat::Text),
            _ => bail!("Invalid format: '{}'. Must be 'json' or 'text'", s),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct ExecuteCellResult {
    success: bool,
    cell_index: usize,
    cell_id: String,
    execution_count: Option<i64>,
    outputs_count: usize,
}

pub fn execute(args: ExecuteCellArgs) -> Result<()> {
    // Create Tokio runtime for async execution
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(execute_async(args))
}

async fn execute_async(args: ExecuteCellArgs) -> Result<()> {
    // Read notebook
    let mut notebook = read_notebook(&args.file).context("Failed to read notebook")?;

    // Find cell by index or ID
    let (cell_index, cell_source) = if let Some(index) = args.cell {
        let idx = crate::commands::common::normalize_index(index, notebook.cells.len())?;
        let cell = &notebook.cells[idx];
        let source = crate::commands::common::cell_to_string(cell);
        (idx, source)
    } else if let Some(ref cell_id) = args.cell_id {
        let (idx, cell) = crate::commands::common::find_cell_by_id(&notebook.cells, cell_id)?;
        let source = crate::commands::common::cell_to_string(cell);
        (idx, source)
    } else {
        bail!("Must specify --cell INDEX or --cell-id ID");
    };

    // Verify it's a code cell and get cell ID
    let (is_code_cell, cell_id) = {
        let cell = &notebook.cells[cell_index];
        (
            matches!(cell, Cell::Code { .. }),
            crate::commands::common::cell_id_to_string(cell),
        )
    };

    if !is_code_cell {
        bail!("Cell at index {} is not a code cell", cell_index);
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
        std::fs::canonicalize(&args.file).context("Failed to resolve notebook path")?;
    let notebook_path_str = notebook_path_abs
        .to_str()
        .context("Notebook path contains invalid UTF-8")?
        .to_string();

    // For remote mode, extract just the filename for session matching
    let notebook_identifier =
        if matches!(mode, crate::execution::types::ExecutionMode::Remote { .. }) {
            std::path::Path::new(&args.file)
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or(notebook_path_str.clone())
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
        notebook_path: Some(notebook_identifier),
    };

    // Create and start backend
    let mut backend = create_backend(config)?;
    backend
        .start()
        .await
        .context("Failed to start execution backend")?;

    // Execute cell
    let result = backend
        .execute_code(&cell_source, Some(&cell_id))
        .await
        .context("Failed to execute cell")?;

    // Stop backend
    backend.stop().await?;

    // Update notebook with outputs
    if !args.dry_run {
        // Update in-memory notebook
        if let Cell::Code {
            ref mut outputs,
            ref mut execution_count,
            ..
        } = notebook.cells[cell_index]
        {
            *outputs = result.outputs.clone();
            *execution_count = result.execution_count.map(|c| c as i32);
        }

        // Persist changes based on mode
        if matches!(mode, ExecutionMode::Local) {
            // Only write to file in local mode
            write_notebook_atomic(&args.file, &notebook).context("Failed to write notebook")?;
        }
        // In remote mode, the server manages the notebook state
    }

    // Output result
    let output_result = ExecuteCellResult {
        success: result.success,
        cell_index,
        cell_id,
        execution_count: result.execution_count,
        outputs_count: result.outputs.len(),
    };

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&output_result)?);
        }
        OutputFormat::Text => {
            if result.success {
                println!("✓ Cell executed successfully");
            } else {
                println!("✗ Cell execution failed");
                if let Some(error) = &result.error {
                    eprintln!("\nError: {}: {}", error.ename, error.evalue);
                    if !error.traceback.is_empty() {
                        eprintln!("\nTraceback:");
                        for line in &error.traceback {
                            eprintln!("  {}", line);
                        }
                    }
                }
            }
            println!("Cell index: {}", cell_index);
            println!("Execution count: {:?}", result.execution_count);
            println!("Outputs: {}", result.outputs.len());

            if args.dry_run {
                println!("\n(Dry run - notebook not updated)");
            } else if matches!(mode, ExecutionMode::Local) {
                println!("\nNotebook updated: {}", args.file);
            } else {
                println!("\n(Executed via Jupyter Server)");
            }
        }
    }

    if !result.success && !args.allow_errors {
        std::process::exit(1);
    }

    Ok(())
}
