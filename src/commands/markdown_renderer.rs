use crate::commands::common::{
    self, is_binary_mime_type, AI_NOTEBOOK_FORMAT,
};
use anyhow::{Context, Result};
use jupyter_protocol::media::Media;
use nbformat::v4::{Cell, Notebook, Output};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Main entry point for rendering a notebook in AI-Optimized Markdown format
pub fn render_notebook_markdown(
    notebook: &Notebook,
    include_outputs: bool,
    output_dir: Option<&Path>,
    inline_limit: usize,
) -> Result<String> {
    let mut result = String::new();

    // Render notebook header
    result.push_str(&render_notebook_header(notebook)?);
    result.push_str("\n\n");

    // Extract language from kernelspec
    let language = extract_language(notebook);

    // Render each cell with index
    for (index, cell) in notebook.cells.iter().enumerate() {
        result.push_str(&render_cell(cell, &language, include_outputs, output_dir, inline_limit, index)?);
        result.push_str("\n");
    }

    Ok(result)
}

/// Render the notebook header with format metadata
fn render_notebook_header(notebook: &Notebook) -> Result<String> {
    let mut metadata_obj = json!({});

    // Include kernelspec if present
    if let Some(kernelspec) = &notebook.metadata.kernelspec {
        metadata_obj["kernelspec"] = json!({
            "name": kernelspec.name,
            "display_name": kernelspec.display_name,
        });
    }

    // Build the header JSON
    let header = if metadata_obj.as_object().unwrap().is_empty() {
        // No metadata, just format
        json!({
            "format": AI_NOTEBOOK_FORMAT,
        })
    } else {
        json!({
            "format": AI_NOTEBOOK_FORMAT,
            "metadata": metadata_obj,
        })
    };

    Ok(format!("@@notebook {}", serde_json::to_string(&header)?))
}

/// Render a single cell (any type)
fn render_cell(
    cell: &Cell,
    language: &str,
    include_outputs: bool,
    output_dir: Option<&Path>,
    inline_limit: usize,
    index: usize,
) -> Result<String> {
    let mut result = String::new();

    // Get cell ID - use empty string if missing
    let cell_id = get_cell_id_or_empty(cell);

    // Render cell header
    match cell {
        Cell::Markdown { metadata, .. } => {
            result.push_str(&render_cell_header(
                cell,
                "markdown",
                None,
                &cell_id,
                None,
                metadata,
                index,
            )?);
            result.push('\n');
            result.push_str(&render_cell_body(cell, language)?);
        }
        Cell::Code {
            execution_count,
            metadata,
            outputs,
            ..
        } => {
            result.push_str(&render_cell_header(
                cell,
                "code",
                Some(language),
                &cell_id,
                *execution_count,
                metadata,
                index,
            )?);
            result.push('\n');
            result.push_str(&render_cell_body(cell, language)?);

            // Render outputs if requested
            if include_outputs && !outputs.is_empty() {
                result.push('\n');
                result.push_str(&render_outputs(outputs, output_dir, inline_limit)?);
            }
        }
        Cell::Raw { metadata, .. } => {
            result.push_str(&render_cell_header(
                cell,
                "raw",
                None,
                &cell_id,
                None,
                metadata,
                index,
            )?);
            result.push('\n');
            result.push_str(&render_cell_body(cell, language)?);
        }
    }

    Ok(result)
}

/// Render cell header with JSON metadata
fn render_cell_header(
    _cell: &Cell,
    cell_type: &str,
    _lang: Option<&str>,
    id: &str,
    execution_count: Option<i32>,
    metadata: &nbformat::v4::CellMetadata,
    index: usize,
) -> Result<String> {
    // Build JSON with canonical key order: index, id, cell_type, execution_count, metadata
    // Note: Using nbformat v4.5 schema property names where applicable
    // "index" is an extension for the markdown format to track cell position
    let mut json_obj = serde_json::Map::new();

    json_obj.insert("index".to_string(), json!(index));
    json_obj.insert("id".to_string(), json!(id));
    json_obj.insert("cell_type".to_string(), json!(cell_type));

    if let Some(count) = execution_count {
        json_obj.insert("execution_count".to_string(), json!(count));
    }

    // Include metadata only if it has meaningful content
    let metadata_value = serde_json::to_value(metadata)?;
    if let Some(obj) = metadata_value.as_object() {
        if !obj.is_empty() {
            json_obj.insert("metadata".to_string(), metadata_value);
        }
    }

    let json_str = serde_json::to_string(&json_obj)?;
    Ok(format!("@@cell {}", json_str))
}

