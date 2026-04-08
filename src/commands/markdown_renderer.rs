use crate::commands::common::{self, is_binary_mime_type, AI_NOTEBOOK_FORMAT};
use anyhow::{Context, Result};
use base64::Engine;
use jupyter_protocol::media::Media;
use nbformat::v4::{Cell, Notebook, Output};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// A cell paired with its original index in the notebook.
/// Used to preserve correct indices when rendering filtered subsets of cells.
pub struct IndexedCell<'a> {
    pub index: usize,
    pub cell: &'a Cell,
}

/// Main entry point for rendering a notebook in AI-Optimized Markdown format
pub fn render_notebook_markdown(
    notebook: &Notebook,
    include_outputs: bool,
    output_dir: Option<&Path>,
    inline_limit: usize,
) -> Result<String> {
    let indexed_cells: Vec<IndexedCell> = notebook
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| IndexedCell { index: i, cell })
        .collect();
    render_indexed_cells_markdown(
        notebook,
        &indexed_cells,
        include_outputs,
        output_dir,
        inline_limit,
    )
}

/// Render a subset of cells with their original indices preserved.
/// This is the core rendering function used by both full-notebook and filtered views.
pub fn render_indexed_cells_markdown(
    notebook: &Notebook,
    cells: &[IndexedCell],
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

    // Render each cell with its original index
    for (i, indexed) in cells.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        result.push_str(&render_cell(
            indexed.cell,
            &language,
            include_outputs,
            output_dir,
            inline_limit,
            indexed.index,
        )?);
        result.push('\n');
    }

    Ok(result)
}

/// Render the notebook header with format metadata
pub fn render_notebook_header(notebook: &Notebook) -> Result<String> {
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

    // Render cell header and body
    match cell {
        Cell::Markdown { metadata, .. } => {
            result.push_str(&render_cell_header(
                "markdown", &cell_id, None, metadata, index,
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
                "code",
                &cell_id,
                *execution_count,
                metadata,
                index,
            )?);
            result.push('\n');
            result.push_str(&render_cell_body(cell, language)?);

            // Render outputs if requested
            if include_outputs && !outputs.is_empty() {
                result.push_str("\n\n");
                result.push_str(&render_outputs(outputs, output_dir, inline_limit)?);
            }
        }
        Cell::Raw { metadata, .. } => {
            result.push_str(&render_cell_header("raw", &cell_id, None, metadata, index)?);
            result.push('\n');
            result.push_str(&render_cell_body(cell, language)?);
        }
    }

    Ok(result)
}

