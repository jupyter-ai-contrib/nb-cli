/// Helper module for test utilities
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

// ==================== AI-OPTIMIZED MARKDOWN PARSING ====================

/// A parsed sentinel line from AI-Optimized Markdown output (@@notebook, @@cell, @@output)
#[derive(Debug, Clone)]
pub struct Sentinel {
    /// Sentinel type: "notebook", "cell", or "output"
    pub kind: String,
    /// Parsed JSON metadata following the sentinel marker
    pub metadata: Value,
}

impl Sentinel {
    /// Get a string field from the metadata
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.metadata.get(key)?.as_str()
    }

    /// Get an integer field from the metadata
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.metadata.get(key)?.as_i64()
    }
}

/// Parse a single line that starts with @@ into a Sentinel
pub fn parse_sentinel(line: &str) -> Option<Sentinel> {
    let line = line.trim();
    if !line.starts_with("@@") {
        return None;
    }
    let rest = &line[2..];
    let space_idx = rest.find(' ')?;
    let kind = rest[..space_idx].to_string();
    let json_str = &rest[space_idx + 1..];
    let metadata: Value = serde_json::from_str(json_str).ok()?;
    Some(Sentinel { kind, metadata })
}

/// Parse all sentinel lines from AI-Optimized Markdown output
pub fn parse_sentinels(output: &str) -> Vec<Sentinel> {
    output.lines().filter_map(parse_sentinel).collect()
}

/// Extract only @@cell sentinels from output
pub fn parse_cells(output: &str) -> Vec<Sentinel> {
    parse_sentinels(output)
        .into_iter()
        .filter(|s| s.kind == "cell")
        .collect()
}

/// Extract only @@output sentinels from output
pub fn parse_outputs(output: &str) -> Vec<Sentinel> {
    parse_sentinels(output)
        .into_iter()
        .filter(|s| s.kind == "output")
        .collect()
}

/// Extract the @@notebook sentinel from output
pub fn parse_notebook_header(output: &str) -> Option<Sentinel> {
    parse_sentinels(output)
        .into_iter()
        .find(|s| s.kind == "notebook")
}

static VENV_PATH: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Check if uv is installed
pub fn has_uv() -> bool {
    Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if Python 3 is available
pub fn has_python3() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Setup test virtual environment with execution dependencies
/// Returns the path to the venv if successful
pub fn setup_execution_venv() -> Option<PathBuf> {
    let mutex = VENV_PATH.get_or_init(|| {
        let venv_path = initialize_venv();
        Mutex::new(venv_path)
    });

    mutex.lock().unwrap().clone()
}

fn initialize_venv() -> Option<PathBuf> {
    if !has_uv() || !has_python3() {
        eprintln!("⚠️  Skipping execution test setup: uv or python3 not available");
        return None;
    }

    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let venv_path = test_dir.join(".test-venv");

    // Create venv if it doesn't exist
    if !venv_path.exists() {
        eprintln!("📦 Creating test venv with uv...");
        let status = Command::new("uv")
            .args(["venv", venv_path.to_str().unwrap()])
            .status();

        if status.map(|s| !s.success()).unwrap_or(true) {
            eprintln!("⚠️  Failed to create test venv");
            return None;
        }
    }

    // Install ipykernel for Python kernel
    eprintln!("📦 Installing ipykernel...");
    let status = Command::new("uv")
        .args([
            "pip",
            "install",
            "--python",
            venv_path.to_str().unwrap(),
            "ipykernel",
        ])
        .status();

    if status.map(|s| s.success()).unwrap_or(false) {
        eprintln!("✅ Test venv ready at: {}", venv_path.display());
        Some(venv_path)
    } else {
        eprintln!("⚠️  Failed to install dependencies in test venv");
        None
    }
}

/// Set environment to use test venv for execution
pub fn setup_venv_environment() -> Option<String> {
    let mutex = VENV_PATH.get()?;
    let venv_path = mutex.lock().unwrap();
    let venv_path = venv_path.as_ref()?;

    let bin_path = if cfg!(windows) {
        venv_path.join("Scripts")
    } else {
        venv_path.join("bin")
    };

    // Prepend venv bin to PATH
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_path.display(), current_path);

    Some(new_path)
}
