#![allow(dead_code)]
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

// ==================== COMMAND RESULT ====================

pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl CommandResult {
    pub fn assert_success(self) -> Self {
        if !self.success {
            panic!(
                "Command failed:\nStderr: {}\nStdout: {}",
                self.stderr, self.stdout
            );
        }
        self
    }

    pub fn assert_failure(self) -> Self {
        if self.success {
            panic!(
                "Expected command to fail but it succeeded:\nStdout: {}\nStderr: {}",
                self.stdout, self.stderr
            );
        }
        self
    }

    pub fn json_value(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).expect("Failed to parse JSON output")
    }

    pub fn contains(&self, text: &str) -> bool {
        self.stdout.contains(text) || self.stderr.contains(text)
    }
}

// ==================== AI-OPTIMIZED MARKDOWN PARSING ====================

/// A parsed sentinel line from AI-Optimized Markdown output (@@notebook, @@cell, @@output)
#[derive(Debug, Clone)]
pub struct Sentinel {
    pub kind: String,
    /// Parsed JSON metadata following the sentinel marker
    #[allow(dead_code)]
    pub metadata: Value,
}

impl Sentinel {
    /// Get a string field from the metadata
    #[allow(dead_code)]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.metadata.get(key)?.as_str()
    }

    /// Get an integer field from the metadata
    #[allow(dead_code)]
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
    let metadata: Value = serde_json::from_str(&rest[space_idx + 1..]).ok()?;
    Some(Sentinel { kind, metadata })
}

/// Parse all sentinel lines from AI-Optimized Markdown output
pub fn parse_sentinels(output: &str) -> Vec<Sentinel> {
    output.lines().filter_map(parse_sentinel).collect()
}

/// Extract only @@cell sentinels from output
#[allow(dead_code)]
pub fn parse_cells(output: &str) -> Vec<Sentinel> {
    parse_sentinels(output)
        .into_iter()
        .filter(|s| s.kind == "cell")
        .collect()
}

/// Extract only @@output sentinels from output
#[allow(dead_code)]
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

// ==================== BACKEND SELECTION ====================

/// Connect-mode backend selector, read from `NB_TEST_BACKEND`. Defaults to
/// unset (empty string), which maps to `.test-venv` — the local-execution venv
/// shared by integration_local_mode and integration_execution (ipykernel only,
/// no Jupyter Server extension). Connect-mode tests set this explicitly to one of:
/// `jsd`, `jupyter-collaboration`, or `none`. Each backend gets its own venv
/// directory since jupyter-collaboration and jupyter-server-documents must never
/// be installed together (competing collaborative-editing extensions).
#[allow(dead_code)]
pub fn test_backend() -> String {
    std::env::var("NB_TEST_BACKEND").unwrap_or_default()
}

/// Venv directory name for the currently selected backend (see [`test_backend`]).
#[allow(dead_code)]
fn venv_dir_name() -> &'static str {
    match test_backend().as_str() {
        "jsd" | "jupyter-server-documents" => ".test-venv-jsd",
        "jupyter-collaboration" | "collab" => ".test-venv-collab",
        "none" | "plain" => ".test-venv-plain",
        _ => ".test-venv",
    }
}

/// Check if uv is installed
#[allow(dead_code)]
pub fn has_uv() -> bool {
    Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ==================== FIXTURE HELPERS ====================

/// Copy a fixture file from tests/fixtures/ to a destination path.
pub fn copy_fixture(fixture_name: &str, dest_path: &std::path::Path) {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_name);
    std::fs::copy(&fixture_path, dest_path)
        .unwrap_or_else(|_| panic!("Failed to copy fixture {}", fixture_name));
}

/// Copy an entire fixture directory (recursively) to a destination path.
pub fn copy_fixture_dir(fixture_subdir: &str, dest_path: &std::path::Path) {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture_subdir);

    fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if ty.is_dir() {
                copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }

    copy_dir_recursive(&fixture_path, dest_path)
        .unwrap_or_else(|_| panic!("Failed to copy fixture directory {}", fixture_subdir));
}

// ==================== VENV HELPERS ====================

static VENV_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Return the path to the pre-built test venv (created by setup_test_env.sh).
/// Returns None and prints a hint if the venv doesn't exist.
#[allow(dead_code)]
pub fn setup_execution_venv() -> Option<PathBuf> {
    VENV_PATH.get_or_init(find_venv).clone()
}

fn find_venv() -> Option<PathBuf> {
    let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    let venv_path = test_dir.join(venv_dir_name());

    let python_bin = if cfg!(windows) {
        venv_path.join("Scripts").join("python.exe")
    } else {
        venv_path.join("bin").join("python")
    };

    if python_bin.exists() {
        Some(venv_path)
    } else {
        eprintln!(
            "⚠️  Test venv not found at {}. Run ./tests/setup_test_env.sh first.",
            venv_path.display()
        );
        None
    }
}

/// Build a PATH string that prepends the test venv's bin directory.
#[allow(dead_code)]
pub fn setup_venv_environment() -> Option<String> {
    let venv_path = VENV_PATH.get()?.as_ref()?;

    let bin_path = if cfg!(windows) {
        venv_path.join("Scripts")
    } else {
        venv_path.join("bin")
    };

    let current_path = std::env::var("PATH").unwrap_or_default();
    Some(format!("{}:{}", bin_path.display(), current_path))
}