/// Render cell body (source content)
fn render_cell_body(cell: &Cell, language: &str) -> Result<String> {
    let source = common::cell_to_string(cell);

    match cell {
        Cell::Markdown { .. } => {
            // Markdown cells: output raw markdown text (no fence)
            Ok(source)
        }
        Cell::Code { .. } => {
            // Code cells: wrap in fenced code block
            Ok(format!("```{}\n{}\n```", language, source))
        }
        Cell::Raw { .. } => {
            // Raw cells: output as-is (no fence)
            Ok(source)
        }
    }
}

/// Render all outputs for a cell
fn render_outputs(outputs: &[Output], output_dir: Option<&Path>, inline_limit: usize) -> Result<String> {
    let mut result = String::new();

    for (i, output) in outputs.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        result.push_str(&render_output(output, output_dir, inline_limit)?);
    }

    Ok(result)
}

/// Render a single output
fn render_output(output: &Output, output_dir: Option<&Path>, inline_limit: usize) -> Result<String> {
    match output {
        Output::Stream { name, text } => {
            let text_str = text.0.clone();

            if text_str.len() > inline_limit {
                // Externalize large output
                if let Some(dir) = output_dir {
                    let path = externalize_output(text_str.as_bytes(), "text/plain", dir)?;
                    Ok(render_output_header(
                        "stream",
                        Some(name),
                        None,
                        None,
                        None,
                        None,
                        Some(&path),
                        &json!({}),
                    ))
                } else {
                    // No output dir provided, inline it anyway
                    Ok(format!(
                        "{}\n{}",
                        render_output_header(
                            "stream",
                            Some(name),
                            None,
                            None,
                            None,
                            None,
                            None,
                            &json!({})
                        ),
                        render_inline_text_output(&text_str, "text/plain")
                    ))
                }
            } else {
                // Inline output
                Ok(format!(
                    "{}\n{}",
                    render_output_header(
                        "stream",
                        Some(name),
                        None,
                        None,
                        None,
                        None,
                        None,
                        &json!({})
                    ),
                    render_inline_text_output(&text_str, "text/plain")
                ))
            }
        }
        Output::ExecuteResult(result) => {
            let metadata_value = serde_json::to_value(&result.metadata)?;
            render_output_data(
                "execute_result",
                &result.data,
                Some(result.execution_count.0 as u32),
                output_dir,
                &metadata_value,
                inline_limit,
            )
        }
        Output::DisplayData(data) => {
            let metadata_value = serde_json::to_value(&data.metadata)?;
            render_output_data("display_data", &data.data, None, output_dir, &metadata_value, inline_limit)
        }
        Output::Error(error) => {
            let traceback = error.traceback.join("\n");
            Ok(format!(
                "{}\n{}",
                render_output_header(
                    "error",
                    None,
                    None,
                    None,
                    Some(&error.ename),
                    Some(&error.evalue),
                    None,
                    &json!({})
                ),
                render_inline_text_output(&traceback, "text/plain")
            ))
        }
    }
}

/// Render output data (execute_result or display_data)
fn render_output_data(
    output_type: &str,
    data: &Media,
    execution_count: Option<u32>,
    output_dir: Option<&Path>,
    metadata: &serde_json::Value,
    inline_limit: usize,
) -> Result<String> {
    // Convert Media to JSON to access mime types
    let data_json = serde_json::to_value(data)?;
    let data_obj = data_json
        .as_object()
        .context("Media data is not an object")?;

    // MIME type priority matching JupyterLab's renderMimeRegistry
    // See: https://github.com/jupyterlab/jupyterlab/blob/master/packages/rendermime/src/registry.ts
    let mime_priority = [
        // Jupyter-specific widgets and interactive outputs
        "application/vnd.jupyter.widget-view+json",
        "application/vnd.jupyter.widget-state+json",
        // Rich HTML output (interactive, styled)
        "text/html",
        // Formatted text with structure
        "text/markdown",
        // Vector graphics (scales without loss)
        "image/svg+xml",
        // LaTeX math
        "text/latex",
        // Images (raster)
        "image/png",
        "image/jpeg",
        "image/gif",
        // JavaScript (executable code)
        "application/javascript",
        // Structured data
        "application/json",
        // Fallback to plain text
        "text/plain",
    ];

    let mut selected_mime: Option<&String> = None;
    for mime in &mime_priority {
        if data_obj.contains_key(*mime) {
            selected_mime = data_obj.keys().find(|k| k.as_str() == *mime);
            break;
        }
    }

    // If no priority mime found, use first available
    if selected_mime.is_none() {
        selected_mime = data_obj.keys().next();
    }

    let mime = selected_mime.context("No mime type found in output data")?;
    let content = &data_obj[mime];

    // Extract text content
    let text_content = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            // Join array of strings
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("")
        }
        _ => serde_json::to_string(content)?,
    };

    // Check if binary or needs externalization
    if is_binary_mime_type(mime) {
        // Binary output - always externalize
        if let Some(dir) = output_dir {
            // Decode base64 for all binary types (images, audio, video, PDF)
            // Binary data in Jupyter notebooks is typically base64 encoded
            let bytes = base64_decode(&text_content)?;

            let path = externalize_output(&bytes, mime, dir)?;
            Ok(render_output_header(
                output_type,
                None,
                Some(mime),
                execution_count,
                None,
                None,
                Some(&path),
                metadata,
            ))
        } else {
            // No output dir, can't externalize binary
            Ok(render_output_header(
                output_type,
                None,
                Some(mime),
                execution_count,
                None,
                None,
                None,
                metadata,
            ))
        }
    } else if text_content.len() > inline_limit {
        // Large text output - externalize
        if let Some(dir) = output_dir {
            let path = externalize_output(text_content.as_bytes(), mime, dir)?;
            Ok(render_output_header(
                output_type,
                None,
                Some(mime),
                execution_count,
                None,
                None,
                Some(&path),
                metadata,
            ))
        } else {
            // No output dir, inline anyway
            Ok(format!(
                "{}\n{}",
                render_output_header(
                    output_type,
                    None,
                    Some(mime),
                    execution_count,
                    None,
                    None,
                    None,
                    metadata
                ),
                render_inline_text_output(&text_content, mime)
            ))
        }
    } else {
        // Inline text output
        Ok(format!(
            "{}\n{}",
            render_output_header(
                output_type,
                None,
                Some(mime),
                execution_count,
                None,
                None,
                None,
                metadata
            ),
            render_inline_text_output(&text_content, mime)
        ))
    }
}

