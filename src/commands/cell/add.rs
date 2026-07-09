use crate::commands::common::{self, CellType, OutputFormat};
use crate::execution::server::ydoc_notebook_ops;
use crate::notebook::session::{resolve_backend, run_mutation, CellMutator};
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::{Cell, CellId, CellMetadata, Notebook};
use serde::Serialize;
use uuid::Uuid;

#[derive(Parser, Clone)]
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

    /// Cell source content (use '-' for stdin). Start with a sentinel line
    /// (@@code, @@markdown, @@raw, or @@cell {"cell_type":"..."}) to add
    /// multiple cells in one call.
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

#[derive(Serialize, Clone)]
struct AddedCellInfo {
    cell_type: String,
    cell_id: String,
    index: usize,
}

/// A cell parsed from sentinel-delimited source input.
struct ParsedCell {
    cell_type: CellType,
    source: Vec<String>,
    metadata: Option<CellMetadata>,
}

/// Try to parse the source text as sentinel-delimited multi-cell input.
///
/// Recognizes `@@code`, `@@markdown`, `@@raw`, and `@@cell {"cell_type": "..."}` on
/// their own line as cell delimiters. Multi-cell mode is only activated when the
/// first non-empty line is a sentinel; this prevents accidental data loss when cell
/// content happens to mention these tokens. Returns `None` if the first non-empty
/// line is not a sentinel (caller should treat the entire text as a single cell).
fn parse_multi_cell_source(text: &str) -> Option<Vec<ParsedCell>> {
    let lines: Vec<&str> = text.lines().collect();

    // Multi-cell mode is only activated when the first non-empty line is a
    // sentinel. This prevents accidental data loss when cell content happens
    // to contain @@code/@@markdown/@@raw as literal text.
    let first_non_empty = lines.iter().find(|line| !line.trim().is_empty());
    if first_non_empty.is_none_or(|line| parse_sentinel(line).is_none()) {
        return None;
    }

    let mut cells = Vec::new();
    let mut current_type: Option<CellType> = None;
    let mut current_metadata: Option<CellMetadata> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in &lines {
        if let Some(info) = parse_sentinel(line) {
            // Finish previous cell if any
            if let Some(ct) = current_type.take() {
                cells.push(ParsedCell {
                    cell_type: ct,
                    source: common::split_source(&join_cell_lines(&current_lines)),
                    metadata: current_metadata.take(),
                });
                current_lines.clear();
            }
            current_type = Some(info.cell_type);
            current_metadata = info.metadata;
        } else if current_type.is_some() {
            current_lines.push(line);
        }
    }

    // Finish last cell
    if let Some(ct) = current_type {
        cells.push(ParsedCell {
            cell_type: ct,
            source: common::split_source(&join_cell_lines(&current_lines)),
            metadata: current_metadata,
        });
    }

    Some(cells)
}

/// Parsed sentinel data: cell type and optional metadata from the JSON block.
struct SentinelInfo {
    cell_type: CellType,
    metadata: Option<CellMetadata>,
}

/// Parse a sentinel line into its cell type and optional metadata.
///
/// Accepts both shorthand (`@@code`, `@@markdown`, `@@raw`) and the full
/// `@@cell {"cell_type": "...", "metadata": {...}}` format produced by `nb read`.
/// When the `@@cell` JSON format includes a `"metadata"` object, it is deserialized
/// as `CellMetadata` and carried through to cell creation.
fn parse_sentinel(line: &str) -> Option<SentinelInfo> {
    let trimmed = line.trim();
    match trimmed {
        "@@code" => Some(SentinelInfo {
            cell_type: CellType::Code,
            metadata: None,
        }),
        "@@markdown" => Some(SentinelInfo {
            cell_type: CellType::Markdown,
            metadata: None,
        }),
        "@@raw" => Some(SentinelInfo {
            cell_type: CellType::Raw,
            metadata: None,
        }),
        _ if trimmed.starts_with("@@cell ") => {
            let json_str = trimmed.strip_prefix("@@cell ")?.trim();
            let json: serde_json::Value = serde_json::from_str(json_str).ok()?;
            let cell_type = match json.get("cell_type")?.as_str()? {
                "code" => CellType::Code,
                "markdown" => CellType::Markdown,
                "raw" => CellType::Raw,
                _ => return None,
            };
            let metadata = json
                .get("metadata")
                .and_then(|v| serde_json::from_value::<CellMetadata>(v.clone()).ok());
            Some(SentinelInfo {
                cell_type,
                metadata,
            })
        }
        _ => None,
    }
}

