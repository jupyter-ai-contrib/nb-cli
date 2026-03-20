use crate::commands::common::{self, CellType, OutputFormat};
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::Cell;
use serde::Serialize;

#[derive(Parser)]
pub struct UpdateCellArgs {
    /// Path to notebook file
    pub file: String,

    /// Cell ID (stable identifier)
    #[arg(
        short = 'c',
        long = "cell",
        value_name = "ID",
        conflicts_with = "cell_index"
    )]
    pub cell: Option<String>,

    /// Cell index (supports negative indexing)
    #[arg(
        short = 'i',
        long = "cell-index",
        value_name = "INDEX",
        allow_negative_numbers = true,
        conflicts_with = "cell"
    )]
    pub cell_index: Option<i32>,

    /// New source content (use '-' for stdin)
    #[arg(
        short = 's',
        long = "source",
        value_name = "TEXT",
        conflicts_with = "append"
    )]
    pub source: Option<String>,

    /// Append to existing source (conflicts with --source)
    #[arg(
        short = 'a',
        long = "append",
        value_name = "TEXT",
        conflicts_with = "source"
    )]
    pub append: Option<String>,

    /// Change cell type
    #[arg(short = 't', long = "type", value_name = "TYPE")]
    pub cell_type: Option<CellType>,

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
struct UpdateCellResult {
    file: String,
    cell_id: String,
    index: usize,
    updated: Vec<String>,
}

pub fn execute(args: UpdateCellArgs) -> Result<()> {
    // Validate that at least one modification is specified
    if args.source.is_none() && args.append.is_none() && args.cell_type.is_none() {
        bail!("Must specify at least one of: --source, --append, or --type");
    }

    // Validate that cell selector is specified
    if args.cell.is_none() && args.cell_index.is_none() {
        bail!("Must specify --cell or --cell-index");
    }

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
    args: UpdateCellArgs,
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

    // Read notebook to find the cell
    let notebook = notebook::read_notebook(&args.file).context("Failed to read notebook")?;

    // Find the target cell
    let (index, cell_id) = if let Some(ref id) = args.cell {
        let (idx, cell) = common::find_cell_by_id(&notebook.cells, id)?;
        (idx, cell.id().to_string())
    } else if let Some(cell_index) = args.cell_index {
        let idx = common::normalize_index(cell_index, notebook.cells.len())?;
        let id = notebook.cells[idx].id().to_string();
        (idx, id)
    } else {
        unreachable!("Already validated cell selector");
    };

    let mut updates = Vec::new();

    // Determine what to update
    let new_source = if let Some(ref source_text) = args.source {
        let parsed = common::parse_source(source_text)?;
        updates.push("source replaced".to_string());
        Some(parsed.join(""))
    } else {
        None
    };

    let append_source = if let Some(ref append_text) = args.append {
        let parsed = common::parse_source(append_text)?;
        updates.push("source appended".to_string());
        Some(parsed.join(""))
    } else {
        None
    };

    // Update via Y.js
    ydoc_notebook_ops::ydoc_update_cell(
        &server_url,
        &token,
        notebook_filename,
        index,
        new_source.as_deref(),
        append_source.as_deref(),
    )
    .await
    .context("Error updating cell")?;

    // Output result
    let result = UpdateCellResult {
        file: args.file.clone(),
        cell_id,
        index,
        updated: updates,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn execute_file_based(args: UpdateCellArgs) -> Result<()> {
    // Read notebook
    let mut notebook = notebook::read_notebook(&args.file).context("Failed to read notebook")?;

    // Find the target cell
    let (index, cell_id) = if let Some(ref id) = args.cell {
        let (idx, cell) = common::find_cell_by_id(&notebook.cells, id)?;
        (idx, cell.id().to_string())
    } else if let Some(cell_index) = args.cell_index {
        let idx = common::normalize_index(cell_index, notebook.cells.len())?;
        let id = notebook.cells[idx].id().to_string();
        (idx, id)
    } else {
        unreachable!("Already validated cell selector");
    };

    let mut updates = Vec::new();

    // Apply modifications
    let cell = &mut notebook.cells[index];

    // Update source if specified
    if let Some(ref source_text) = args.source {
        let new_source = common::parse_source(source_text)?;
        match cell {
            Cell::Code {
                source,
                execution_count,
                ..
            } => {
                *source = new_source;
                *execution_count = None; // Reset execution count when modifying source
                updates.push("source replaced".to_string());
            }
            Cell::Markdown { source, .. } => {
                *source = new_source;
                updates.push("source replaced".to_string());
            }
            Cell::Raw { source, .. } => {
                *source = new_source;
                updates.push("source replaced".to_string());
            }
        }
    }

    // Append to source if specified
    if let Some(ref append_text) = args.append {
        let append_source = common::parse_source(append_text)?;
        match cell {
            Cell::Code {
                source,
                execution_count,
                ..
            } => {
                source.extend(append_source);
                *execution_count = None; // Reset execution count when modifying source
                updates.push("source appended".to_string());
            }
            Cell::Markdown { source, .. } => {
                source.extend(append_source);
                updates.push("source appended".to_string());
            }
            Cell::Raw { source, .. } => {
                source.extend(append_source);
                updates.push("source appended".to_string());
            }
        }
    }

    // Change cell type if specified
    if let Some(new_type) = args.cell_type {
        let old_cell = notebook.cells.remove(index);
        let (old_id, old_metadata, old_source) = match old_cell {
            Cell::Code {
                id,
                metadata,
                source,
                ..
            } => (id, metadata, source),
            Cell::Markdown {
                id,
                metadata,
                source,
                ..
            } => (id, metadata, source),
            Cell::Raw {
                id,
                metadata,
                source,
            } => (id, metadata, source),
        };

        let new_cell = match new_type {
            CellType::Code => Cell::Code {
                id: old_id,
                metadata: old_metadata,
                execution_count: None,
                source: old_source,
                outputs: vec![],
            },
            CellType::Markdown => Cell::Markdown {
                id: old_id,
                metadata: old_metadata,
                source: old_source,
                attachments: None,
            },
            CellType::Raw => Cell::Raw {
                id: old_id,
                metadata: old_metadata,
                source: old_source,
            },
        };

        notebook.cells.insert(index, new_cell);
        let type_name = match new_type {
            CellType::Code => "code",
            CellType::Markdown => "markdown",
            CellType::Raw => "raw",
        };
        updates.push(format!("type changed to {}", type_name));
    }

    // Write notebook atomically
    notebook::write_notebook_atomic(&args.file, &notebook).context("Failed to write notebook")?;

    // Output result
    let result = UpdateCellResult {
        file: args.file.clone(),
        cell_id,
        index,
        updated: updates,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn output_result(result: &UpdateCellResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!("Updated cell at index {}: {}", result.index, result.file);
            println!("Cell ID: {}", result.cell_id);
            println!("Changes: {}", result.updated.join(", "));
        }
    }
    Ok(())
}
