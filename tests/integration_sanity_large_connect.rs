//! Connect-mode sanity tests for larger notebooks.
//!
//! These are the connect-mode counterparts to integration_sanity_large.rs.
//! They spin up a real Jupyter Server and exercise the same large-notebook
//! scenarios over the connect-mode execution path.
//!
//! Like integration_connect_mode, these require NB_TEST_BACKEND to be set and
//! must run single-threaded:
//!
//!   NB_TEST_BACKEND=jsd cargo test --test integration_sanity_large_connect -- --test-threads=1
//!   NB_TEST_BACKEND=none cargo test --test integration_sanity_large_connect -- --test-threads=1
//!   NB_TEST_BACKEND=jupyter-collaboration cargo test --test integration_sanity_large_connect -- --test-threads=1

mod test_helpers;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, Once, OnceLock};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ==================== SERVER INFRASTRUCTURE ====================
// (Same shared-server pattern as integration_connect_mode.rs)

struct SharedServerInfo {
    server_url: String,
    token: String,
    server_root: PathBuf,
    binary_path: PathBuf,
    venv_path_env: String,
    venv_root: PathBuf,
}

static SHARED_SERVER: OnceLock<Option<SharedServerInfo>> = OnceLock::new();
static SERVER_STOP_INFO: OnceLock<Mutex<Option<ServerStopInfo>>> = OnceLock::new();
static REGISTER_SERVER_STOP: Once = Once::new();

fn shared_server() -> Option<&'static SharedServerInfo> {
    SHARED_SERVER.get_or_init(start_shared_server).as_ref()
}

struct ServerStopInfo {
    jupyter_bin: PathBuf,
    port: u16,
    venv_path_env: String,
    venv_root: PathBuf,
}

fn server_stop_info() -> &'static Mutex<Option<ServerStopInfo>> {
    SERVER_STOP_INFO.get_or_init(|| Mutex::new(None))
}

fn register_server_stop_at_exit(info: ServerStopInfo) {
    if let Ok(mut stop_info) = server_stop_info().lock() {
        *stop_info = Some(info);
    }

    REGISTER_SERVER_STOP.call_once(|| unsafe {
        extern "C" {
            fn atexit(cb: extern "C" fn()) -> std::os::raw::c_int;
        }

        let _ = atexit(stop_shared_server_at_exit);
    });
}

extern "C" fn stop_shared_server_at_exit() {
    let Some(info) = server_stop_info()
        .lock()
        .ok()
        .and_then(|mut info| info.take())
    else {
        return;
    };

    let _ = Command::new(&info.jupyter_bin)
        .args(["server", "stop", &info.port.to_string(), "-y"])
        .env("PATH", &info.venv_path_env)
        .env("VIRTUAL_ENV", &info.venv_root)
        .env_remove("PYTHONHOME")
        .status();
}