/// Join content lines back into a single string, stripping leading and trailing
/// blank lines so that each cell has no empty lines at the top or bottom.
fn join_cell_lines(lines: &[&str]) -> String {
    let mut start = 0;
    while start < lines.len() && lines[start].is_empty() {
        start += 1;
    }
    let mut end = lines.len();
    while end > start && lines[end - 1].is_empty() {
        end -= 1;
    }
    if start >= end {
        return String::new();
    }
    lines[start..end].join("\n")
}

/// Parse source text into one or more cells.
///
/// If the first non-empty line is a sentinel (`@@code`/`@@markdown`/`@@raw` or
/// `@@cell {"cell_type": "..."}`), multi-cell mode is activated and each sentinel
/// starts a new cell. Otherwise a single cell of `default_type` is returned.
fn parse_source_into_cells(text: &str, default_type: &CellType) -> Vec<ParsedCell> {
    parse_multi_cell_source(text).unwrap_or_else(|| {
        vec![ParsedCell {
            cell_type: default_type.clone(),
            source: common::split_source(text),
            metadata: None,
        }]
    })
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

/// Resolve the index at which new cells should be inserted, from
/// `--insert-at`/`--after`/`--before` (mutually exclusive; default: append).
fn resolve_insert_index(args: &AddCellArgs, cells: &[Cell]) -> Result<usize> {
    if let Some(idx) = args.insert_at {
        if idx < 0 {
            let abs_idx = idx.unsigned_abs() as usize;
            if abs_idx > cells.len() {
                bail!(
                    "Negative index {} out of range (notebook has {} cells)",
                    idx,
                    cells.len()
                );
            }
            Ok(cells.len() - abs_idx)
        } else {
            let pos_idx = idx as usize;
            if pos_idx > cells.len() {
                bail!(
                    "Index {} out of range (notebook has {} cells)",
                    idx,
                    cells.len()
                );
            }
            Ok(pos_idx)
        }
    } else if let Some(ref after_id) = args.after {
        let (index, _) = common::find_cell_by_id(cells, after_id)?;
        Ok(index + 1)
    } else if let Some(ref before_id) = args.before {
        let (index, _) = common::find_cell_by_id(cells, before_id)?;
        Ok(index)
    } else {
        Ok(cells.len())
    }
}

struct AddCellMutator<'a> {
    args: &'a AddCellArgs,
}

#[async_trait::async_trait]
impl CellMutator for AddCellMutator<'_> {
    type Output = (Vec<AddedCellInfo>, usize);

    fn mutate_notebook(&self, notebook: &mut Notebook, _file_path: &str) -> Result<Self::Output> {
        let raw_text = common::parse_source_text(&self.args.source)?;
        let parsed_cells = parse_source_into_cells(&raw_text, &self.args.cell_type);

        if parsed_cells.len() > 1 && self.args.id.is_some() {
            bail!("--id cannot be used when adding multiple cells");
        }

        let insert_index = resolve_insert_index(self.args, &notebook.cells)?;
        let mut added_cells: Vec<AddedCellInfo> = Vec::new();

        for (i, parsed) in parsed_cells.into_iter().enumerate() {
            let cell_id = new_cell_id(self.args, &notebook.cells)?;
            let cell_type_str = common::cell_type_enum_str(&parsed.cell_type);
            let metadata = parsed.metadata.unwrap_or_else(common::empty_cell_metadata);
            let new_cell = create_cell(parsed.cell_type, cell_id.clone(), metadata, parsed.source);

            let actual_index = insert_index + i;
            notebook.cells.insert(actual_index, new_cell);

            added_cells.push(AddedCellInfo {
                cell_type: cell_type_str.to_string(),
                cell_id: cell_id.to_string(),
                index: actual_index,
            });
        }

        Ok((added_cells, notebook.cells.len()))
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

        let raw_text = common::parse_source_text(&self.args.source)?;
        let parsed_cells = parse_source_into_cells(&raw_text, &self.args.cell_type);

        if parsed_cells.len() > 1 && self.args.id.is_some() {
            bail!("--id cannot be used when adding multiple cells");
        }

        let insert_index = resolve_insert_index(self.args, &notebook.cells)?;

        let mut new_cells: Vec<Cell> = Vec::new();
        let mut added_cells: Vec<AddedCellInfo> = Vec::new();

        for (i, parsed) in parsed_cells.into_iter().enumerate() {
            let cell_id = new_cell_id(self.args, &notebook.cells)?;
            let cell_type_str = common::cell_type_enum_str(&parsed.cell_type);
            let metadata = parsed.metadata.unwrap_or_else(common::empty_cell_metadata);
            let new_cell = create_cell(parsed.cell_type, cell_id.clone(), metadata, parsed.source);

            added_cells.push(AddedCellInfo {
                cell_type: cell_type_str.to_string(),
                cell_id: cell_id.to_string(),
                index: insert_index + i,
            });

            new_cells.push(new_cell);
        }

        let num_added = added_cells.len();

        // Add cells via Y.js (don't write to file - let JupyterLab handle persistence)
        ydoc_notebook_ops::ydoc_add_cells(server_url, token, server_path, &new_cells, insert_index)
            .await
            .context("Error adding cells")?;

        Ok((added_cells, notebook.cells.len() + num_added))
    }
}

