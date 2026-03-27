use crate::config::Config;
use crate::execution::remote::client::JupyterClient;
use anyhow::{Context, Result};
use clap::Args;

#[derive(Args)]
pub struct StatusArgs {
    /// Validate the current connection
    #[arg(long)]
    pub validate: bool,

    /// Output only the Python command prefix (for use in scripts)
    #[arg(long)]
    pub python: bool,

    /// Output status information as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn execute(args: StatusArgs) -> Result<()> {
    let config = Config::load().context("Failed to load config")?;

    match config.connection {
        None => {
            if args.json {
                println!("null");
            } else if args.python {
                // No connection, output nothing
            } else {
                println!("Not connected to any Jupyter server");
                println!("\nTo connect, run:");
                println!("  nb connect");
                println!("  nb connect --server URL --token TOKEN");
            }
            Ok(())
        }
        Some(conn) => {
            // Handle --validate flag first (takes priority, show simplified validation output)
            if args.validate {
                return validate_connection(&conn);
            }

            // Handle --python flag (just output the command prefix)
            if args.python {
                let python_prefix = get_python_prefix(&conn);
                println!("{}", python_prefix);
                return Ok(());
            }

            // Handle --json flag
            if args.json {
                output_json(&conn)?;
                return Ok(());
            }

            // Default human-readable output
            println!("✓ Connected to Jupyter server at {}", conn.server_url);
            println!("  Connected at: {}", conn.connected_at);

            if let Some(working_dir) = &conn.working_dir {
                println!("  Working directory: {}", working_dir);
            }

            if let Some(last_validated) = conn.last_validated {
                println!("  Last validated: {}", last_validated);
            }

            // Environment section
            if let Some(env_manager) = &conn.env_manager {
                println!();
                println!("  Environment: {}", env_manager);
                if let Some(project_root) = &conn.project_root {
                    println!("  Environment root: {}", project_root);
                }
                let python_prefix = get_python_prefix(&conn);
                if !python_prefix.is_empty() {
                    println!("  Python prefix: {}", python_prefix);
                }
            }

            Ok(())
        }
    }
}

fn validate_connection(conn: &crate::config::JupyterConnection) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    println!("Validating connection to {}", conn.server_url);
    let result = runtime.block_on(async {
        let client = JupyterClient::new(conn.server_url.clone(), conn.token.clone())?;
        client.test_connection().await
    });

    match result {
        Ok(_) => {
            println!("✓ Connection is valid");
            Ok(())
        }
        Err(e) => {
            println!("✗ Connection failed: {}", e);
            println!("\nTo connect again:");
            println!("1. Make sure a Jupyter server is running");
            println!("2. Use `nb connect --uv` if server is running in uv env");
            println!("3. Or, `nb connect --pixi` if server is running in pixi env");
            println!("4. Or, `nb connect`");
            Ok(())
        }
    }
}

fn get_python_prefix(conn: &crate::config::JupyterConnection) -> String {
    match conn.env_manager.as_deref() {
        Some("uv") => "uv run".to_string(),
        Some("pixi") => "pixi run".to_string(),
        None => String::new(),
        Some(other) => {
            eprintln!("Warning: Unknown environment manager '{}'", other);
            String::new()
        }
    }
}

fn output_json(conn: &crate::config::JupyterConnection) -> Result<()> {
    use serde_json::json;

    let python_prefix = get_python_prefix(conn);

    let status = json!({
        "server_url": conn.server_url,
        "connected_at": conn.connected_at,
        "working_directory": conn.working_dir,
        "last_validated": conn.last_validated,
        "environment": conn.env_manager,
        "environment_root": conn.project_root,
        "python_prefix": if python_prefix.is_empty() { None } else { Some(python_prefix) },
    });

    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}
