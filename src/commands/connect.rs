use crate::config::{Config, JupyterConnection};
use crate::execution::remote::client::JupyterClient;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::Args;
use std::process::Command;

#[derive(Args)]
pub struct ConnectArgs {
    /// Server URL (manual specification, skips auto-detection)
    #[arg(long)]
    pub server: Option<String>,

    /// Authentication token (manual specification)
    #[arg(long)]
    pub token: Option<String>,

    /// Skip validation checks
    #[arg(long)]
    pub skip_validation: bool,
}

#[derive(Debug, Clone)]
struct DetectedServer {
    url: String,
    token: String,
    working_dir: String,
    valid: bool,
}

pub fn execute(args: ConnectArgs) -> Result<()> {
    // Create Tokio runtime for async operations
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(execute_async(args))
}

async fn execute_async(args: ConnectArgs) -> Result<()> {
    // Manual connection mode
    if let (Some(server_url), Some(token)) = (args.server, args.token) {
        return connect_manual(server_url, token, args.skip_validation).await;
    }

    // Auto-detection mode
    println!("🔍 Detecting running Jupyter servers...");

    let servers = detect_jupyter_servers().await?;

    if servers.is_empty() {
        bail!(
            "No running Jupyter servers found.\n\
            \nStart a Jupyter server with:\n  jupyter lab\n  jupyter notebook\n\
            \nOr connect manually with:\n  nb connect --server URL --token TOKEN"
        );
    }

    // Filter valid servers
    let valid_servers: Vec<_> = if args.skip_validation {
        servers
    } else {
        servers.into_iter().filter(|s| s.valid).collect()
    };

    if valid_servers.is_empty() {
        bail!("Found servers but none are valid or responding. Try --skip-validation to connect anyway.");
    }

    // Select server (interactive if multiple)
    let selected = if valid_servers.len() == 1 {
        println!("✓ Found 1 server");
        valid_servers[0].clone()
    } else {
        select_server_interactive(&valid_servers)?
    };

    // Create connection
    let connection = JupyterConnection {
        server_url: selected.url.clone(),
        token: selected.token.clone(),
        connected_at: Utc::now(),
        working_dir: Some(selected.working_dir.clone()),
        last_validated: Some(Utc::now()),
    };

    // Save config
    let mut config = Config::load().unwrap_or_default();
    config.version = "1".to_string();
    config.connection = Some(connection);

    let config_path = config.save()?;

    println!("\n✓ Connected to Jupyter server");
    println!("  Server: {}", selected.url);
    println!("  Working dir: {}", selected.working_dir);
    println!("  Config: {}", config_path.display());
    println!(
        "\nYou can now run commands without --server and --token flags:\n  nb cell execute notebook.ipynb --cell 0"
    );

    Ok(())
}

async fn connect_manual(server_url: String, token: String, skip_validation: bool) -> Result<()> {
    // Validate connection if requested
    if !skip_validation {
        println!("🔍 Validating connection...");
        let client = JupyterClient::new(server_url.clone(), token.clone())?;
        client
            .test_connection()
            .await
            .context("Failed to connect to server")?;
        println!("✓ Connection validated");
    }

    // Create connection
    let connection = JupyterConnection {
        server_url: server_url.clone(),
        token: token.clone(),
        connected_at: Utc::now(),
        working_dir: None,
        last_validated: if skip_validation {
            None
        } else {
            Some(Utc::now())
        },
    };

    // Save config
    let mut config = Config::load().unwrap_or_default();
    config.version = "1".to_string();
    config.connection = Some(connection);

    let config_path = config.save()?;

    println!("\n✓ Connected to Jupyter server");
    println!("  Server: {}", server_url);
    println!("  Config: {}", config_path.display());

    Ok(())
}

async fn detect_jupyter_servers() -> Result<Vec<DetectedServer>> {
    // Execute `jupyter server list`
    let output = Command::new("jupyter")
        .args(&["server", "list"])
        .output()
        .context("Failed to execute 'jupyter server list'. Is Jupyter installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("jupyter server list failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse output
    let mut servers = Vec::new();

    for line in stdout.lines() {
        // Format: http://localhost:8888/?token=abc123 :: /path/to/working/dir
        if let Some(parsed) = parse_server_line(line) {
            // Validate server
            let valid = validate_server(&parsed.url, &parsed.token).await;

            servers.push(DetectedServer {
                url: parsed.url,
                token: parsed.token,
                working_dir: parsed.working_dir,
                valid,
            });
        }
    }

    Ok(servers)
}

struct ParsedServer {
    url: String,
    token: String,
    working_dir: String,
}

fn parse_server_line(line: &str) -> Option<ParsedServer> {
    // Format: http://localhost:8888/?token=abc123 :: /path/to/working/dir
    let parts: Vec<&str> = line.split(" :: ").collect();
    if parts.len() != 2 {
        return None;
    }

    let url_part = parts[0].trim();
    let working_dir = parts[1].trim().to_string();

    // Parse URL and extract token
    let url = url::Url::parse(url_part).ok()?;

    // Extract token from query parameters
    let token = url
        .query_pairs()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.to_string())?;

    // Get base URL without query parameters
    let base_url = format!(
        "{}://{}{}",
        url.scheme(),
        url.host_str()?,
        if let Some(port) = url.port() {
            format!(":{}", port)
        } else {
            String::new()
        }
    );

    Some(ParsedServer {
        url: base_url,
        token,
        working_dir,
    })
}

async fn validate_server(server_url: &str, token: &str) -> bool {
    match JupyterClient::new(server_url.to_string(), token.to_string()) {
        Ok(client) => client.test_connection().await.is_ok(),
        Err(_) => false,
    }
}

fn select_server_interactive(servers: &[DetectedServer]) -> Result<DetectedServer> {
    println!("\nFound {} servers:", servers.len());

    let items: Vec<String> = servers
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {} ({})", i + 1, s.url, s.working_dir))
        .collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Select a server to connect to")
        .items(&items)
        .default(0)
        .interact()
        .context("Failed to get user input")?;

    Ok(servers[selection].clone())
}
