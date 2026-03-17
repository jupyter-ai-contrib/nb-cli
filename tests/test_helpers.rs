/// Helper module for test utilities
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

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
