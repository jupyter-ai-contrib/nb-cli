//! Tests for `nb status` and `nb disconnect` against pre-written config files.
//!
//! These tests do NOT require a live Jupyter server — they exercise the config
//! read/write code path only. Each test gets its own TempDir so there is no
//! shared state and tests can run in parallel (default Cargo test runner).

mod test_helpers;

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use test_helpers::CommandResult;

// ==================== TEST HELPERS ====================

struct ConfigTestEnv {
    temp_dir: TempDir,
    binary_path: PathBuf,
}

impl ConfigTestEnv {
    fn new() -> Self {
        ConfigTestEnv {
            temp_dir: TempDir::new().expect("Failed to create temp dir"),
            binary_path: env!("CARGO_BIN_EXE_nb").into(),
        }
    }

    /// Write arbitrary bytes to .jupyter/cli.json (for testing malformed input).
    fn write_raw_config(&self, content: &str) {
        let config_dir = self.temp_dir.path().join(".jupyter");
        fs::create_dir_all(&config_dir).expect("Failed to create .jupyter dir");
        fs::write(config_dir.join("cli.json"), content).expect("Failed to write raw config");
    }

    /// Write a minimal valid .jupyter/cli.json with the given connection.
    fn write_connection(&self, server_url: &str, token: &str) {
        let config_dir = self.temp_dir.path().join(".jupyter");
        fs::create_dir_all(&config_dir).expect("Failed to create .jupyter dir");
        let config_path = config_dir.join("cli.json");
        let json = serde_json::json!({
            "version": "",
            "connection": {
                "server_url": server_url,
                "token": token,
                "connected_at": "2024-01-01T00:00:00Z",
                "working_dir": null,
                "last_validated": null
            }
        });
        fs::write(&config_path, serde_json::to_string_pretty(&json).unwrap())
            .expect("Failed to write config");
    }

    /// Write a connection with an env_manager field set.
    fn write_connection_with_env(&self, server_url: &str, token: &str, env_manager: &str) {
        let config_dir = self.temp_dir.path().join(".jupyter");
        fs::create_dir_all(&config_dir).expect("Failed to create .jupyter dir");
        let config_path = config_dir.join("cli.json");
        let json = serde_json::json!({
            "version": "",
            "connection": {
                "server_url": server_url,
                "token": token,
                "connected_at": "2024-01-01T00:00:00Z",
                "working_dir": null,
                "last_validated": null,
                "env_manager": env_manager
            }
        });
        fs::write(&config_path, serde_json::to_string_pretty(&json).unwrap())
            .expect("Failed to write config");
    }

    /// Read and parse .jupyter/cli.json; returns None if it does not exist.
    fn read_config(&self) -> Option<Value> {
        let config_path = self.temp_dir.path().join(".jupyter").join("cli.json");
        if !config_path.exists() {
            return None;
        }
        let content = fs::read_to_string(&config_path).expect("Failed to read config");
        Some(serde_json::from_str(&content).expect("Failed to parse config"))
    }

    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.binary_path)
            .args(args)
            .current_dir(self.temp_dir.path())
            .output()
            .expect("Failed to execute nb command");
        CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
    }
}

// ==================== STATUS TESTS ====================

#[test]
fn test_status_when_not_connected() {
    let env = ConfigTestEnv::new();
    let result = env.run(&["status"]).assert_success();
    assert!(
        result
            .stdout
            .contains("Not connected to any Jupyter server"),
        "Expected 'Not connected' message\nStdout: {}",
        result.stdout
    );
}

#[test]
fn test_status_human_readable_when_connected() {
    let env = ConfigTestEnv::new();
    env.write_connection("http://127.0.0.1:8888", "testtoken");
    let result = env.run(&["status"]).assert_success();
    assert!(
        result.stdout.contains("http://127.0.0.1:8888"),
        "Expected server URL in status output\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("✓ Connected"),
        "Expected connected indicator\nStdout: {}",
        result.stdout
    );
}

#[test]
fn test_status_json_when_not_connected() {
    let env = ConfigTestEnv::new();
    let result = env.run(&["status", "--json"]).assert_success();
    assert_eq!(
        result.stdout.trim(),
        "null",
        "Expected 'null' JSON when not connected\nStdout: {}",
        result.stdout
    );
}

