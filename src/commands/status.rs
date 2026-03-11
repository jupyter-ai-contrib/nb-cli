use crate::config::Config;
use crate::execution::remote::client::JupyterClient;
use anyhow::{Context, Result};
use clap::Args;

#[derive(Args)]
pub struct StatusArgs {
    /// Validate the current connection
    #[arg(long)]
    pub validate: bool,
}

pub fn execute(args: StatusArgs) -> Result<()> {
    let config = Config::load().context("Failed to load config")?;

    match config.connection {
        None => {
            println!("Not connected to any Jupyter server");
            println!("\nTo connect, run:");
            println!("  nb connect");
            println!("  nb connect --server URL --token TOKEN");
            Ok(())
        }
        Some(conn) => {
            println!("✓ Connected to Jupyter server");
            println!("  Server: {}", conn.server_url);
            println!("  Connected at: {}", conn.connected_at);

            if let Some(working_dir) = &conn.working_dir {
                println!("  Working dir: {}", working_dir);
            }

            if let Some(last_validated) = conn.last_validated {
                println!("  Last validated: {}", last_validated);
            }

            if let Ok(config_path) = Config::config_path() {
                println!("  Config file: {}", config_path.display());
            }

            // Validate connection if requested
            if args.validate {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;

                println!("\n🔍 Validating connection...");
                let result = runtime.block_on(async {
                    let client = JupyterClient::new(conn.server_url.clone(), conn.token.clone())?;
                    client.test_connection().await
                });

                match result {
                    Ok(_) => {
                        println!("✓ Connection is valid");
                    }
                    Err(e) => {
                        println!("✗ Connection failed: {}", e);
                        println!("\nTry reconnecting with:");
                        println!("  nb connect");
                    }
                }
            }

            Ok(())
        }
    }
}