/// Render output header with JSON metadata
fn render_output_header(
    output_type: &str,
    name: Option<&str>,
    mime: Option<&str>,
    execution_count: Option<u32>,
    ename: Option<&str>,
    evalue: Option<&str>,
    path: Option<&PathBuf>,
    metadata: &serde_json::Value,
) -> String {
    // Build JSON with canonical key order
    // Note: Using nbformat v4.5 schema property names where applicable
    // Additional fields (mime, path) are extensions for the markdown format
    let mut json_obj = serde_json::Map::new();

    json_obj.insert("output_type".to_string(), json!(output_type));

    if let Some(n) = name {
        json_obj.insert("name".to_string(), json!(n));
    }

    if let Some(m) = mime {
        // Note: mime is a markdown format extension for simplicity
        // In nbformat, this would be in the data mimebundle
        json_obj.insert("mime".to_string(), json!(m));
    }

    if let Some(count) = execution_count {
        json_obj.insert("execution_count".to_string(), json!(count));
    }

    if let Some(e) = ename {
        json_obj.insert("ename".to_string(), json!(e));
    }

    if let Some(e) = evalue {
        json_obj.insert("evalue".to_string(), json!(e));
    }

    if let Some(p) = path {
        // Convert to absolute path string
        json_obj.insert("path".to_string(), json!(p.to_string_lossy()));
    }

    // Include metadata only if it has meaningful content
    if let Some(obj) = metadata.as_object() {
        if !obj.is_empty() {
            json_obj.insert("metadata".to_string(), metadata.clone());
        }
    }

    let json_str = serde_json::to_string(&json_obj).unwrap_or_default();
    format!("@@output {}", json_str)
}

/// Render inline text output wrapped in fenced code block
fn render_inline_text_output(text: &str, mime: &str) -> String {
    let fence_lang = get_fence_language(mime);
    format!("```{}\n{}\n```", fence_lang, text)
}

/// Externalize output to a file, returns the absolute file path
/// Uses content-based hashing to ensure unique filenames and prevent stale data issues
fn externalize_output(content: &[u8], mime: &str, output_dir: &Path) -> Result<PathBuf> {
    // Create output directory if it doesn't exist
    fs::create_dir_all(output_dir)?;

    // Compute SHA256 hash of content
    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash_bytes = hasher.finalize();
    let hash_hex = format!("{:x}", hash_bytes);

    // Use first 16 characters of hash for filename (64 bits, enough to avoid collisions)
    let hash_prefix = &hash_hex[..16];

    // Determine file extension from mime type
    let ext = match mime {
        "text/plain" => "txt",
        "text/html" => "html",
        "text/markdown" => "md",
        "application/json" => "json",
        "text/latex" | "text/x-latex" | "application/x-latex" => "tex",
        "text/css" => "css",
        "application/javascript" | "text/javascript" => "js",
        "application/xml" | "text/xml" => "xml",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/svg+xml" => "svg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "application/pdf" => "pdf",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        _ => "txt",
    };

    // File naming pattern: <hash>.<ext>
    // This ensures same content = same filename (deduplication)
    // and prevents agents from guessing filenames based on cell IDs
    let filename = format!("{}.{}", hash_prefix, ext);
    let path = output_dir.join(&filename);

    // Only write if file doesn't already exist (deduplication)
    if !path.exists() {
        fs::write(&path, content)?;
    }

    // Return absolute path
    let absolute_path = fs::canonicalize(&path)
        .context("Failed to get absolute path for externalized output")?;

    Ok(absolute_path)
}

