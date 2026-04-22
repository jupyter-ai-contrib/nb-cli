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

    /// Cell type (used when source has no @@code/@@markdown/@@raw sentinels)
    #[arg(
        short = 't',
        long = "type",
        default_value = "code",
        value_name = "TYPE"
    )]
    pub cell_type: CellType,

    /// Cell source content (use '-' for stdin). Use @@code, @@markdown, @@raw
    /// sentinels on their own line to add multiple cells in one call.
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

#[derive(Serialize)]
struct AddCellsResult {
    file: String,
    cells_added: usize,
    total_cells: usize,
    cells: Vec<AddedCellInfo>,
}

#[derive(Serialize)]
struct AddedCellInfo {
    cell_type: String,
    cell_id: String,
    index: usize,
}

/// A cell parsed from sentinel-delimited source input.
struct ParsedCell {
    cell_type: CellType,
    source: Vec<String>,
}

/// Try to parse the source text as sentinel-delimited multi-cell input.
///
/// Recognizes `@@code`, `@@markdown`, and `@@raw` on their own line as cell
/// delimiters. Returns `None` if no sentinels are found (caller should treat the
/// entire text as a single cell).
fn parse_multi_cell_source(text: &str) -> Option<Vec<ParsedCell>> {
    let lines: Vec<&str> = text.lines().collect();

    let has_sentinels = lines.iter().any(|line| sentinel_type(line).is_some());
    if !has_sentinels {
        return None;
    }

    let mut cells = Vec::new();
    let mut current_type: Option<CellType> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in &lines {
        if let Some(cell_type) = sentinel_type(line) {
            // Finish previous cell if any
            if let Some(ct) = current_type.take() {
                cells.push(ParsedCell {
                    cell_type: ct,
                    source: common::split_source(&join_cell_lines(&current_lines)),
                });
                current_lines.clear();
            }
            current_type = Some(cell_type);
        } else if current_type.is_some() {
            current_lines.push(line);
        }
        // Lines before the first sentinel are ignored
    }

    // Finish last cell
    if let Some(ct) = current_type {
        cells.push(ParsedCell {
            cell_type: ct,
            source: common::split_source(&join_cell_lines(&current_lines)),
        });
    }

    Some(cells)
}

/// Match a sentinel line to a cell type.
fn sentinel_type(line: &str) -> Option<CellType> {
    match line.trim() {
        "@@code" => Some(CellType::Code),
        "@@markdown" => Some(CellType::Markdown),
        "@@raw" => Some(CellType::Raw),
        _ => None,
    }
}

/// Join content lines back into a single string, stripping trailing blank lines.
fn join_cell_lines(lines: &[&str]) -> String {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    if end == 0 {
        return String::new();
    }
    lines[..end].join("\n")
}