#[test]
fn test_status_json_when_connected() {
    let env = ConfigTestEnv::new();
    env.write_connection("http://127.0.0.1:9999", "mytoken");
    let result = env.run(&["status", "--json"]).assert_success();
    let json: Value =
        serde_json::from_str(&result.stdout).expect("status --json did not produce valid JSON");
    assert_eq!(
        json["server_url"].as_str(),
        Some("http://127.0.0.1:9999"),
        "JSON missing server_url"
    );
    assert!(
        json["connected_at"].is_string(),
        "JSON missing connected_at"
    );
    assert!(
        json.get("working_directory").is_some(),
        "JSON missing working_directory key"
    );
}

#[test]
fn test_status_python_no_env_manager() {
    let env = ConfigTestEnv::new();
    env.write_connection("http://127.0.0.1:8888", "tok");
    let result = env.run(&["status", "--python"]).assert_success();
    assert!(
        result.stdout.trim().is_empty(),
        "Expected empty output for --python with no env_manager\nStdout: {:?}",
        result.stdout
    );
}

#[test]
fn test_status_python_uv() {
    let env = ConfigTestEnv::new();
    env.write_connection_with_env("http://127.0.0.1:8888", "tok", "uv");
    let result = env.run(&["status", "--python"]).assert_success();
    assert_eq!(
        result.stdout.trim(),
        "uv run",
        "Expected 'uv run' for uv env_manager\nStdout: {:?}",
        result.stdout
    );
}

#[test]
fn test_status_python_pixi() {
    let env = ConfigTestEnv::new();
    env.write_connection_with_env("http://127.0.0.1:8888", "tok", "pixi");
    let result = env.run(&["status", "--python"]).assert_success();
    assert_eq!(
        result.stdout.trim(),
        "pixi run",
        "Expected 'pixi run' for pixi env_manager\nStdout: {:?}",
        result.stdout
    );
}

// ==================== DISCONNECT TESTS ====================

#[test]
fn test_disconnect_when_connected() {
    let env = ConfigTestEnv::new();
    env.write_connection("http://127.0.0.1:8888", "tok");

    let result = env.run(&["disconnect"]).assert_success();
    assert!(
        result.stdout.contains("✓ Disconnected"),
        "Expected disconnected confirmation\nStdout: {}",
        result.stdout
    );

    // Config should still exist but connection should be null
    let config = env
        .read_config()
        .expect("Config file should still exist after disconnect");
    assert!(
        config["connection"].is_null(),
        "connection should be null after disconnect\nConfig: {}",
        config
    );
}

#[test]
fn test_disconnect_when_not_connected() {
    let env = ConfigTestEnv::new();
    let result = env.run(&["disconnect"]).assert_success();
    assert!(
        result
            .stdout
            .contains("Not connected to any Jupyter server"),
        "Expected not-connected message\nStdout: {}",
        result.stdout
    );
}

#[test]
fn test_status_after_disconnect() {
    let env = ConfigTestEnv::new();
    env.write_connection("http://127.0.0.1:8888", "tok");

    env.run(&["disconnect"]).assert_success();
    let result = env.run(&["status"]).assert_success();

    assert!(
        result
            .stdout
            .contains("Not connected to any Jupyter server"),
        "Expected 'Not connected' after disconnect\nStdout: {}",
        result.stdout
    );
}

// ==================== CONFIG ERROR TESTS ====================

#[test]
fn test_status_with_malformed_config_json() {
    let env = ConfigTestEnv::new();
    env.write_raw_config("{{{ not valid json");

    // Must exit non-zero with a parse error — must not panic.
    let result = env.run(&["status"]);
    assert!(
        !result.success,
        "malformed config must cause a non-zero exit\nStdout: {}\nStderr: {}",
        result.stdout, result.stderr
    );
    assert!(
        !result.stderr.is_empty(),
        "malformed config must print an error message to stderr\nStderr: {}",
        result.stderr
    );
}

#[test]
fn test_status_with_incomplete_connection_object() {
    let env = ConfigTestEnv::new();
    // Missing required `token` field — serde will fail to deserialize JupyterConnection.
    env.write_raw_config(r#"{"version":"","connection":{"server_url":"http://127.0.0.1:8888"}}"#);

    let result = env.run(&["status"]);
    assert!(
        !result.success,
        "incomplete connection object must cause a non-zero exit\nStdout: {}\nStderr: {}",
        result.stdout, result.stderr
    );
}
