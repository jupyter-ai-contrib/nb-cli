use crate::commands::env_manager::{EnvConfig, EnvManager};
use crate::config::{Config, JupyterConnection};
use crate::execution::remote::client::JupyterClient;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::Args;

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

    /// Use uv to run jupyter commands
    #[arg(long, conflicts_with = "pixi")]
    pub uv: bool,

    /// Use pixi to run jupyter commands
    #[arg(long, conflicts_with = "uv")]
    pub pixi: bool,
}

#[derive(Debug, Clone)]
struct DetectedServer {
    url: String,
    token: String,
    working_dir: String,
    valid: bool,
}

#[derive(Debug, serde::Deserialize)]
struct JupyterServerInfo {
    url: String,
    token: String,
    root_dir: String,
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

    // Create environment configuration
    let env_config = EnvConfig::from_flags(args.uv, args.pixi)?;

    // Auto-detection mode
    match env_config.manager {
        EnvManager::Direct => {
            println!("🔍 Detecting running Jupyter servers...");
        }
        EnvManager::Uv => {
            println!("🔍 Detecting running Jupyter servers using uv...");
            if let Some(root) = &env_config.project_root {
                println!("   Project root: {}", root.display());
            }
        }
        EnvManager::Pixi => {
            println!("🔍 Detecting running Jupyter servers using pixi...");
            if let Some(root) = &env_config.project_root {
                println!("   Project root: {}", root.display());
            }
        }
    }

    let servers = detect_jupyter_servers(&env_config).await?;

    if servers.is_empty() {
        let start_command = match env_config.manager {
            EnvManager::Direct => "jupyter lab",
            EnvManager::Uv => "uv run jupyter lab",
            EnvManager::Pixi => "pixi run jupyter lab",
        };

        bail!(
            "No running Jupyter servers found.\n\
            \nStart a Jupyter server with:\n  {}\n\
            \nOr connect manually with:\n  nb connect --server URL --token TOKEN",
            start_command
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
        env_manager: match env_config.manager {
            EnvManager::Direct => None,
            EnvManager::Uv => Some("uv".to_string()),
            EnvManager::Pixi => Some("pixi".to_string()),
        },
        project_root: env_config
            .project_root
            .as_ref()
            .map(|p| p.display().to_string()),
    };

    // Save config
    let mut config = Config::load().unwrap_or_default();
    config.version = "1".to_string();
    config.connection = Some(connection);

    let _config_path = config.save()?;

    println!("\n✓ Connected to Jupyter server at {}", selected.url);
    println!("  Working directory: {}", selected.working_dir);

    // Show environment info if using uv or pixi
    match env_config.manager {
        EnvManager::Uv => {
            println!("\n  Environment: uv");
            if let Some(root) = &env_config.project_root {
                println!("  Environment root: {}", root.display());
            }
            println!("  Python prefix: uv run");
        }
        EnvManager::Pixi => {
            println!("\n  Environment: pixi");
            if let Some(root) = &env_config.project_root {
                println!("  Environment root: {}", root.display());
            }
            println!("  Python prefix: pixi run");
        }
        EnvManager::Direct => {}
    }

    println!(
        "\nYou can now run commands without --server and --token flags:\n  nb execute notebook.ipynb"
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
        env_manager: None,
        project_root: None,
    };

    // Save config
    let mut config = Config::load().unwrap_or_default();
    config.version = "1".to_string();
    config.connection = Some(connection);

    let _config_path = config.save()?;

    println!("\n✓ Connected to Jupyter server at {}", server_url);

    Ok(())
}

async fn detect_jupyter_servers(env_config: &EnvConfig) -> Result<Vec<DetectedServer>> {
    // Execute `jupyter server list --json`
    let mut cmd = env_config.build_jupyter_command(&["server", "list", "--json"]);
    let output = cmd
        .output()
        .with_context(|| {
            match env_config.manager {
                EnvManager::Direct => {
                    "Failed to execute 'jupyter server list'. Is Jupyter installed?".to_string()
                }
                EnvManager::Uv => {
                    format!(
                        "Failed to execute 'uv run jupyter server list'. Is uv installed and is jupyter in your project?\n\
                        Project root: {}",
                        env_config.project_root.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".to_string())
                    )
                }
                EnvManager::Pixi => {
                    format!(
                        "Failed to execute 'pixi run jupyter server list'. Is pixi installed and is jupyter in your project?\n\
                        Project root: {}",
                        env_config.project_root.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".to_string())
                    )
                }
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("jupyter server list failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON output (each line is a separate JSON object)
    let mut servers = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try to parse as JSON
        match serde_json::from_str::<JupyterServerInfo>(line) {
            Ok(server_info) => {
                // The URL from JSON already includes the base path
                let url = server_info.url.trim_end_matches('/').to_string();

                // Validate server
                let valid = validate_server(&url, &server_info.token).await;

                servers.push(DetectedServer {
                    url,
                    token: server_info.token,
                    working_dir: server_info.root_dir,
                    valid,
                });
            }
            Err(e) => {
                // Skip lines that aren't valid JSON (e.g., informational messages)
                eprintln!(
                    "Warning: Failed to parse line as JSON: {} (error: {})",
                    line, e
                );
            }
        }
    }

    Ok(servers)
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
