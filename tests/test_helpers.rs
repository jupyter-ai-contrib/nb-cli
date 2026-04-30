#![allow(dead_code)]
use serde_json::Value;
use std::path::PathBuf;
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
    pub metadata: Value,
}

impl Sentinel {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.metadata.get(key)?.as_str()
    }

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

// ==================== VENV HELPERS ====================

static VENV_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Return the path to the pre-built test venv (created by setup_test_env.sh).
/// Returns None and prints a hint if the venv doesn't exist.
#[allow(dead_code)]
pub fn setup_execution_venv() -> Option<PathBuf> {
    VENV_PATH.get_or_init(find_venv).clone()
}

fn find_venv() -> Option<PathBuf> {
    let venv_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(".test-venv");

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