/// Extract language from notebook kernelspec
fn extract_language(notebook: &Notebook) -> String {
    notebook
        .metadata
        .kernelspec
        .as_ref()
        .and_then(|ks| ks.language.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("python")
        .to_string()
}

/// Get cell ID or return empty string if missing
/// We don't generate fake IDs because agents might try to read cells by that ID later
fn get_cell_id_or_empty(cell: &Cell) -> String {
    let id_str = cell.id().as_str();

    // If ID is empty or whitespace-only, return empty string
    if id_str.trim().is_empty() {
        String::new()
    } else {
        id_str.to_string()
    }
}

/// Map MIME type to fence language
fn get_fence_language(mime: &str) -> &str {
    match mime {
        "text/plain" => "text",
        "text/html" => "html",
        "text/markdown" => "markdown",
        "application/json" => "json",
        "text/x-python" => "python",
        "application/javascript" | "text/javascript" => "javascript",
        "text/x-rsrc" => "r",
        "text/latex" | "text/x-latex" | "application/x-latex" => "latex",
        "image/svg+xml" => "svg",
        "text/css" => "css",
        "application/xml" | "text/xml" => "xml",
        "application/x-sh" | "text/x-sh" => "bash",
        "text/x-ruby" => "ruby",
        "text/x-java" => "java",
        "text/x-c" => "c",
        "text/x-c++" | "text/x-cpp" => "cpp",
        _ => "text",
    }
}

/// Decode base64 string to bytes
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    // Simple base64 decoding - handle standard base64
    // Remove whitespace
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();

    // Use base64 decoding (we'll need to add base64 crate or use a simple implementation)
    // For now, let's use a basic implementation
    base64_decode_simple(&cleaned)
}

/// Simple base64 decoder
fn base64_decode_simple(encoded: &str) -> Result<Vec<u8>> {
    const DECODE_TABLE: [u8; 256] = [
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 62, 255, 255, 255, 63,
        52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 255, 255, 255, 0, 255, 255,
        255, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
        15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 255, 255, 255, 255, 255,
        255, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
        41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
    ];

    let bytes = encoded.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let mut buf = [0u8; 4];
        let mut buf_len = 0;

        // Read up to 4 valid base64 characters
        while buf_len < 4 && i < bytes.len() {
            let c = bytes[i];
            i += 1;

            if c == b'=' {
                break;
            }

            let decoded = DECODE_TABLE[c as usize];
            if decoded != 255 {
                buf[buf_len] = decoded;
                buf_len += 1;
            }
        }

        if buf_len >= 2 {
            result.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if buf_len >= 3 {
            result.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if buf_len >= 4 {
            result.push((buf[2] << 6) | buf[3]);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_fence_language() {
        assert_eq!(get_fence_language("text/plain"), "text");
        assert_eq!(get_fence_language("text/html"), "html");
        assert_eq!(get_fence_language("application/json"), "json");
        assert_eq!(get_fence_language("text/markdown"), "markdown");
    }

    #[test]
    fn test_is_binary_mime_type() {
        assert!(is_binary_mime_type("image/png"));
        assert!(is_binary_mime_type("image/jpeg"));
        assert!(is_binary_mime_type("audio/mp3"));
        assert!(is_binary_mime_type("video/mp4"));
        assert!(is_binary_mime_type("application/pdf"));
        assert!(!is_binary_mime_type("text/plain"));
        assert!(!is_binary_mime_type("application/json"));
    }

    #[test]
    fn test_get_cell_id_or_empty() {
        use nbformat::v4::{Cell, CellId, CellMetadata};
        use std::collections::HashMap;

        // Test with valid ID
        let cell_with_id = Cell::Code {
            id: CellId::new("test-id").unwrap(),
            metadata: CellMetadata {
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
            },
            execution_count: None,
            source: vec![],
            outputs: vec![],
        };
        assert_eq!(get_cell_id_or_empty(&cell_with_id), "test-id");

        // Note: Cannot easily test empty ID case as CellId::new("") would fail
        // The get_cell_id_or_empty function handles this at runtime by returning empty string
    }
}
