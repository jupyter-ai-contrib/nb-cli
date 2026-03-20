use crate::commands::common::{self, CellType, OutputFormat};
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::{Cell, CellId};
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Parser)]
pub struct AddCellArgs {
    /// Path to notebook file
    pub file: String,

    /// Cell type
    #[arg(
        short = 't',
        long = "type",
        default_value = "code",
        value_name = "TYPE"
    )]
    pub cell_type: CellType,

    /// Cell source content (use '-' for stdin)
    #[arg(short = 's', long = "source", value_name = "TEXT", default_value = "")]
    pub source: String,

    /// Insert at index (supports negative, default: append)
    #[arg(short = 'i', long = "insert-at", value_name = "INDEX", allow_negative_numbers = true, conflicts_with_all = ["after", "before"])]
    pub insert_at: Option<i32>,

    /// Insert after cell with ID
    #[arg(short = 'a', long = "after", value_name = "CELL_ID", conflicts_with_all = ["insert_at", "before"])]
    pub after: Option<String>,

    /// Insert before cell with ID
    #[arg(short = 'b', long = "before", value_name = "CELL_ID", conflicts_with_all = ["insert_at", "after"])]
    pub before: Option<String>,

    /// Custom cell ID (default: auto-generate UUID)
    #[arg(long = "id", value_name = "ID")]
    pub id: Option<String>,

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
struct AddCellResult {
    file: String,
    cell_type: String,
    cell_id: String,
    index: usize,
    total_cells: usize,
}