fn start_shared_server() -> Option<SharedServerInfo> {
    let backend = test_helpers::test_backend();
    if backend.is_empty() {
        eprintln!(
            "Skipping connect-mode sanity tests: set NB_TEST_BACKEND=jsd, NB_TEST_BACKEND=jupyter-collaboration, or NB_TEST_BACKEND=none"
        );
        return None;
    }

    let venv_root = test_helpers::setup_execution_venv()?;
    let venv_path_env = test_helpers::setup_venv_environment()?;

    let venv_bin = if cfg!(windows) {
        venv_root.join("Scripts")
    } else {
        venv_root.join("bin")
    };

    let packages: &[&str] = match backend.as_str() {
        "jupyter-collaboration" | "collab" => {
            &["jupyter_server==2.20.0", "jupyter-collaboration==4.4.1"]
        }
        "none" | "plain" => &["jupyter_server==2.20.0"],
        _ => &["jupyter_server==2.20.0", "jupyter-server-documents==0.2.5"],
    };

    let mut install_args = vec!["pip", "install", "--python", venv_root.to_str().unwrap()];
    install_args.extend_from_slice(packages);
    let install_ok = Command::new("uv")
        .args(&install_args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !install_ok {
        eprintln!(
            "⚠️  Could not install {:?} into test venv for backend '{}'",
            packages, backend
        );
        return None;
    }

    let jupyter_bin = venv_bin.join("jupyter");
    if !jupyter_bin.exists() {
        eprintln!(
            "⚠️  jupyter binary not found at {} — skipping connect-mode sanity tests",
            jupyter_bin.display()
        );
        return None;
    }

    let server_root_tmp: &'static TempDir = Box::leak(Box::new(
        TempDir::new().expect("Failed to create server root tmpdir"),
    ));
    let server_root = server_root_tmp.path().to_path_buf();

    let token = "nbsanity123".to_string();

    let mut cmd = Command::new(&jupyter_bin);
    cmd.args([
        "server",
        "--no-browser",
        &format!("--ServerApp.token={}", token),
        &format!("--ServerApp.root_dir={}", server_root.display()),
    ])
    .env("PATH", &venv_path_env)
    .env("VIRTUAL_ENV", &venv_root)
    .env_remove("PYTHONHOME")
    .current_dir(&server_root)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().ok()?;

    let (server_url, port) = match wait_for_server_list_entry(
        &jupyter_bin,
        &server_root,
        &token,
        &venv_path_env,
        &venv_root,
        Duration::from_secs(30),
    ) {
        Some(server) => server,
        None => {
            eprintln!("⚠️  Jupyter Server did not appear in time — skipping");
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    if !wait_for_server(&server_url, &token, Duration::from_secs(15)) {
        eprintln!("⚠️  Jupyter Server not healthy — skipping");
        let _ = Command::new(&jupyter_bin)
            .args(["server", "stop", &port.to_string(), "-y"])
            .env("PATH", &venv_path_env)
            .env("VIRTUAL_ENV", &venv_root)
            .env_remove("PYTHONHOME")
            .status();
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }

    register_server_stop_at_exit(ServerStopInfo {
        jupyter_bin,
        port,
        venv_path_env: venv_path_env.clone(),
        venv_root: venv_root.clone(),
    });

    let binary_path = env!("CARGO_BIN_EXE_nb").into();

    Some(SharedServerInfo {
        server_url,
        token,
        server_root,
        binary_path,
        venv_path_env,
        venv_root,
    })
}

// ==================== TEST CONTEXT ====================

struct TestCtx {
    info: &'static SharedServerInfo,
}

impl TestCtx {
    fn new() -> Option<Self> {
        if test_helpers::test_backend().is_empty() {
            return None;
        }

        match shared_server() {
            Some(info) => Some(TestCtx { info }),
            None => panic!(
                "connect-mode backend '{}' was requested, but the shared Jupyter server was not available",
                test_helpers::test_backend()
            ),
        }
    }

    fn copy_fixture(&self, fixture_name: &str, dest_name: &str) -> PathBuf {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let dest_path = self.info.server_root.join(dest_name);
        fs::copy(&fixture_path, &dest_path)
            .unwrap_or_else(|_| panic!("Failed to copy fixture {}", fixture_name));
        dest_path
    }

    fn copy_fixture_with_teardown(
        &self,
        fixture_name: &str,
        dest_name: &str,
    ) -> NotebookSession<'_> {
        self.copy_fixture(fixture_name, dest_name);
        NotebookSession {
            ctx: self,
            dest_name: dest_name.to_string(),
        }
    }

    fn delete_session_for(&self, notebook_name: &str) {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return,
        };

        rt.block_on(async {
            let list_url = format!(
                "{}/api/sessions?token={}",
                self.info.server_url, self.info.token
            );
            let Ok(resp) = reqwest::get(&list_url).await else {
                return;
            };
            let Ok(sessions) = resp.json::<Vec<serde_json::Value>>().await else {
                return;
            };

            let client = reqwest::Client::new();
            for session in sessions {
                if session.get("path").and_then(|p| p.as_str()) != Some(notebook_name) {
                    continue;
                }
                let Some(id) = session.get("id").and_then(|i| i.as_str()) else {
                    continue;
                };
                let delete_url = format!(
                    "{}/api/sessions/{}?token={}",
                    self.info.server_url, id, self.info.token
                );
                let _ = client.delete(&delete_url).send().await;
            }
        });
    }

    fn run(&self, args: &[&str]) -> CommandResult {
        let mut cmd = Command::new(&self.info.binary_path);
        cmd.args(args);
        if args.contains(&"execute") && !args.contains(&"--timeout") {
            cmd.args(["--timeout", "120"]);
        }
        let output = cmd
            .args([
                "--server",
                &self.info.server_url,
                "--token",
                &self.info.token,
            ])
            .current_dir(&self.info.server_root)
            .env("PATH", &self.info.venv_path_env)
            .env("VIRTUAL_ENV", &self.info.venv_root)
            .env_remove("PYTHONHOME")
            .output()
            .expect("Failed to execute nb command");

        CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
    }
}