/// Parse source text into one or more cells.
///
/// If the text contains `@@code`/`@@markdown`/`@@raw` sentinels, each sentinel
/// starts a new cell of that type. Otherwise a single cell of `default_type` is
/// returned.
fn parse_source_into_cells(text: &str, default_type: &CellType) -> Vec<ParsedCell> {
    parse_multi_cell_source(text).unwrap_or_else(|| {
        vec![ParsedCell {
            cell_type: default_type.clone(),
            source: common::split_source(text),
        }]
    })
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

    // Normalize notebook path
    let file_path = common::normalize_notebook_path(&args.file);

    // Compute notebook path relative to server root for Y.js connection
    let server_root = common::resolve_server_root();
    let notebook_server_path = common::notebook_path_for_server(&file_path, server_root.as_deref());

    // Read notebook to calculate insertion index and create cells
    let notebook = notebook::read_notebook(&file_path).context("Failed to read notebook")?;

    // Parse source content into cells
    let raw_text = common::parse_source_text(&args.source)?;
    let parsed_cells = parse_source_into_cells(&raw_text, &args.cell_type);

    // Validate: --id can only be used with a single cell
    if parsed_cells.len() > 1 && args.id.is_some() {
        bail!("--id cannot be used when adding multiple cells");
    }

    // Determine insertion index
    let insert_index = if let Some(idx) = args.insert_at {
        if idx < 0 {
            let abs_idx = idx.unsigned_abs() as usize;
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

    // Create all cells
    let mut new_cells: Vec<Cell> = Vec::new();
    let mut added_cells: Vec<AddedCellInfo> = Vec::new();

    for (i, parsed) in parsed_cells.into_iter().enumerate() {
        let cell_id = if let Some(ref id) = args.id {
            if notebook.cells.iter().any(|c| c.id().as_str() == *id) {
                bail!("Cell ID '{}' already exists in notebook", id);
            }
            CellId::new(id).map_err(|e| anyhow::anyhow!("Invalid cell ID: {}", e))?
        } else {
            CellId::from(Uuid::new_v4())
        };

        let cell_type_str = cell_type_to_str(&parsed.cell_type);
        let metadata = create_empty_metadata();
        let new_cell = create_cell(parsed.cell_type, cell_id.clone(), metadata, parsed.source);

        added_cells.push(AddedCellInfo {
            cell_type: cell_type_str.to_string(),
            cell_id: cell_id.to_string(),
            index: insert_index + i,
        });

        new_cells.push(new_cell);
    }

    // Add cells via Y.js (don't write to file - let JupyterLab handle persistence)
    ydoc_notebook_ops::ydoc_add_cells(
        &server_url,
        &token,
        &notebook_server_path,
        &new_cells,
        insert_index,
    )
    .await
    .context("Error adding cells")?;

    // Output result
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    let num_added = added_cells.len();
    output_results(
        &file_path,
        added_cells,
        notebook.cells.len() + num_added,
        &format,
    )?;

    Ok(())
}

fn execute_file_based(args: AddCellArgs) -> Result<()> {
    // Normalize notebook path
    let file_path = common::normalize_notebook_path(&args.file);

    // Read notebook
    let mut notebook = notebook::read_notebook(&file_path).context("Failed to read notebook")?;

    // Parse source content into cells
    let raw_text = common::parse_source_text(&args.source)?;
    let parsed_cells = parse_source_into_cells(&raw_text, &args.cell_type);

    // Validate: --id can only be used with a single cell
    if parsed_cells.len() > 1 && args.id.is_some() {
        bail!("--id cannot be used when adding multiple cells");
    }

    // Determine insertion index
    let insert_index = if let Some(idx) = args.insert_at {
        if idx < 0 {
            let abs_idx = idx.unsigned_abs() as usize;
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

    // Create and insert cells
    let mut added_cells: Vec<AddedCellInfo> = Vec::new();

    for (i, parsed) in parsed_cells.into_iter().enumerate() {
        let cell_id = if let Some(ref id) = args.id {
            if notebook.cells.iter().any(|c| c.id().as_str() == *id) {
                bail!("Cell ID '{}' already exists in notebook", id);
            }
            CellId::new(id).map_err(|e| anyhow::anyhow!("Invalid cell ID: {}", e))?
        } else {
            CellId::from(Uuid::new_v4())
        };

        let cell_type_str = cell_type_to_str(&parsed.cell_type);
        let metadata = create_empty_metadata();
        let new_cell = create_cell(parsed.cell_type, cell_id.clone(), metadata, parsed.source);

        let actual_index = insert_index + i;
        notebook.cells.insert(actual_index, new_cell);

        added_cells.push(AddedCellInfo {
            cell_type: cell_type_str.to_string(),
            cell_id: cell_id.to_string(),
            index: actual_index,
        });
    }

    // Write notebook atomically
    notebook::write_notebook_atomic(&file_path, &notebook).context("Failed to write notebook")?;

    // Output result
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    output_results(&file_path, added_cells, notebook.cells.len(), &format)?;

    Ok(())
}

fn cell_type_to_str(ct: &CellType) -> &'static str {
    match ct {
        CellType::Code => "code",
        CellType::Markdown => "markdown",
        CellType::Raw => "raw",
    }
}

fn create_cell(
    cell_type: CellType,
    id: CellId,
    metadata: nbformat::v4::CellMetadata,
    source: Vec<String>,
) -> Cell {
    match cell_type {
        CellType::Code => Cell::Code {
            id,
            metadata,
            execution_count: None,
            source,
            outputs: vec![],
        },
        CellType::Markdown => Cell::Markdown {
            id,
            metadata,
            source,
            attachments: None,
        },
        CellType::Raw => Cell::Raw {
            id,
            metadata,
            source,
        },
    }
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

fn output_results(
    file: &str,
    added_cells: Vec<AddedCellInfo>,
    total_cells: usize,
    format: &OutputFormat,
) -> Result<()> {
    if added_cells.len() == 1 {
        let info = &added_cells[0];
        let result = AddCellResult {
            file: file.to_string(),
            cell_type: info.cell_type.clone(),
            cell_id: info.cell_id.clone(),
            index: info.index,
            total_cells,
        };
        output_result(&result, format)
    } else {
        let result = AddCellsResult {
            file: file.to_string(),
            cells_added: added_cells.len(),
            total_cells,
            cells: added_cells,
        };
        output_multi_result(&result, format)
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

fn output_multi_result(result: &AddCellsResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!("Added {} cells to: {}", result.cells_added, result.file);
            for cell in &result.cells {
                println!(
                    "  Cell ID: {} (type: {}, index: {})",
                    cell.cell_id, cell.cell_type, cell.index
                );
            }
            println!("Total cells: {}", result.total_cells);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sentinel_type() {
        assert!(matches!(sentinel_type("@@code"), Some(CellType::Code)));
        assert!(matches!(
            sentinel_type("@@markdown"),
            Some(CellType::Markdown)
        ));
        assert!(matches!(sentinel_type("@@raw"), Some(CellType::Raw)));
        assert!(sentinel_type("@@cell").is_none());
        assert!(sentinel_type("@@output").is_none());
        assert!(sentinel_type("not a sentinel").is_none());
        assert!(sentinel_type("").is_none());
        // Trimmed
        assert!(matches!(sentinel_type("  @@code  "), Some(CellType::Code)));
    }

    #[test]
    fn test_parse_multi_cell_no_sentinels() {
        assert!(parse_multi_cell_source("x = 1\ny = 2").is_none());
        assert!(parse_multi_cell_source("").is_none());
        assert!(parse_multi_cell_source("just plain text").is_none());
    }

    #[test]
    fn test_parse_multi_cell_single_sentinel() {
        let cells = parse_multi_cell_source("@@code\nx = 1").unwrap();
        assert_eq!(cells.len(), 1);
        assert!(matches!(cells[0].cell_type, CellType::Code));
        assert_eq!(cells[0].source.join(""), "x = 1");
    }

    #[test]
    fn test_parse_multi_cell_multiple_sentinels() {
        let input = "@@code\nx = 1\n@@markdown\n# Title\n@@raw\nraw stuff";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 3);
        assert!(matches!(cells[0].cell_type, CellType::Code));
        assert_eq!(cells[0].source.join(""), "x = 1");
        assert!(matches!(cells[1].cell_type, CellType::Markdown));
        assert_eq!(cells[1].source.join(""), "# Title");
        assert!(matches!(cells[2].cell_type, CellType::Raw));
        assert_eq!(cells[2].source.join(""), "raw stuff");
    }

    #[test]
    fn test_parse_multi_cell_multiline_source() {
        let input = "@@code\nx = 1\ny = 2\nz = 3\n@@markdown\n# Title\nSome text";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].source.join(""), "x = 1\ny = 2\nz = 3");
        assert_eq!(cells[1].source.join(""), "# Title\nSome text");
    }

    #[test]
    fn test_parse_multi_cell_empty_cell() {
        let input = "@@code\n@@markdown\n# Title";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert!(cells[0].source.is_empty());
        assert_eq!(cells[1].source.join(""), "# Title");
    }

    #[test]
    fn test_parse_multi_cell_content_before_first_sentinel_ignored() {
        let input = "ignored preamble\n@@code\nx = 1";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].source.join(""), "x = 1");
    }

    #[test]
    fn test_parse_multi_cell_trailing_blank_lines_stripped() {
        let input = "@@code\nx = 1\n\n\n@@markdown\n# Title\n\n";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].source.join(""), "x = 1");
        assert_eq!(cells[1].source.join(""), "# Title");
    }
}
