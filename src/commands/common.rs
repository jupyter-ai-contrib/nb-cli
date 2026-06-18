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

/// Read-modify-save a notebook via the Contents API.
/// Handles: extract credentials, normalize path, fetch notebook, call `mutate`, save back.
/// The closure receives (&mut Notebook, &normalized_file_path) and returns a
/// value that is handed back to the caller only after the save succeeds, so
/// callers report results only for persisted mutations.
pub async fn with_contents_api<F, T>(file: &str, mode: &ExecutionMode, mutate: F) -> Result<T>
where
    F: FnOnce(&mut nbformat::v4::Notebook, &str) -> Result<T>,
{
    let (server_url, token) = match mode {
        ExecutionMode::Remote { server_url, token } => (server_url.clone(), token.clone()),
        _ => bail!("Expected remote execution mode"),
    };

    let file_path = normalize_notebook_path(file);
    let server_root = resolve_server_root();
    let notebook_server_path = notebook_path_for_server(&file_path, server_root.as_deref());

    let client = crate::execution::remote::client::JupyterClient::new(server_url, token)?;
    let mut notebook = client
        .get_notebook(&notebook_server_path)
        .await
        .context("Failed to read notebook from server")?;

    let result = mutate(&mut notebook, &file_path)?;

    client
        .save_notebook(&notebook_server_path, &notebook)
        .await
        .context("Failed to save notebook to server")?;

    Ok(result)
}

/// Get the Jupyter server root directory from saved connection config.
/// Returns None for manual connections or when no config exists.
pub fn resolve_server_root() -> Option<String> {
    Config::load()
        .ok()
        .and_then(|c| c.connection)
        .and_then(|c| c.working_dir)
}

/// Read a notebook from a Jupyter Server via the Contents API.
///
/// `server_path` must be the notebook path relative to the server root
/// (as computed by `notebook_path_for_server`).
pub async fn read_notebook_remote(
    server_url: &str,
    token: &str,
    server_path: &str,
) -> Result<nbformat::v4::Notebook> {
    let client = crate::execution::remote::client::JupyterClient::new(
        server_url.to_string(),
        token.to_string(),
    )?;
    client
        .get_notebook(server_path)
        .await
        .with_context(|| format!("Failed to read notebook '{}' from server", server_path))
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

    struct ContentsStub {
        url: String,
        put_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        put_bodies: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    /// Minimal Contents API stub: serves GET with a valid one-cell notebook and
    /// answers PUT with the given status, recording every PUT body.
    /// Note: with_contents_api also calls Config::load() from the process cwd,
    /// which may pick up a developer-local ./.jupyter/cli.json and change the
    /// request path; the tests are insulated because this stub ignores the
    /// request path entirely.
    async fn contents_api_stub(put_status: u16) -> ContentsStub {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let put_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let put_bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let put_count_srv = std::sync::Arc::clone(&put_count);
        let put_bodies_srv = std::sync::Arc::clone(&put_bodies);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                // Read until end of headers, then drain any body by content-length
                let body_start = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break None;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(pos + 4);
                    }
                };
                let Some(body_start) = body_start else {
                    continue;
                };
                let head = String::from_utf8_lossy(&buf[..body_start]).to_string();
                let content_length: usize = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while buf.len() - body_start < content_length {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let response = if head.starts_with("GET") {
                    let nb = r#"{"content":{"cells":[{"cell_type":"code","execution_count":null,"id":"c1","metadata":{},"outputs":[],"source":["original"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}}"#;
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        nb.len(),
                        nb
                    )
                } else {
                    put_count_srv.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let body = String::from_utf8_lossy(&buf[body_start..]).to_string();
                    put_bodies_srv.lock().unwrap().push(body);
                    format!(
                        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                        put_status
                    )
                };
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        ContentsStub {
            url: format!("http://{}", addr),
            put_count,
            put_bodies,
        }
    }

    fn remote_mode(server_url: String) -> ExecutionMode {
        ExecutionMode::Remote {
            server_url,
            token: "t".to_string(),
        }
    }

    #[tokio::test]
    async fn with_contents_api_saves_mutated_notebook_then_returns_result() {
        let stub = contents_api_stub(200).await;
        let result =
            super::with_contents_api("test.ipynb", &remote_mode(stub.url), |notebook, _| {
                match &mut notebook.cells[0] {
                    nbformat::v4::Cell::Code { source, .. } => {
                        *source = vec!["mutated".to_string()]
                    }
                    _ => panic!("expected code cell"),
                }
                Ok(42)
            })
            .await;
        assert_eq!(result.unwrap(), 42);
        let bodies = stub.put_bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert!(
            bodies[0].contains("mutated") && !bodies[0].contains("original"),
            "PUT body must contain the mutated notebook: {}",
            bodies[0]
        );
    }

    #[tokio::test]
    async fn with_contents_api_skips_save_when_mutation_fails() {
        let stub = contents_api_stub(200).await;
        let result = super::with_contents_api(
            "test.ipynb",
            &remote_mode(stub.url),
            |_, _| -> anyhow::Result<i32> { anyhow::bail!("mutation rejected") },
        )
        .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutation rejected"));
        assert_eq!(
            stub.put_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a failed mutation must not be saved"
        );
    }

    #[tokio::test]
    async fn with_contents_api_withholds_result_when_save_fails() {
        let stub = contents_api_stub(500).await;
        let mutated = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mutated_clone = std::sync::Arc::clone(&mutated);
        let result = super::with_contents_api("test.ipynb", &remote_mode(stub.url), move |_, _| {
            mutated_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(42)
        })
        .await;
        // The mutation ran, but a failed save must surface as an error so the
        // caller never reports success for an unpersisted change.
        assert!(mutated.load(std::sync::atomic::Ordering::SeqCst));
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to save notebook to server"),
            "unexpected error: {}",
            err
        );
    }
}
