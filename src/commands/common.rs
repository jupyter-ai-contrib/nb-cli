use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use nbformat::v4::Cell;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{self, Read};

use crate::config::Config;
use crate::execution::types::ExecutionMode;

#[derive(Clone, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CellType {
    Code,
    Markdown,
    Raw,
}

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Json,
    /// AI-Optimized Markdown format (for read/search commands that render notebook content)
    Markdown,
    /// Plain text format (for mutating commands like add/delete/update/create/clear/execute)
    Text,
}

// AI-Optimized Markdown format constants
pub const AI_NOTEBOOK_FORMAT: &str = "ai-notebook";

/// Default maximum characters for inline output before externalization
pub const DEFAULT_INLINE_LIMIT: usize = 4000;

/// Normalize notebook path by adding .ipynb extension if missing
/// This allows users to omit the .ipynb extension for convenience
pub fn normalize_notebook_path(path: &str) -> String {
    if path.ends_with(".ipynb") {
        path.to_string()
    } else {
        format!("{}.ipynb", path)
    }
}

/// Check if output is binary (non-text MIME type)
pub fn is_binary_mime_type(mime: &str) -> bool {
    (mime.starts_with("image/") && mime != "image/svg+xml")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime == "application/octet-stream"
        || mime == "application/pdf"
}

/// Normalize a cell index, supporting negative indexing (e.g., -1 for last cell)
pub fn normalize_index(index: i32, len: usize) -> Result<usize> {
    if index < 0 {
        let abs_index = index.unsigned_abs() as usize;
        if abs_index > len {
            bail!(
                "Negative index {} out of range (notebook has {} cells)",
                index,
                len
            );
        }
        Ok(len - abs_index)
    } else {
        let idx = index as usize;
        if idx >= len {
            bail!(
                "Cell index {} out of range (notebook has {} cells)",
                index,
                len
            );
        }
        Ok(idx)
    }
}

/// Find a cell by ID, returning its index and an immutable reference
pub fn find_cell_by_id<'a>(cells: &'a [Cell], cell_id: &str) -> Result<(usize, &'a Cell)> {
    for (index, cell) in cells.iter().enumerate() {
        if cell.id().as_str() == cell_id {
            return Ok((index, cell));
        }
    }
    bail!("Cell with ID '{}' not found", cell_id);
}

/// Find a cell by ID, returning its index and a mutable reference
pub fn find_cell_by_id_mut<'a>(
    cells: &'a mut [Cell],
    cell_id: &str,
) -> Result<(usize, &'a mut Cell)> {
    for (index, cell) in cells.iter_mut().enumerate() {
        if cell.id().as_str() == cell_id {
            return Ok((index, cell));
        }
    }
    bail!("Cell with ID '{}' not found", cell_id);
}

/// Parse source text from a string or stdin (when input is "-")
/// Returns a Vec<String> in Jupyter's line format
pub fn parse_source(input: &str) -> Result<Vec<String>> {
    let text = parse_source_text(input)?;
    Ok(split_source(&text))
}

/// Parse source text from a string or stdin (when input is "-")
/// Returns the raw text string (without converting to Jupyter line format)
pub fn parse_source_text(input: &str) -> Result<String> {
    if input == "-" {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("Failed to read from stdin")?;
        Ok(buffer)
    } else {
        Ok(unescape_string(input))
    }
}

/// Unescape common escape sequences in a string
fn unescape_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push('\n');
                }
                Some('t') => {
                    chars.next();
                    result.push('\t');
                }
                Some('r') => {
                    chars.next();
                    result.push('\r');
                }
                Some('\\') => {
                    chars.next();
                    result.push('\\');
                }
                Some('\'') => {
                    chars.next();
                    result.push('\'');
                }
                Some('"') => {
                    chars.next();
                    result.push('"');
                }
                _ => {
                    // Unknown escape sequence, keep the backslash
                    result.push(ch);
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Split text into Jupyter's line format (Vec<String> with newlines preserved)
/// Each line ends with \n except the last line
pub fn split_source(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![];
    }

    let ends_with_newline = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();

    if lines.is_empty() {
        return vec![text.to_string()];
    }

    let mut result: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i < lines.len() - 1 || ends_with_newline {
                format!("{}\n", line)
            } else {
                line.to_string()
            }
        })
        .collect();

    // If text ended with newline, add empty string at end
    if ends_with_newline {
        result.push(String::new());
    }

    result
}

/// Serialize notebook cells to JSON values with an `index` field added to each.
/// When `include_outputs` is false, the `outputs` field is stripped from code cells.
/// Used by both `read` and `execute` commands for consistent JSON output.
pub fn serialize_cells_json(cells: &[Cell], include_outputs: bool) -> Vec<serde_json::Value> {
    cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let mut cell_json = serde_json::to_value(cell).unwrap_or(serde_json::json!(null));
            if let Some(obj) = cell_json.as_object_mut() {
                obj.insert("index".to_string(), serde_json::json!(index));
                if !include_outputs {
                    obj.remove("outputs");
                }
            }
            cell_json
        })
        .collect()
}

/// Convert cell source to a single string
pub fn cell_to_string(cell: &Cell) -> String {
    cell.source().join("")
}

/// Convert cell ID to a string
pub fn cell_id_to_string(cell: &Cell) -> String {
    cell.id().to_string()
}

