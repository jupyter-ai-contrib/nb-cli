use crate::config::Config;
use anyhow::{Context, Result};
use clap::Args;

#[derive(Args)]
pub struct DisconnectArgs {}

pub fn execute(_args: DisconnectArgs) -> Result<()> {
    let mut config = Config::load().context("Failed to load config")?;

    if config.connection.is_none() {
        println!("Not connected to any Jupyter server");
        return Ok(());
    }

    // Clear connection
    config.connection = None;
    config.save()?;

    println!("✓ Disconnected from Jupyter server");
    println!("\nTo connect again, run:");
    println!("  nb connect");

    Ok(())
}