pub fn execute(args: AddCellArgs) -> Result<()> {
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
    args: AddCellArgs,
    mode: crate::execution::types::ExecutionMode,
) -> Result<()> {
    use crate::execution::remote::ydoc_notebook_ops;

    let (server_url, token) = match mode {
        crate::execution::types::ExecutionMode::Remote { server_url, token } => (server_url, token),
        _ => bail!("Expected remote execution mode"),
    };

    // Extract notebook filename for Y.js connection
    let notebook_filename = std::path::Path::new(&args.file)
        .file_name()
        .and_then(|n| n.to_str())
        .context("Invalid notebook path")?;

    // Read notebook to calculate insertion index and create cell
    let notebook = notebook::read_notebook(&args.file).context("Failed to read notebook")?;

    // Parse source content
    let source = common::parse_source(&args.source)?;

    // Generate or validate cell ID
    let cell_id = if let Some(id) = args.id {
        if notebook.cells.iter().any(|c| c.id().as_str() == id) {
            bail!("Cell ID '{}' already exists in notebook", id);
        }
        CellId::new(&id).map_err(|e| anyhow::anyhow!("Invalid cell ID: {}", e))?
    } else {
        CellId::from(Uuid::new_v4())
    };

    // Create metadata
    let metadata = create_empty_metadata();

    // Create the new cell
    let new_cell = match args.cell_type {
        CellType::Code => Cell::Code {
            id: cell_id.clone(),
            metadata,
            execution_count: None,
            source,
            outputs: vec![],
        },
        CellType::Markdown => Cell::Markdown {
            id: cell_id.clone(),
            metadata,
            source,
            attachments: None,
        },
        CellType::Raw => Cell::Raw {
            id: cell_id.clone(),
            metadata,
            source,
        },
    };

    // Determine insertion index
    let insert_index = if let Some(idx) = args.insert_at {
        if idx < 0 {
            let abs_idx = idx.abs() as usize;
            if abs_idx > notebook.cells.len() {
                bail!(
                    "Negative index {} out of range (notebook has {} cells)",
                    idx,
                    notebook.cells.len()
                );
            }
            notebook.cells.len() - abs_idx
        } else {
            let pos_idx = idx as usize;
            if pos_idx > notebook.cells.len() {
                bail!(
                    "Index {} out of range (notebook has {} cells)",
                    idx,
                    notebook.cells.len()
                );
            }
            pos_idx
        }
    } else if let Some(ref after_id) = args.after {
        let (index, _) = common::find_cell_by_id(&notebook.cells, after_id)?;
        index + 1
    } else if let Some(ref before_id) = args.before {
        let (index, _) = common::find_cell_by_id(&notebook.cells, before_id)?;
        index
    } else {
        notebook.cells.len()
    };

    // Add cell via Y.js (don't write to file - let JupyterLab handle persistence)
    ydoc_notebook_ops::ydoc_add_cell(
        &server_url,
        &token,
        notebook_filename,
        &new_cell,
        insert_index,
    )
    .await
    .context("Error adding cell")?;

    // Output result
    let cell_type_str = match args.cell_type {
        CellType::Code => "code",
        CellType::Markdown => "markdown",
        CellType::Raw => "raw",
    };

    let result = AddCellResult {
        file: args.file.clone(),
        cell_type: cell_type_str.to_string(),
        cell_id: cell_id.to_string(),
        index: insert_index,
        total_cells: notebook.cells.len() + 1,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn execute_file_based(args: AddCellArgs) -> Result<()> {
    // Read notebook
    let mut notebook = notebook::read_notebook(&args.file).context("Failed to read notebook")?;

    // Parse source content (may read from stdin)
    let source = common::parse_source(&args.source)?;

    // Generate or validate cell ID
    let cell_id = if let Some(id) = args.id {
        // Validate that ID is unique
        if notebook.cells.iter().any(|c| c.id().as_str() == id) {
            bail!("Cell ID '{}' already exists in notebook", id);
        }
        CellId::new(&id).map_err(|e| anyhow::anyhow!("Invalid cell ID: {}", e))?
    } else {
        CellId::from(Uuid::new_v4())
    };

    // Create empty metadata
    let metadata = create_empty_metadata();

    // Create the new cell
    let new_cell = match args.cell_type {
        CellType::Code => Cell::Code {
            id: cell_id.clone(),
            metadata,
            execution_count: None,
            source,
            outputs: vec![],
        },
        CellType::Markdown => Cell::Markdown {
            id: cell_id.clone(),
            metadata,
            source,
            attachments: None,
        },
        CellType::Raw => Cell::Raw {
            id: cell_id.clone(),
            metadata,
            source,
        },
    };

    // Determine insertion index
    let insert_index = if let Some(idx) = args.insert_at {
        // Insert at specific index
        if idx < 0 {
            // Negative index: insert from end
            let abs_idx = idx.abs() as usize;
            if abs_idx > notebook.cells.len() {
                bail!(
                    "Negative index {} out of range (notebook has {} cells)",
                    idx,
                    notebook.cells.len()
                );
            }
            notebook.cells.len() - abs_idx
        } else {
            // Positive index: can be len() for append
            let pos_idx = idx as usize;
            if pos_idx > notebook.cells.len() {
                bail!(
                    "Index {} out of range (notebook has {} cells)",
                    idx,
                    notebook.cells.len()
                );
            }
            pos_idx
        }
    } else if let Some(ref after_id) = args.after {
        // Insert after specific cell
        let (index, _) = common::find_cell_by_id(&notebook.cells, after_id)?;
        index + 1
    } else if let Some(ref before_id) = args.before {
        // Insert before specific cell
        let (index, _) = common::find_cell_by_id(&notebook.cells, before_id)?;
        index
    } else {
        // Default: append to end
        notebook.cells.len()
    };

    // Insert the new cell
    notebook.cells.insert(insert_index, new_cell);

    // Write notebook atomically
    notebook::write_notebook_atomic(&args.file, &notebook).context("Failed to write notebook")?;

    // Output result
    let cell_type_str = match args.cell_type {
        CellType::Code => "code",
        CellType::Markdown => "markdown",
        CellType::Raw => "raw",
    };

    let result = AddCellResult {
        file: args.file.clone(),
        cell_type: cell_type_str.to_string(),
        cell_id: cell_id.to_string(),
        index: insert_index,
        total_cells: notebook.cells.len(),
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn create_empty_metadata() -> nbformat::v4::CellMetadata {
    nbformat::v4::CellMetadata {
        id: None,
        collapsed: None,
        scrolled: None,
        deletable: None,
        editable: None,
        format: None,
        name: None,
        tags: None,
        jupyter: None,
        execution: None,
        additional: HashMap::new(),
    }
}

fn output_result(result: &AddCellResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!("Added {} cell to: {}", result.cell_type, result.file);
            println!("Cell ID: {}", result.cell_id);
            println!(
                "Index: {} (total: {} cells)",
                result.index, result.total_cells
            );
        }
    }
    Ok(())
}
