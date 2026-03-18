//! Check if a notebook has an active session in Jupyter

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Session {
    id: String,
    path: String,
    name: String,
    #[serde(rename = "type")]
    session_type: String,
}

/// Check if a notebook file has an active session
///
/// Returns true if the notebook is currently open in JupyterLab
#[allow(dead_code)]
pub async fn has_active_session(
    server_url: &str,
    token: &str,
    notebook_path: &str,
) -> Result<bool> {
    let url = format!("{}/api/sessions", server_url);

    let client = Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("token {}", token))
        .send()
        .await
        .context("Failed to call sessions API")?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Sessions API request failed with status {}: {}",
            status,
            error_text
        );
    }

    let sessions: Vec<Session> = response
        .json()
        .await
        .context("Failed to parse sessions API response")?;

    // Check if any session matches this notebook path
    // The path might be just the filename or a relative/absolute path
    let notebook_name = std::path::Path::new(notebook_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(notebook_path);

    let has_session = sessions.iter().any(|s| {
        // Match by exact path or by filename
        s.path == notebook_path || s.path == notebook_name || s.path.ends_with(notebook_name)
    });

    Ok(has_session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_check() {
        // This is a manual test - requires a running Jupyter server
        // Run with: cargo test test_session_check -- --nocapture --ignored

        let server_url = "http://localhost:8888";
        let token = "your-token";
        let notebook_path = "test.ipynb";

        match has_active_session(server_url, token, notebook_path).await {
            Ok(has_session) => {
                println!("Has active session: {}", has_session);
            }
            Err(e) => {
                println!("Error checking session: {}", e);
            }
        }
    }
}