/// Render cell header with JSON metadata
fn render_cell_header(
    cell_type: &str,
    id: &str,
    execution_count: Option<i32>,
    metadata: &nbformat::v4::CellMetadata,
    index: usize,
) -> Result<String> {
    // Build JSON with canonical key order: index, id, cell_type, execution_count, metadata
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
fn render_outputs(
    outputs: &[Output],
    output_dir: Option<&Path>,
    inline_limit: usize,
) -> Result<String> {
    let mut result = String::new();

    for (i, output) in outputs.iter().enumerate() {
        if i > 0 {
            result.push_str("\n\n");
        }
        result.push_str(&render_output(output, output_dir, inline_limit)?);
    }

    Ok(result)
}

/// Structured output header metadata, replacing the 8-parameter function.
struct OutputHeaderBuilder {
    output_type: String,
    name: Option<String>,
    mime: Option<String>,
    execution_count: Option<u32>,
    ename: Option<String>,
    evalue: Option<String>,
    path: Option<PathBuf>,
    metadata: serde_json::Value,
}

impl OutputHeaderBuilder {
    fn new(output_type: &str) -> Self {
        Self {
            output_type: output_type.to_string(),
            name: None,
            mime: None,
            execution_count: None,
            ename: None,
            evalue: None,
            path: None,
            metadata: json!({}),
        }
    }

    fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    fn mime(mut self, mime: &str) -> Self {
        self.mime = Some(mime.to_string());
        self
    }

    fn execution_count(mut self, count: u32) -> Self {
        self.execution_count = Some(count);
        self
    }

    fn error(mut self, ename: &str, evalue: &str) -> Self {
        self.ename = Some(ename.to_string());
        self.evalue = Some(evalue.to_string());
        self
    }

    fn path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    fn build(self) -> String {
        let mut json_obj = serde_json::Map::new();

        json_obj.insert("output_type".to_string(), json!(self.output_type));

        if let Some(n) = self.name {
            json_obj.insert("name".to_string(), json!(n));
        }

        if let Some(m) = self.mime {
            json_obj.insert("mime".to_string(), json!(m));
        }

        if let Some(count) = self.execution_count {
            json_obj.insert("execution_count".to_string(), json!(count));
        }

        if let Some(e) = self.ename {
            json_obj.insert("ename".to_string(), json!(e));
        }

        if let Some(e) = self.evalue {
            json_obj.insert("evalue".to_string(), json!(e));
        }

        if let Some(p) = self.path {
            json_obj.insert("path".to_string(), json!(p.to_string_lossy()));
        }

        // Include metadata only if it has meaningful content
        if let Some(obj) = self.metadata.as_object() {
            if !obj.is_empty() {
                json_obj.insert("metadata".to_string(), self.metadata);
            }
        }

        let json_str = serde_json::to_string(&json_obj).unwrap_or_default();
        format!("@@output {}", json_str)
    }
}

/// Render a single output (public, for streaming use)
pub fn render_single_output(
    output: &Output,
    output_dir: Option<&Path>,
    inline_limit: usize,
) -> Result<String> {
    render_output(output, output_dir, inline_limit)
}

/// Render a cell header and body without outputs (public, for streaming use).
/// The `execution_count` parameter overrides the cell's stored value.
pub fn render_cell_header_and_body(
    cell: &Cell,
    notebook: &Notebook,
    index: usize,
    execution_count: Option<i32>,
) -> Result<String> {
    let language = extract_language(notebook);
    let cell_id = get_cell_id_or_empty(cell);
    let metadata = match cell {
        Cell::Code { metadata, .. } => metadata,
        Cell::Markdown { metadata, .. } => metadata,
        Cell::Raw { metadata, .. } => metadata,
    };
    let cell_type = match cell {
        Cell::Code { .. } => "code",
        Cell::Markdown { .. } => "markdown",
        Cell::Raw { .. } => "raw",
    };
    let mut result = render_cell_header(cell_type, &cell_id, execution_count, metadata, index)?;
    result.push('\n');
    result.push_str(&render_cell_body(cell, &language)?);
    Ok(result)
}

/// Render a single output
fn render_output(
    output: &Output,
    output_dir: Option<&Path>,
    inline_limit: usize,
) -> Result<String> {
    match output {
        Output::Stream { name, text } => {
            let text_str = text.0.clone();

            if text_str.len() > inline_limit {
                if let Some(dir) = output_dir {
                    let path = externalize_output(text_str.as_bytes(), "text/plain", dir)?;
                    Ok(OutputHeaderBuilder::new("stream")
                        .name(name)
                        .path(path)
                        .build())
                } else {
                    Ok(format!(
                        "{}\n{}",
                        OutputHeaderBuilder::new("stream").name(name).build(),
                        render_inline_text_output(&text_str, "text/plain")
                    ))
                }
            } else {
                Ok(format!(
                    "{}\n{}",
                    OutputHeaderBuilder::new("stream").name(name).build(),
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
                metadata_value,
                inline_limit,
            )
        }
        Output::DisplayData(data) => {
            let metadata_value = serde_json::to_value(&data.metadata)?;
            render_output_data(
                "display_data",
                &data.data,
                None,
                output_dir,
                metadata_value,
                inline_limit,
            )
        }
        Output::Error(error) => {
            let traceback = error.traceback.join("\n");
            Ok(format!(
                "{}\n{}",
                OutputHeaderBuilder::new("error")
                    .error(&error.ename, &error.evalue)
                    .build(),
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
    metadata: serde_json::Value,
    inline_limit: usize,
) -> Result<String> {
    // Convert Media to JSON to access mime types
    let data_json = serde_json::to_value(data)?;
    let data_obj = data_json
        .as_object()
        .context("Media data is not an object")?;

    // MIME type priority matching JupyterLab's renderMimeRegistry
    let mime_priority = [
        "application/vnd.jupyter.widget-view+json",
        "application/vnd.jupyter.widget-state+json",
        "text/html",
        "text/markdown",
        "image/svg+xml",
        "text/latex",
        "image/png",
        "image/jpeg",
        "image/gif",
        "application/javascript",
        "application/json",
        "text/plain",
    ];

    let mut selected_mime: Option<&String> = None;
    for mime in &mime_priority {
        if let Some(key) = data_obj.keys().find(|k| k.as_str() == *mime) {
            selected_mime = Some(key);
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
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => serde_json::to_string(content)?,
    };

    // Build the base header
    let make_header = |path: Option<PathBuf>| {
        let mut builder = OutputHeaderBuilder::new(output_type)
            .mime(mime)
            .metadata(metadata.clone());
        if let Some(count) = execution_count {
            builder = builder.execution_count(count);
        }
        if let Some(p) = path {
            builder = builder.path(p);
        }
        builder.build()
    };

    // Check if binary or needs externalization
    if is_binary_mime_type(mime) {
        if let Some(dir) = output_dir {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(text_content.as_bytes())
                .context("Failed to decode base64 binary output")?;
            let path = externalize_output(&bytes, mime, dir)?;
            Ok(make_header(Some(path)))
        } else {
            Ok(make_header(None))
        }
    } else if text_content.len() > inline_limit {
        if let Some(dir) = output_dir {
            let path = externalize_output(text_content.as_bytes(), mime, dir)?;
            Ok(make_header(Some(path)))
        } else {
            Ok(format!(
                "{}\n{}",
                make_header(None),
                render_inline_text_output(&text_content, mime)
            ))
        }
    } else {
        Ok(format!(
            "{}\n{}",
            make_header(None),
            render_inline_text_output(&text_content, mime)
        ))
    }
}

/// Render inline text output wrapped in fenced code block
fn render_inline_text_output(text: &str, mime: &str) -> String {
    let fence_lang = get_fence_language(mime);
    format!("```{}\n{}\n```", fence_lang, text)
}

/// Externalize output to a file, returns the absolute file path.
/// Uses content-based hashing to ensure unique filenames and prevent stale data issues.
fn externalize_output(content: &[u8], mime: &str, output_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(output_dir)?;

    // Compute SHA256 hash of content
    let mut hasher = Sha256::new();
    hasher.update(content);
    let hash_bytes = hasher.finalize();
    let hash_hex = format!("{:x}", hash_bytes);

    // Use first 16 characters of hash for filename (64 bits, enough to avoid collisions)
    let hash_prefix = &hash_hex[..16];

    let ext = mime_to_extension(mime);
    let filename = format!("{}.{}", hash_prefix, ext);
    let path = output_dir.join(&filename);

    // Only write if file doesn't already exist (deduplication)
    if !path.exists() {
        fs::write(&path, content)?;
    }

    let absolute_path =
        fs::canonicalize(&path).context("Failed to get absolute path for externalized output")?;

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
fn get_cell_id_or_empty(cell: &Cell) -> String {
    let id_str = cell.id().as_str();
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

/// Map MIME type to file extension
fn mime_to_extension(mime: &str) -> &str {
    match mime {
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
    }
}

/// Compute the output directory for a notebook.
/// Creates a deterministic path: `<temp>/nb-cli/<notebook-stem>/`
pub fn notebook_output_dir(notebook_path: &str) -> PathBuf {
    let stem = Path::new(notebook_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("notebook");
    std::env::temp_dir().join("nb-cli").join(stem)
}

/// Remove all nb-cli output directories from the temp dir
pub fn clean_output_dirs() -> Result<()> {
    let nb_cli_dir = std::env::temp_dir().join("nb-cli");
    if nb_cli_dir.exists() {
        fs::remove_dir_all(&nb_cli_dir).context("Failed to remove nb-cli output directory")?;
    }
    Ok(())
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
        assert!(!is_binary_mime_type("image/svg+xml"));
        assert!(!is_binary_mime_type("text/plain"));
        assert!(!is_binary_mime_type("application/json"));
    }

    #[test]
    fn test_get_cell_id_or_empty() {
        use nbformat::v4::{Cell, CellId, CellMetadata};
        use std::collections::HashMap;

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
    }

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension("image/png"), "png");
        assert_eq!(mime_to_extension("text/html"), "html");
        assert_eq!(mime_to_extension("application/pdf"), "pdf");
        assert_eq!(mime_to_extension("unknown/type"), "txt");
    }

    #[test]
    fn test_notebook_output_dir() {
        let dir = notebook_output_dir("/path/to/my_notebook.ipynb");
        assert!(dir.ends_with("nb-cli/my_notebook"));
    }

    #[test]
    fn test_output_header_builder() {
        let header = OutputHeaderBuilder::new("stream").name("stdout").build();
        assert!(header.starts_with("@@output "));
        let json_str = &header["@@output ".len()..];
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(json["output_type"], "stream");
        assert_eq!(json["name"], "stdout");
    }

    #[test]
    fn test_svg_output_rendered_as_text() {
        // SVG is stored as raw XML in Jupyter (not base64), so it must not
        // go through the binary/base64 decode path.
        let svg_xml = "<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"10\"/></svg>";
        let media_json = json!({ "image/svg+xml": svg_xml });
        let media: Media = serde_json::from_value(media_json).unwrap();

        let result =
            render_output_data("display_data", &media, None, None, json!({}), 10_000).unwrap();

        // Should contain the raw SVG markup inline, not error out
        assert!(result.contains(svg_xml), "SVG XML should appear inline");
        assert!(result.contains("```svg"), "SVG should be fenced as svg");
    }

    #[test]
    fn test_output_header_builder_with_error() {
        let header = OutputHeaderBuilder::new("error")
            .error("ValueError", "invalid value")
            .build();
        let json_str = &header["@@output ".len()..];
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(json["ename"], "ValueError");
        assert_eq!(json["evalue"], "invalid value");
    }
}
