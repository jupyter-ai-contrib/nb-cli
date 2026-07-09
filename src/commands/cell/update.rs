use crate::commands::common::{self, CellType, OutputFormat};
use crate::execution::server::ydoc_notebook_ops;
use crate::notebook::session::{resolve_backend, run_mutation, CellMutator};
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::{Cell, Notebook};
use serde::Serialize;

#[derive(Parser, Clone)]
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

/// Resolve the target cell from `--cell`/`--cell-index` (already validated
/// to have exactly one set), returning its index and stable ID.
fn resolve_target(args: &UpdateCellArgs, cells: &[Cell]) -> Result<(usize, String)> {
    if let Some(ref id) = args.cell {
        let (idx, cell) = common::find_cell_by_id(cells, id)?;
        Ok((idx, cell.id().to_string()))
    } else if let Some(cell_index) = args.cell_index {
        let idx = common::normalize_index(cell_index, cells.len())?;
        let id = cells[idx].id().to_string();
        Ok((idx, id))
    } else {
        unreachable!("Already validated cell selector");
    }
}

/// Apply `--source`/`--append`/`--type` to the cell at `index`, returning a
/// human-readable list of what changed.
fn apply_updates(
    notebook: &mut Notebook,
    index: usize,
    args: &UpdateCellArgs,
) -> Result<Vec<String>> {
    let mut updates = Vec::new();
    let cell = &mut notebook.cells[index];

    if let Some(ref source_text) = args.source {
        let new_source = common::parse_source(source_text)?;
        match cell {
            Cell::Code {
                source,
                execution_count,
                ..
            } => {
                *source = new_source;
                *execution_count = None;
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

    if let Some(ref append_text) = args.append {
        let append_source = common::parse_source(append_text)?;
        match cell {
            Cell::Code {
                source,
                execution_count,
                ..
            } => {
                source.extend(append_source);
                *execution_count = None;
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

    if let Some(new_type) = args.cell_type.clone() {
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
        let type_name = common::cell_type_enum_str(&new_type);
        updates.push(format!("type changed to {}", type_name));
    }

    Ok(updates)
}

struct UpdateCellMutator<'a> {
    args: &'a UpdateCellArgs,
}

#[async_trait::async_trait]
impl CellMutator for UpdateCellMutator<'_> {
    /// (cell_id, index, updated)
    type Output = (String, usize, Vec<String>);

    fn mutate_notebook(&self, notebook: &mut Notebook, _file_path: &str) -> Result<Self::Output> {
        let (index, cell_id) = resolve_target(self.args, &notebook.cells)?;
        let updates = apply_updates(notebook, index, self.args)?;
        Ok((cell_id, index, updates))
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

        let (index, cell_id) = resolve_target(self.args, &notebook.cells)?;

        let mut updates = Vec::new();

        let new_source = if let Some(ref source_text) = self.args.source {
            let parsed = common::parse_source(source_text)?;
            updates.push("source replaced".to_string());
            Some(parsed.join(""))
        } else {
            None
        };

        let append_source = if let Some(ref append_text) = self.args.append {
            let parsed = common::parse_source(append_text)?;
            updates.push("source appended".to_string());
            Some(parsed.join(""))
        } else {
            None
        };

        // Note: the realtime path does not support --type changes today
        // (ydoc_update_cell has no cell-type-change operation) — this
        // matches the pre-existing behavior of the code this replaces.

        ydoc_notebook_ops::ydoc_update_cell(
            server_url,
            token,
            server_path,
            index,
            new_source.as_deref(),
            append_source.as_deref(),
        )
        .await
        .context("Error updating cell")?;

        Ok((cell_id, index, updates))
    }
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

    let backend = resolve_backend(&args.file, args.server.clone(), args.token.clone())?;
    let mutator = UpdateCellMutator { args: &args };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (file_path, (cell_id, index, updated)) =
        runtime.block_on(run_mutation(backend, &mutator))?;

    let result = UpdateCellResult {
        file: file_path,
        cell_id,
        index,
        updated,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)
}

fn output_result(result: &UpdateCellResult, format: &OutputFormat) -> Result<()> {
    common::print_result(result, format, |result| {
        println!("Updated cell at index {}: {}", result.index, result.file);
        println!("Cell ID: {}", result.cell_id);
        println!("Changes: {}", result.updated.join(", "));
    })
}
