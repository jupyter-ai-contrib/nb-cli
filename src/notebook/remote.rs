//! Notebook I/O via the Jupyter Server Contents API (the "remote" leg of
//! notebook persistence, as opposed to `notebook::local`'s filesystem I/O).

use anyhow::{bail, Context, Result};

use crate::commands::common::normalize_notebook_path;
use crate::config::Config;
use crate::execution::server::client::JupyterClient;
use crate::execution::types::ExecutionMode;

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

/// Read a notebook from a Jupyter Server via the Contents API.
///
/// `server_path` must be the notebook path relative to the server root
/// (as computed by `notebook_path_for_server`).
pub async fn read_notebook_remote(
    server_url: &str,
    token: &str,
    server_path: &str,
) -> Result<nbformat::v4::Notebook> {
    let client = JupyterClient::new(server_url.to_string(), token.to_string())?;
    client
        .get_notebook(server_path)
        .await
        .with_context(|| format!("Failed to read notebook '{}' from server", server_path))
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

    let client = JupyterClient::new(server_url, token)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notebook_path_for_server_fallbacks() {
        // No server root → input returned unchanged
        assert_eq!(
            notebook_path_for_server("mynotebook.ipynb", None),
            "mynotebook.ipynb"
        );
        // Non-existent file → canonicalize fails → input returned unchanged
        assert_eq!(
            notebook_path_for_server("/nonexistent/path/nb.ipynb", Some("/tmp")),
            "/nonexistent/path/nb.ipynb"
        );
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