struct NotebookSession<'a> {
    ctx: &'a TestCtx,
    dest_name: String,
}

impl Drop for NotebookSession<'_> {
    fn drop(&mut self) {
        self.ctx.delete_session_for(&self.dest_name);
    }
}

struct CommandResult {
    stdout: String,
    stderr: String,
    success: bool,
}

impl CommandResult {
    fn assert_success(self) -> Self {
        if !self.success {
            panic!(
                "Command failed:\nStderr: {}\nStdout: {}",
                self.stderr, self.stdout
            );
        }
        self
    }
}

// ==================== SERVER HELPERS ====================

fn wait_for_server_list_entry(
    jupyter_bin: &std::path::Path,
    server_root: &std::path::Path,
    token: &str,
    venv_path_env: &str,
    venv_root: &std::path::Path,
    timeout: Duration,
) -> Option<(String, u16)> {
    let deadline = Instant::now() + timeout;
    let mut interval_ms = 200u64;

    while Instant::now() < deadline {
        if let Some(server) =
            find_server_list_entry(jupyter_bin, server_root, token, venv_path_env, venv_root)
        {
            return Some(server);
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
        interval_ms = (interval_ms * 2).min(2_000);
    }
    None
}

fn find_server_list_entry(
    jupyter_bin: &std::path::Path,
    server_root: &std::path::Path,
    token: &str,
    venv_path_env: &str,
    venv_root: &std::path::Path,
) -> Option<(String, u16)> {
    let output = Command::new(jupyter_bin)
        .args(["server", "list", "--jsonlist"])
        .env("PATH", venv_path_env)
        .env("VIRTUAL_ENV", venv_root)
        .env_remove("PYTHONHOME")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let servers: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let servers = servers.as_array()?;
    let expected_root = server_root.to_string_lossy();

    for server in servers {
        let root_dir = server
            .get("root_dir")
            .or_else(|| server.get("notebook_dir"))
            .and_then(|value| value.as_str());
        if root_dir != Some(expected_root.as_ref()) {
            continue;
        }
        if server.get("token").and_then(|value| value.as_str()) != Some(token) {
            continue;
        }
        let port = server
            .get("port")
            .and_then(|value| value.as_u64())
            .and_then(|port| u16::try_from(port).ok())?;
        let url = server.get("url").and_then(|value| value.as_str())?;
        let server_url = url.trim_end_matches('/').to_string();
        return Some((server_url, port));
    }
    None
}

fn wait_for_server(server_url: &str, token: &str, timeout: Duration) -> bool {
    let url = format!("{}/api?token={}", server_url, token);
    let deadline = Instant::now() + timeout;
    let mut interval_ms = 200u64;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");

    while Instant::now() < deadline {
        let ok = rt.block_on(async {
            match reqwest::get(&url).await {
                Ok(resp) => resp.status().is_success(),
                Err(_) => false,
            }
        });
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
        interval_ms = (interval_ms * 2).min(2_000);
    }
    false
}

// ==================== CONNECT-MODE SANITY TESTS ====================

/// Execute a 20-cell notebook with interleaved code/markdown, class definitions,
/// loops, and cross-cell state dependencies over connect mode.
#[test]
fn test_connect_large_stateful_notebook_full_execution() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping: connect-mode sanity test (no backend set)");
        return;
    };

    let _session = ctx.copy_fixture_with_teardown("large_stateful.ipynb", "sanity_stateful.ipynb");

    let result = ctx
        .run(&["execute", "sanity_stateful.ipynb"])
        .assert_success();

    assert!(
        result.stdout.contains("total=4950"),
        "Expected 'total=4950'\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );
    assert!(
        result.stdout.contains("combined=5060"),
        "Expected 'combined=5060' (cross-cell state)\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );
    assert!(
        result.stdout.contains("counter_final=25"),
        "Expected 'counter_final=25' (class across cells)\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );
    assert!(
        result.stdout.contains("all_assertions_passed=True"),
        "Expected 'all_assertions_passed=True'\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );
    assert!(
        result.stdout.contains("NOTEBOOK_COMPLETE"),
        "Expected final sentinel\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(200)..]
    );
}

/// Heavy output notebook over connect mode — verifies output is not truncated.
#[test]
fn test_connect_heavy_output_notebook() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping: connect-mode sanity test (no backend set)");
        return;
    };

    let _session = ctx.copy_fixture_with_teardown("large_heavy_output.ipynb", "sanity_heavy.ipynb");

    let result = ctx.run(&["execute", "sanity_heavy.ipynb"]).assert_success();

    assert!(
        result.stdout.contains("output_line_0000"),
        "Expected first output line\nStdout (first 500): {}",
        &result.stdout[..result.stdout.len().min(500)]
    );
    assert!(
        result.stdout.contains("output_line_0099"),
        "Expected last output line (99) — output may be truncated"
    );
    assert!(
        result.stdout.contains("HEAVY_OUTPUT_COMPLETE"),
        "Expected final sentinel\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(200)..]
    );
}

/// Error handling over connect mode — execution should halt at error cell.
#[test]
fn test_connect_error_notebook_halts() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping: connect-mode sanity test (no backend set)");
        return;
    };

    let _session = ctx.copy_fixture_with_teardown("large_with_error.ipynb", "sanity_error.ipynb");

    let result = ctx.run(&["execute", "sanity_error.ipynb"]);

    assert!(!result.success, "Execution should fail when a cell errors");

    let combined = format!("{}\n{}", result.stdout, result.stderr);
    assert!(
        combined.contains("ZeroDivisionError"),
        "Expected ZeroDivisionError\nOutput: {}",
        &combined[combined.len().saturating_sub(500)..]
    );

    // Verify no @@output stream sections after the error output
    let lines: Vec<&str> = result.stdout.lines().collect();
    let error_output_idx = lines
        .iter()
        .position(|l| l.contains("@@output") && l.contains("\"output_type\":\"error\""));
    assert!(
        error_output_idx.is_some(),
        "Expected an error @@output section"
    );

    let after_error_outputs: Vec<&&str> = lines[error_output_idx.unwrap()..]
        .iter()
        .filter(|l| l.contains("@@output") && l.contains("\"output_type\":\"stream\""))
        .collect();
    assert!(
        after_error_outputs.is_empty(),
        "No stream outputs should appear after the error\nFound: {:?}",
        after_error_outputs
    );
}