/// Lowercase type name of a cell, e.g. "code"/"markdown"/"raw"
pub fn cell_type_str(cell: &Cell) -> &'static str {
    match cell {
        Cell::Code { .. } => "code",
        Cell::Markdown { .. } => "markdown",
        Cell::Raw { .. } => "raw",
    }
}

/// Lowercase type name of a `CellType` arg value, matching `cell_type_str`
pub fn cell_type_enum_str(ct: &CellType) -> &'static str {
    match ct {
        CellType::Code => "code",
        CellType::Markdown => "markdown",
        CellType::Raw => "raw",
    }
}

/// An empty `CellMetadata` with all fields unset, used when constructing
/// fresh cells (e.g. `nb create`, `nb cell add`) that have no metadata yet.
pub fn empty_cell_metadata() -> nbformat::v4::CellMetadata {
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

/// Print a command result as pretty JSON, or via `text_fn` for the
/// Text/Markdown formats. Centralizes the `OutputFormat::Json => println!(...)`
/// boilerplate repeated across mutating commands.
pub fn print_result<T: Serialize>(
    result: &T,
    format: &OutputFormat,
    text_fn: impl FnOnce(&T),
) -> Result<()> {
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(result)?),
        OutputFormat::Text | OutputFormat::Markdown => text_fn(result),
    }
    Ok(())
}

/// Resolve execution mode from CLI args and saved config
/// CLI arguments take priority over saved configuration
pub fn resolve_execution_mode(
    server_arg: Option<String>,
    token_arg: Option<String>,
) -> Result<ExecutionMode> {
    // If both server and token are provided, use them directly
    if let Some(server_url) = &server_arg {
        let token = token_arg
            .as_ref()
            .context("Must specify --token when using --server")?;
        return Ok(ExecutionMode::Remote {
            server_url: server_url.clone(),
            token: token.clone(),
        });
    }

    // If only one is provided, that's an error
    if token_arg.is_some() {
        bail!("Cannot specify --token without --server");
    }

    // Try to load from config
    let config = Config::load().context("Failed to load config")?;

    if let Some((server_url, token)) = config.resolve_connection(server_arg, token_arg)? {
        Ok(ExecutionMode::Remote { server_url, token })
    } else {
        Ok(ExecutionMode::Local)
    }
}

/// Resolve cached ydoc_available from saved connection config.
/// Returns None for ad-hoc --server/--token connections and for saved
/// connections without a cached probe result. None is a routing hint meaning
/// "unknown": the executor tries Y.js and falls back to the Contents API path
/// on the definitive backend-absent signal.
pub fn resolve_ydoc_available(
    server_arg: &Option<String>,
    token_arg: &Option<String>,
) -> Option<bool> {
    if server_arg.is_some() && token_arg.is_some() {
        return None;
    }
    Config::load()
        .ok()
        .and_then(|c| c.connection)
        .and_then(|c| c.ydoc_available)
}

/// Print the self-heal hint when a cached "collaborative" connection turns
/// out to have no Y.js backend anymore.
pub fn warn_stale_collab_cache(cached: Option<bool>) {
    if cached == Some(true) {
        eprintln!(
            "Collaboration backend no longer found on server; using Contents API. \
             Run 'nb connect' to refresh."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_index() {
        assert_eq!(normalize_index(0, 5).unwrap(), 0);
        assert_eq!(normalize_index(4, 5).unwrap(), 4);
        assert_eq!(normalize_index(-1, 5).unwrap(), 4);
        assert_eq!(normalize_index(-5, 5).unwrap(), 0);
        assert!(normalize_index(5, 5).is_err());
        assert!(normalize_index(-6, 5).is_err());
    }

    #[test]
    fn test_split_source() {
        assert_eq!(split_source(""), Vec::<String>::new());
        assert_eq!(split_source("single line"), vec!["single line"]);
        assert_eq!(
            split_source("line1\nline2\nline3"),
            vec!["line1\n", "line2\n", "line3"]
        );
        assert_eq!(
            split_source("line1\nline2\n"),
            vec!["line1\n", "line2\n", ""]
        );
    }

    #[test]
    fn test_unescape_string() {
        assert_eq!(super::unescape_string("hello\\nworld"), "hello\nworld");
        assert_eq!(super::unescape_string("tab\\there"), "tab\there");
        assert_eq!(
            super::unescape_string("backslash\\\\here"),
            "backslash\\here"
        );
        assert_eq!(super::unescape_string("quote\\'here"), "quote'here");
        assert_eq!(super::unescape_string("no escapes"), "no escapes");
        assert_eq!(super::unescape_string("\\n\\t\\r"), "\n\t\r");
    }

    #[test]
    fn test_is_binary_mime_type_svg_exception() {
        assert!(
            !is_binary_mime_type("image/svg+xml"),
            "SVG must not be binary"
        );
        assert!(is_binary_mime_type("image/png"), "PNG must be binary");
        assert!(is_binary_mime_type("image/jpeg"), "JPEG must be binary");
        assert!(is_binary_mime_type("application/pdf"), "PDF must be binary");
        assert!(!is_binary_mime_type("text/html"), "HTML must not be binary");
    }

    #[test]
    fn test_resolve_execution_mode_flag_error_cases() {
        let server = Some("http://localhost:8888".to_string());
        let token = Some("tok".to_string());

        assert!(resolve_execution_mode(server.clone(), None).is_err());
        assert!(resolve_execution_mode(None, token.clone()).is_err());
        assert!(resolve_execution_mode(server, token).is_ok());
    }
}
