use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use nbformat::v4::Cell;
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

/// Get the Jupyter server root directory from saved connection config.
/// Returns None for manual connections or when no config exists.
pub fn resolve_server_root() -> Option<String> {
    Config::load()
        .ok()
        .and_then(|c| c.connection)
        .and_then(|c| c.working_dir)
}

/// Compute a notebook path relative to the Jupyter server root.
///
/// The Jupyter Server identifies notebooks by their path relative to the
/// server's root directory. This function canonicalizes the given file path
/// and strips the server root prefix to produce that relative path.
///
/// Falls back to the user-provided path when the server root is unknown
/// (manual connections) or when the notebook is outside the server root.
pub fn notebook_path_for_server(file_path: &str, server_root: Option<&str>) -> String {
    let Some(root) = server_root else {
        return file_path.to_string();
    };

    let Ok(abs) = std::fs::canonicalize(file_path) else {
        return file_path.to_string();
    };

    let Ok(root_canon) = std::fs::canonicalize(root) else {
        return file_path.to_string();
    };

    abs.strip_prefix(&root_canon)
        .ok()
        .and_then(|rel| rel.to_str().map(String::from))
        .unwrap_or_else(|| file_path.to_string())
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
}