/// Error handling with --allow-errors over connect mode.
#[test]
fn test_connect_error_notebook_continues_with_allow_errors() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping: connect-mode sanity test (no backend set)");
        return;
    };

    let _session =
        ctx.copy_fixture_with_teardown("large_with_error.ipynb", "sanity_error_continue.ipynb");

    let result = ctx.run(&["execute", "sanity_error_continue.ipynb", "--allow-errors"]);

    assert!(
        result.stdout.contains("ZeroDivisionError"),
        "Error should be reported\nStdout: {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );

    // All 5 cells should have @@output sections
    let output_count = result
        .stdout
        .lines()
        .filter(|l| l.starts_with("@@output"))
        .count();
    assert_eq!(
        output_count, 5,
        "With --allow-errors, expected 5 @@output sections, got {}",
        output_count
    );

    assert!(
        result.stdout.contains("after_error=True"),
        "Post-error cells should execute\nStdout: {}",
        &result.stdout[result.stdout.len().saturating_sub(300)..]
    );
}

/// Partial execution (single cell) over connect mode.
#[test]
fn test_connect_partial_execution() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping: connect-mode sanity test (no backend set)");
        return;
    };

    let _session = ctx.copy_fixture_with_teardown("large_stateful.ipynb", "sanity_partial.ipynb");

    let result = ctx
        .run(&["execute", "sanity_partial.ipynb", "--cell-index", "0"])
        .assert_success();

    assert!(
        result.stdout.contains("Python 3"),
        "Expected Python version from cell 0\nStdout: {}",
        &result.stdout[..result.stdout.len().min(500)]
    );

    // Only 1 @@output section for the single executed cell
    let output_count = result
        .stdout
        .lines()
        .filter(|l| l.starts_with("@@output"))
        .count();
    assert_eq!(
        output_count, 1,
        "Partial execution should produce exactly 1 @@output, got {}",
        output_count
    );
}
