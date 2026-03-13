use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use std::env;
use std::fs;
use std::path::PathBuf;

const REPO: &str = "jupyter-ai-contrib/nb-cli";

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Subcommand)]
pub enum SelfCommands {
    /// Show the installed version
    Version,
    /// Update to the latest release version
    Update,
}

pub fn execute(command: SelfCommands) -> Result<()> {
    match command {
        SelfCommands::Version => execute_version(),
        SelfCommands::Update => execute_update(),
    }
}

fn execute_version() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!("nb-cli version {}", version);
    Ok(())
}

fn execute_update() -> Result<()> {
    // Use tokio runtime since we're in a sync context
    let rt = tokio::runtime::Runtime::new()
        .context("Failed to create async runtime")?;

    rt.block_on(async {
        execute_update_async().await
    })
}

async fn execute_update_async() -> Result<()> {
    println!("🔍 Checking for updates...");

    // Get current version
    let current_version = env!("CARGO_PKG_VERSION");

    // Fetch latest release version
    let latest_version = fetch_latest_version()
        .await
        .context("Failed to fetch latest version from GitHub")?;

    println!("📦 Current version: v{}", current_version);
    println!("📦 Latest version:  {}", latest_version);

    // Compare versions (strip 'v' prefix if present)
    let latest_clean = latest_version.trim_start_matches('v');
    if current_version == latest_clean {
        println!("✅ You are already on the latest version!");
        return Ok(());
    }

    // Get current binary path
    let current_exe = env::current_exe()
        .context("Failed to get current executable path")?;

    println!("📍 Current installation: {}", current_exe.display());

    // Detect platform
    let binary_name = detect_platform_binary()?;

    // Download URL
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        REPO, latest_version, binary_name
    );

    println!("⬇️  Downloading from GitHub...");

    // Create temp file for download
    let temp_dir = env::temp_dir();
    let temp_file = temp_dir.join(format!("nb-update-{}", std::process::id()));

    // Download the new binary
    download_binary(&url, &temp_file)
        .await
        .context("Failed to download new version")?;

    // Make it executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_file)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_file, perms)?;
    }

    // Replace current binary
    println!("🔄 Installing update...");

    // On Unix, we need to move the old binary out of the way first
    #[cfg(unix)]
    {
        let backup = current_exe.with_extension("old");
        if let Err(e) = fs::rename(&current_exe, &backup) {
            println!("⚠️  Warning: Could not create backup: {}", e);
        }

        fs::copy(&temp_file, &current_exe)
            .context("Failed to replace binary")?;

        // Remove backup
        let _ = fs::remove_file(&backup);
    }

    #[cfg(windows)]
    {
        fs::copy(&temp_file, &current_exe)
            .context("Failed to replace binary")?;
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_file);

    println!("✅ Successfully updated to {}!", latest_version);
    println!("🎉 Run 'nb self version' to verify the update");

    Ok(())
}

async fn fetch_latest_version() -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);

    let client = reqwest::Client::builder()
        .user_agent("nb-cli") // GitHub API requires a user agent
        .build()?;

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to fetch release info from GitHub")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API returned error: {}",
            response.status()
        ));
    }

    let release: GithubRelease = response
        .json()
        .await
        .context("Failed to parse GitHub API response")?;

    Ok(release.tag_name)
}

fn get_platform_binary_name(os: &str, arch: &str) -> Result<String> {
    let binary = match (os, arch) {
        ("macos", "aarch64") => "nb-macos-arm64",
        ("macos", "x86_64") => "nb-macos-amd64",
        ("linux", "x86_64") => "nb-linux-amd64",
        ("linux", "aarch64") => "nb-linux-arm64",
        ("windows", "x86_64") => "nb-windows-amd64.exe",
        _ => {
            return Err(anyhow!(
                "Unsupported platform: {} ({}). Please install from source: cargo install nb-cli",
                os,
                arch
            ));
        }
    };

    Ok(binary.to_string())
}

fn detect_platform_binary() -> Result<String> {
    get_platform_binary_name(env::consts::OS, env::consts::ARCH)
}

async fn download_binary(url: &str, dest: &PathBuf) -> Result<()> {
    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to download binary")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "Failed to download binary: HTTP {}. The binary for your platform may not be available yet.",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read download response")?;

    fs::write(dest, bytes)
        .context("Failed to write binary to disk")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_detection() {
        // Test that we can detect the current platform
        let result = detect_platform_binary();
        assert!(result.is_ok());
    }
}