/// Generate (or validate a user-supplied) cell ID, checking for collisions
/// against the existing cells.
fn new_cell_id(args: &AddCellArgs, cells: &[Cell]) -> Result<CellId> {
    if let Some(ref id) = args.id {
        if cells.iter().any(|c| c.id().as_str() == *id) {
            bail!("Cell ID '{}' already exists in notebook", id);
        }
        CellId::new(id).map_err(|e| anyhow::anyhow!("Invalid cell ID: {}", e))
    } else {
        Ok(CellId::from(Uuid::new_v4()))
    }
}

pub fn execute(args: AddCellArgs) -> Result<()> {
    let backend = resolve_backend(&args.file, args.server.clone(), args.token.clone())?;
    let mutator = AddCellMutator { args: &args };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (file_path, (added_cells, total_cells)) =
        runtime.block_on(run_mutation(backend, &mutator))?;

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    output_results(&file_path, added_cells, total_cells, &format)
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
    common::print_result(result, format, |result| {
        println!("Added {} cell to: {}", result.cell_type, result.file);
        println!("Cell ID: {}", result.cell_id);
        println!(
            "Index: {} (total: {} cells)",
            result.index, result.total_cells
        );
    })
}

fn output_multi_result(result: &AddCellsResult, format: &OutputFormat) -> Result<()> {
    common::print_result(result, format, |result| {
        println!("Added {} cells to: {}", result.cells_added, result.file);
        for cell in &result.cells {
            println!(
                "  Cell ID: {} (type: {}, index: {})",
                cell.cell_id, cell.cell_type, cell.index
            );
        }
        println!("Total cells: {}", result.total_cells);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sentinel_shorthand() {
        let info = parse_sentinel("@@code").unwrap();
        assert!(matches!(info.cell_type, CellType::Code));
        assert!(info.metadata.is_none());

        let info = parse_sentinel("@@markdown").unwrap();
        assert!(matches!(info.cell_type, CellType::Markdown));
        assert!(info.metadata.is_none());

        let info = parse_sentinel("@@raw").unwrap();
        assert!(matches!(info.cell_type, CellType::Raw));
        assert!(info.metadata.is_none());

        assert!(parse_sentinel("@@output").is_none());
        assert!(parse_sentinel("not a sentinel").is_none());
        assert!(parse_sentinel("").is_none());
        // Trimmed
        assert!(matches!(
            parse_sentinel("  @@code  ").map(|i| i.cell_type),
            Some(CellType::Code)
        ));
    }

    #[test]
    fn test_parse_sentinel_cell_json() {
        // Full @@cell {json} format (matches nb read output)
        let info = parse_sentinel(r#"@@cell {"cell_type": "code"}"#).unwrap();
        assert!(matches!(info.cell_type, CellType::Code));
        assert!(info.metadata.is_none());

        assert!(matches!(
            parse_sentinel(r#"@@cell {"cell_type": "markdown"}"#).map(|i| i.cell_type),
            Some(CellType::Markdown)
        ));
        assert!(matches!(
            parse_sentinel(r#"@@cell {"cell_type": "raw"}"#).map(|i| i.cell_type),
            Some(CellType::Raw)
        ));
        // With extra fields (as produced by nb read) — no metadata key
        let info = parse_sentinel(
            r#"@@cell {"index":0,"id":"abc","cell_type":"code","execution_count":1}"#,
        )
        .unwrap();
        assert!(matches!(info.cell_type, CellType::Code));
        assert!(info.metadata.is_none());

        // @@cell without JSON or with invalid JSON
        assert!(parse_sentinel("@@cell").is_none());
        assert!(parse_sentinel("@@cell {}").is_none());
        assert!(parse_sentinel("@@cell not-json").is_none());
        // Unknown cell_type
        assert!(parse_sentinel(r#"@@cell {"cell_type": "unknown"}"#).is_none());
    }

    #[test]
    fn test_parse_sentinel_cell_json_with_metadata() {
        // Metadata with tags
        let info = parse_sentinel(
            r#"@@cell {"cell_type": "code", "metadata": {"tags": ["test", "important"]}}"#,
        )
        .unwrap();
        assert!(matches!(info.cell_type, CellType::Code));
        let meta = info.metadata.unwrap();
        assert_eq!(
            meta.tags.as_ref().unwrap(),
            &vec!["test".to_string(), "important".to_string()]
        );

        // Metadata with editable flag
        let info =
            parse_sentinel(r#"@@cell {"cell_type": "markdown", "metadata": {"editable": false}}"#)
                .unwrap();
        assert!(matches!(info.cell_type, CellType::Markdown));
        let meta = info.metadata.unwrap();
        assert_eq!(meta.editable, Some(false));

        // Empty metadata object — deserialized but all fields are None
        let info = parse_sentinel(r#"@@cell {"cell_type": "code", "metadata": {}}"#).unwrap();
        assert!(info.metadata.is_some());

        // Full nb read-style sentinel with metadata
        let info = parse_sentinel(
            r#"@@cell {"index":0,"id":"abc","cell_type":"code","metadata":{"tags":["auto"]}}"#,
        )
        .unwrap();
        let meta = info.metadata.unwrap();
        assert_eq!(meta.tags.as_ref().unwrap(), &vec!["auto".to_string()]);
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
    fn test_parse_multi_cell_content_before_sentinel_is_single_cell() {
        // When the first non-empty line is NOT a sentinel, multi-cell mode is
        // not activated — the entire text is treated as single-cell content.
        // This prevents data loss when cell content mentions @@code etc.
        let input = "ignored preamble\n@@code\nx = 1";
        assert!(parse_multi_cell_source(input).is_none());
    }

    #[test]
    fn test_parse_multi_cell_leading_blank_lines_before_sentinel() {
        // Leading blank lines before the first sentinel are fine
        let input = "\n\n@@code\nx = 1";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].source.join(""), "x = 1");
    }

    #[test]
    fn test_parse_multi_cell_cell_json_format() {
        // Accept @@cell {json} format (matches nb read output)
        let input = "@@cell {\"cell_type\": \"code\"}\nx = 1\n@@cell {\"cell_type\": \"markdown\"}\n# Title";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert!(matches!(cells[0].cell_type, CellType::Code));
        assert_eq!(cells[0].source.join(""), "x = 1");
        assert!(cells[0].metadata.is_none());
        assert!(matches!(cells[1].cell_type, CellType::Markdown));
        assert_eq!(cells[1].source.join(""), "# Title");
        assert!(cells[1].metadata.is_none());
    }

    #[test]
    fn test_parse_multi_cell_cell_json_with_metadata() {
        // Metadata from @@cell JSON is carried through to ParsedCell
        let input = "@@cell {\"cell_type\": \"code\", \"metadata\": {\"tags\": [\"setup\"]}}\nx = 1\n@@cell {\"cell_type\": \"markdown\"}\n# Title";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        let meta = cells[0].metadata.as_ref().unwrap();
        assert_eq!(meta.tags.as_ref().unwrap(), &vec!["setup".to_string()]);
        assert!(cells[1].metadata.is_none());
    }

    #[test]
    fn test_parse_multi_cell_mixed_shorthand_and_json() {
        // Mix shorthand and @@cell {json} formats
        let input = "@@code\nx = 1\n@@cell {\"cell_type\": \"markdown\"}\n# Title";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert!(matches!(cells[0].cell_type, CellType::Code));
        assert!(cells[0].metadata.is_none());
        assert!(matches!(cells[1].cell_type, CellType::Markdown));
        assert!(cells[1].metadata.is_none());
    }

    #[test]
    fn test_parse_multi_cell_trailing_blank_lines_stripped() {
        let input = "@@code\nx = 1\n\n\n@@markdown\n# Title\n\n";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].source.join(""), "x = 1");
        assert_eq!(cells[1].source.join(""), "# Title");
    }

    #[test]
    fn test_parse_multi_cell_leading_blank_lines_in_cell_stripped() {
        let input = "@@code\n\n\nx = 1\n@@markdown\n\n# Title";
        let cells = parse_multi_cell_source(input).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].source.join(""), "x = 1");
        assert_eq!(cells[1].source.join(""), "# Title");
    }
}
