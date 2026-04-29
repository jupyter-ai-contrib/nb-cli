//! Connect-mode integration tests.
//!
//! These tests spin up a real Jupyter Server and must be run **single-threaded**
//! to avoid races on the shared server. Always invoke with:
//!
//!   cargo test --test integration_connect_mode -- --test-threads=1

mod test_helpers;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use test_helpers::CommandResult;

// reqwest is a workspace dependency used for the server health-check poll.
use reqwest;

// ==================== SERVER INFRASTRUCTURE ====================

/// Lightweight info about the shared Jupyter Server, shared across all tests.
struct SharedServerInfo {
    server_url: String,
    token: String,
    /// Path to the server root directory (a leaked TempDir, lives until process exit).
    server_root: PathBuf,
    binary_path: PathBuf,
    venv_path_env: String,
    venv_root: PathBuf,
}

// SAFETY: All fields are Send + Sync (Strings and PathBufs).
unsafe impl Send for SharedServerInfo {}
unsafe impl Sync for SharedServerInfo {}

/// One shared Jupyter Server for the whole test suite.
/// Initialized on first access; lives until the test process exits.
static SHARED_SERVER: OnceLock<Option<SharedServerInfo>> = OnceLock::new();

fn shared_server() -> Option<&'static SharedServerInfo> {
    SHARED_SERVER.get_or_init(start_shared_server).as_ref()
}

fn start_shared_server() -> Option<SharedServerInfo> {
    // If NB_TEST_SERVER_URL/TOKEN are set (e.g. by `just test`), use the
    // externally-managed server instead of starting one. This avoids the
    // port-contention race that occurs when many test processes start in parallel.
    if let (Ok(server_url), Ok(token)) = (
        std::env::var("NB_TEST_SERVER_URL"),
        std::env::var("NB_TEST_SERVER_TOKEN"),
    ) {
        let venv_root = test_helpers::setup_execution_venv()?;
        let venv_path_env = test_helpers::setup_venv_environment()?;
        let binary_path = env!("CARGO_BIN_EXE_nb").into();
        let server_root: PathBuf = std::env::var("NB_TEST_SERVER_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        if !wait_for_server(&server_url, &token, Duration::from_secs(5)) {
            eprintln!("⚠️  NB_TEST_SERVER_URL is set but server is not responding — skipping connect-mode tests");
            return None;
        }
        return Some(SharedServerInfo {
            server_url,
            token,
            server_root,
            binary_path,
            venv_path_env,
            venv_root,
        });
    }

    // Reuse the existing execution venv (setup_test_env.sh has already installed
    // ipykernel, jupyter_server, and jupyter-server-documents into it).
    let venv_root = test_helpers::setup_execution_venv()?;
    let venv_path_env = test_helpers::setup_venv_environment()?;

    let venv_bin = if cfg!(windows) {
        venv_root.join("Scripts")
    } else {
        venv_root.join("bin")
    };

    // Verify the `jupyter` binary exists in the venv (installed by setup_test_env.sh).
    let jupyter_bin = venv_bin.join("jupyter");
    if !jupyter_bin.exists() {
        eprintln!(
            "⚠️  jupyter binary not found at {} — skipping connect-mode tests",
            jupyter_bin.display()
        );
        return None;
    }

    // Leak the TempDir so the directory persists for the lifetime of the process.
    // The OS will clean up the temp files on process exit.
    let server_root_tmp: &'static TempDir = Box::leak(Box::new(
        TempDir::new().expect("Failed to create server root tmpdir"),
    ));
    let server_root = server_root_tmp.path().to_path_buf();

    let token = "nbtest123".to_string();

    // Pre-allocate a port by binding, recording it, then releasing.
    // The TOCTOU window is small; the env-var path (used by `just test`)
    // avoids this entirely by starting the server before test processes launch.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        listener.local_addr().ok()?.port()
    };

    let child = Command::new(&jupyter_bin)
        .args([
            "server",
            "--no-browser",
            &format!("--ServerApp.token={}", token),
            &format!("--ServerApp.root_dir={}", server_root.display()),
            &format!("--port={}", port),
            "--ServerApp.open_browser=False",
        ])
        .env("PATH", &venv_path_env)
        .env("VIRTUAL_ENV", &venv_root)
        .env_remove("PYTHONHOME")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Leak the process guard so it lives until process exit (and kills the server).
    let _guard: &'static mut ServerKillGuard = Box::leak(Box::new(ServerKillGuard { child }));

    let server_url = format!("http://127.0.0.1:{}", port);

    if !wait_for_server(&server_url, &token, Duration::from_secs(15)) {
        eprintln!("⚠️  Jupyter Server did not become ready in time — skipping connect-mode tests");
        return None;
    }

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

/// Kills the child process when dropped.
struct ServerKillGuard {
    child: std::process::Child,
}

impl Drop for ServerKillGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Per-test helper that wraps the shared server and provides convenience methods.
struct TestCtx {
    info: &'static SharedServerInfo,
    /// Per-test temp directory used as CWD for config-path-dependent commands
    /// (connect, disconnect, status). Each test gets its own .jupyter/cli.json.
    work_dir: TempDir,
}

impl TestCtx {
    fn new() -> Option<Self> {
        shared_server().map(|info| TestCtx {
            info,
            work_dir: TempDir::new().expect("Failed to create per-test work dir"),
        })
    }

    /// Copy a fixture notebook into the server root under `dest_name` and return the path.
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

    /// Run any `nb` command in server_root without implicit args.
    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.info.binary_path)
            .args(args)
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

    /// Run a remote `nb` command, automatically appending `--server` and `--token`.
    fn run_remote(&self, args: &[&str]) -> CommandResult {
        self.run(
            &args
                .iter()
                .copied()
                .chain([
                    "--server",
                    &self.info.server_url,
                    "--token",
                    &self.info.token,
                ])
                .collect::<Vec<_>>(),
        )
    }

    /// Run `nb` in `work_dir` WITHOUT auto-appended `--server`/`--token`.
    /// Use for connect/disconnect/status commands that rely on saved config.
    fn run_in_workdir(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.info.binary_path)
            .args(args)
            .current_dir(self.work_dir.path())
            .env("PATH", &self.info.venv_path_env)
            .env("VIRTUAL_ENV", &self.info.venv_root)
            .env_remove("PYTHONHOME")
            .output()
            .expect("Failed to execute nb command in work_dir");

        CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
    }
}

/// Jupyter serializes MultilineString as either a plain JSON string or an array of strings.
/// This helper joins array elements into one string, or returns the string as-is.
fn join_jupyter_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => String::new(),
    }
}

/// Block until `GET {server_url}/api?token={token}` returns HTTP 200, or `timeout` elapses.
fn wait_for_server(server_url: &str, token: &str, timeout: Duration) -> bool {
    let url = format!("{}/api?token={}", server_url, token);
    let deadline = Instant::now() + timeout;
    let mut interval_ms = 200u64;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime for server health check");

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

// ==================== CONNECT MODE TESTS ====================

/// Sentinel: fail loudly if the Jupyter server did not start.
///
/// Without this, every test in this file silently returns early (Rust marks them "ok")
/// when the server is unavailable — producing a false-green CI signal with zero assertions.
/// Alphabetical prefix "aaa_" ensures this runs first under --test-threads=1.
#[test]
fn aaa_server_must_be_available() {
    if shared_server().is_none() {
        panic!(
            "Jupyter server failed to start — all connect-mode tests would silently skip. \
             Check that jupyter_server and jupyter-server-documents are installed in the test venv \
             (run ./tests/setup_test_env.sh) and that a free port was available."
        );
    }
}

/// Prove that without `--restart-kernel`, the kernel state persists between executions.
///
/// 1. Execute the full notebook → `persistent_var` is set, cell-use prints it.
/// 2. Execute only cell-use (index 1) without restarting → the value is still in scope.
#[test]
fn test_execute_without_restart_preserves_state() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_connect_restart.ipynb", "test_preserve.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    // First: execute the full notebook to establish kernel state.
    let result = ctx.run_remote(&["execute", nb_str]).assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );

    // Second: execute only cell-use (index 1) — no restart.
    // The kernel should still have `persistent_var` in scope.
    let result = ctx
        .run_remote(&["execute", nb_str, "--cell-index", "1"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Cell-use re-execution without restart should still print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );
}

/// Prove that `--restart-kernel` clears the kernel state.
///
/// 1. Execute the full notebook → session is established, `persistent_var` is set.
/// 2. Execute only cell-use (index 1) without restart → succeeds (state preserved).
/// 3. Execute only cell-use (index 1) with `--restart-kernel --allow-errors` →
///    the kernel has been restarted so `persistent_var` is undefined → NameError.
#[test]
fn test_restart_kernel_clears_state() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // Use a unique notebook name so this test has its own independent session.
    let nb_path = ctx.copy_fixture("for_connect_restart.ipynb", "test_restart.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    // Step 1: run the full notebook to create the session and set state.
    let result = ctx.run_remote(&["execute", nb_str]).assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );

    // Step 2: run cell-use without restart — variable should still be in scope.
    let result = ctx
        .run_remote(&["execute", nb_str, "--cell-index", "1"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Without restart, cell-use should still find persistent_var\nStdout: {}",
        result.stdout
    );

    // Step 3: run cell-use *with* restart → NameError because the kernel was restarted
    // and `persistent_var` was never re-defined.
    let result = ctx
        .run_remote(&[
            "execute",
            nb_str,
            "--cell-index",
            "1",
            "--restart-kernel",
            "--allow-errors",
        ])
        .assert_failure();

    let combined = format!("{}\n{}", result.stdout, result.stderr);
    assert!(
        combined.contains("NameError"),
        "After restart, cell-use should produce a NameError because persistent_var is undefined\nStdout: {}\nStderr: {}",
        result.stdout,
        result.stderr
    );
}

/// Prove that `--restart-kernel` followed by a full notebook re-execution succeeds.
///
/// After the kernel is restarted, running all cells from scratch must work correctly
/// and produce the expected output.
#[test]
fn test_restart_kernel_then_full_notebook_works() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // Use a unique notebook name so this test has its own independent session.
    let nb_path = ctx.copy_fixture("for_connect_restart.ipynb", "test_restart_full.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    // Step 1: initial full execution to create the session.
    ctx.run_remote(&["execute", nb_str]).assert_success();

    // Step 2: full re-execution with --restart-kernel.
    // All cells are run in order from scratch, so cell-set runs before cell-use.
    let result = ctx
        .run_remote(&["execute", nb_str, "--restart-kernel"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution after restart should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );
}

// ==================== CONNECT / STATUS / DISCONNECT TESTS ====================

/// `nb connect --server URL --token TOKEN` must write .jupyter/cli.json with the URL.
#[test]
fn test_connect_manual_saves_config() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let result = ctx
        .run_in_workdir(&[
            "connect",
            "--server",
            &ctx.info.server_url,
            "--token",
            &ctx.info.token,
        ])
        .assert_success();

    assert!(
        result.stdout.contains("Connected"),
        "Expected 'Connected' in output\nStdout: {}",
        result.stdout
    );

    let config_path = ctx.work_dir.path().join(".jupyter").join("cli.json");
    assert!(
        config_path.exists(),
        ".jupyter/cli.json must exist after connect"
    );

    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    let json: serde_json::Value =
        serde_json::from_str(&content).expect("Config must be valid JSON");
    assert_eq!(
        json["connection"]["server_url"].as_str(),
        Some(ctx.info.server_url.as_str()),
        "Config must store the connected server_url"
    );
}

/// `nb connect --server DEAD_URL --token tok --skip-validation` must save config
/// without attempting a network connection.
#[test]
fn test_connect_manual_skip_validation() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let result = ctx
        .run_in_workdir(&[
            "connect",
            "--server",
            "http://127.0.0.1:1",
            "--token",
            "tok",
            "--skip-validation",
        ])
        .assert_success();

    assert!(
        result.stdout.contains("Connected"),
        "Expected 'Connected' even with dead URL + --skip-validation\nStdout: {}",
        result.stdout
    );

    let config_path = ctx.work_dir.path().join(".jupyter").join("cli.json");
    assert!(
        config_path.exists(),
        ".jupyter/cli.json must be written even with --skip-validation"
    );
}

/// After `nb connect`, `nb status --validate` must report "✓ Connection is valid".
#[test]
fn test_status_validate_live_connection() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.run_in_workdir(&[
        "connect",
        "--server",
        &ctx.info.server_url,
        "--token",
        &ctx.info.token,
    ])
    .assert_success();

    let result = ctx
        .run_in_workdir(&["status", "--validate"])
        .assert_success();
    assert!(
        result.stdout.contains("✓ Connection is valid"),
        "Expected '✓ Connection is valid'\nStdout: {}",
        result.stdout
    );
}

/// After saving a dead URL via `--skip-validation`, `nb status --validate`
/// must report "✗ Connection failed". Exit code is 0 (status always succeeds).
#[test]
fn test_status_validate_dead_connection() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.run_in_workdir(&[
        "connect",
        "--server",
        "http://127.0.0.1:1",
        "--token",
        "tok",
        "--skip-validation",
    ])
    .assert_success();

    let result = ctx
        .run_in_workdir(&["status", "--validate"])
        .assert_success();
    assert!(
        result.stdout.contains("✗ Connection failed"),
        "Expected '✗ Connection failed'\nStdout: {}",
        result.stdout
    );
}

/// Full cycle: connect → disconnect → status → "Not connected".
#[test]
fn test_disconnect_and_reannotate_status() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.run_in_workdir(&[
        "connect",
        "--server",
        &ctx.info.server_url,
        "--token",
        &ctx.info.token,
    ])
    .assert_success();

    ctx.run_in_workdir(&["disconnect"]).assert_success();

    let result = ctx.run_in_workdir(&["status"]).assert_success();
    assert!(
        result
            .stdout
            .contains("Not connected to any Jupyter server"),
        "Expected 'Not connected' after disconnect\nStdout: {}",
        result.stdout
    );
}

// ==================== REMOTE EXECUTION PARITY TESTS ====================

/// An error cell must cause exit non-zero in remote mode (same as local mode).
#[test]
fn test_remote_execute_with_error_fails() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("with_error.ipynb", "test_remote_error.ipynb");
    ctx.run_remote(&["execute", nb_path.to_str().unwrap()])
        .assert_failure();
}

/// `--allow-errors` continues executing cells after an error (does not stop early).
/// The command still exits non-zero because errors occurred.
#[test]
fn test_remote_execute_with_allow_errors() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("with_error.ipynb", "test_remote_allow_err.ipynb");
    let result = ctx.run_remote(&["execute", nb_path.to_str().unwrap(), "--allow-errors"]);

    // Exits non-zero (errors occurred) but produces output for all cells.
    assert!(
        !result.success,
        "--allow-errors must still exit non-zero when errors occur"
    );
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "must output notebook markdown even when errors occur"
    );
    // Both cells ran — valid_code cell succeeded, error cell has error output.
    let cells = test_helpers::parse_cells(&result.stdout);
    assert!(cells.len() >= 2, "both cells must appear in output");
}

/// An error mid-notebook must (a) exit non-zero, (b) report partial results for cells
/// that ran before the error, and (c) not execute cells after the error.
///
/// Mirrors `test_execute_error_shows_partial_results_and_error` in integration_execution.rs.
/// Uses `for_connect_error_stop.ipynb`: cell-0 `x=100`, cell-1 `raise ValueError`, cell-2 `print(x)`.
#[test]
fn test_remote_execute_error_shows_partial_results() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture(
        "for_connect_error_stop.ipynb",
        "test_remote_error_stop.ipynb",
    );

    let result = ctx
        .run_remote(&["execute", nb_path.to_str().unwrap(), "--json"])
        .assert_failure();

    let json: serde_json::Value = serde_json::from_str(&result.stdout)
        .expect("--json output must be valid JSON even on error");
    assert_eq!(
        json["success"], false,
        "success must be false when a cell raises"
    );

    let cells = json["cells"]
        .as_array()
        .expect("JSON must have 'cells' array");
    let code_cells: Vec<_> = cells.iter().filter(|c| c["cell_type"] == "code").collect();
    assert_eq!(code_cells.len(), 3, "Fixture has 3 code cells");

    // Cell 0 (`x = 100`) must have run: execution_count is a number, not null.
    assert!(
        code_cells[0]["execution_count"].is_number(),
        "Cell 0 (x=100) must have execution_count — it ran before the error\nCell: {:?}",
        code_cells[0]
    );

    // Cell 1 (`raise ValueError`) must have an error output.
    let c1_outputs = code_cells[1]["outputs"]
        .as_array()
        .expect("Cell 1 must have outputs array");
    assert!(
        c1_outputs.iter().any(|o| o["output_type"] == "error"),
        "Cell 1 must have an error output\nOutputs: {:?}",
        c1_outputs
    );

    // Cell 2 (`print(x)`) must NOT have executed: no execution_count.
    assert!(
        code_cells[2]["execution_count"].is_null(),
        "Cell 2 (print(x)) must not execute after error in cell 1\nCell: {:?}",
        code_cells[2]
    );
}

/// `--json` output in remote mode must include captured cell outputs.
#[test]
fn test_remote_execute_json_includes_outputs() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_json_out.ipynb");
    let result = ctx
        .run_remote(&["execute", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");

    assert_eq!(json["success"], true);
    let cells = json["cells"]
        .as_array()
        .expect("JSON must have 'cells' array");
    let code_cells: Vec<_> = cells.iter().filter(|c| c["cell_type"] == "code").collect();
    let last = code_cells.last().expect("Must have at least one code cell");
    let outputs = last["outputs"]
        .as_array()
        .expect("Last cell must have outputs");
    assert!(
        !outputs.is_empty(),
        "Remote execution must capture cell outputs in JSON"
    );

    let output_text = serde_json::to_string(outputs).unwrap();
    assert!(
        output_text.contains("Result: 52"),
        "Output must contain 'Result: 52'\nOutputs: {}",
        output_text
    );
}

// ==================== ERROR HANDLING TESTS ====================

/// Executing with a valid server URL but wrong token must fail.
#[test]
fn test_execute_with_bad_token_fails() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_bad_tok.ipynb");

    Command::new(&ctx.info.binary_path)
        .args(["execute", nb_path.to_str().unwrap()])
        .args([
            "--server",
            &ctx.info.server_url,
            "--token",
            "wrong_token_xyz",
        ])
        .current_dir(&ctx.info.server_root)
        .env("PATH", &ctx.info.venv_path_env)
        .env("VIRTUAL_ENV", &ctx.info.venv_root)
        .env_remove("PYTHONHOME")
        .output()
        .map(|o| {
            assert!(
                !o.status.success(),
                "Execution with wrong token must exit non-zero"
            );
        })
        .expect("Failed to spawn nb command");
}

/// Executing against an unreachable server must fail quickly (not hang).
#[test]
fn test_execute_with_bad_server_url_fails() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_bad_server.ipynb");

    let start = std::time::Instant::now();
    let output = Command::new(&ctx.info.binary_path)
        .args(["execute", nb_path.to_str().unwrap()])
        .args(["--server", "http://127.0.0.1:1", "--token", "tok"])
        .current_dir(&ctx.info.server_root)
        .env("PATH", &ctx.info.venv_path_env)
        .env("VIRTUAL_ENV", &ctx.info.venv_root)
        .env_remove("PYTHONHOME")
        .output()
        .expect("Failed to spawn nb command");

    assert!(
        !output.status.success(),
        "Execution against unreachable server must exit non-zero"
    );
    assert!(
        start.elapsed() < Duration::from_secs(15),
        "Must fail within 15 seconds, not hang indefinitely"
    );
}

/// Connecting with a valid server URL but a wrong token must fail (not write config).
#[test]
fn test_connect_with_bad_credentials_fails() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let result = ctx.run_in_workdir(&[
        "connect",
        "--server",
        &ctx.info.server_url,
        "--token",
        "definitely_wrong_token_xyz",
    ]);

    assert!(
        !result.success,
        "connect with wrong token must exit non-zero\nStdout: {}\nStderr: {}",
        result.stdout, result.stderr
    );

    let config_path = ctx.work_dir.path().join(".jupyter").join("cli.json");
    assert!(
        !config_path.exists(),
        ".jupyter/cli.json must NOT be written when connection validation fails"
    );
}

/// Execute a single cell by stable ID (--cell <id>) in remote mode.
#[test]
fn test_remote_execute_cell_by_id() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // cell-1 in for_execution.ipynb is `x = 42` — no dependencies, safe to run alone.
    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_cell_by_id.ipynb");
    ctx.run_remote(&["execute", nb_path.to_str().unwrap(), "--cell", "cell-1"])
        .assert_success();
}

/// Execute a range of cells (--start / --end) in remote mode; verify code cells run in order.
#[test]
fn test_remote_execute_cell_range() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // for_connect_cell_selection.ipynb layout:
    //   0: markdown "# H1"
    //   1: code    `a = 1`
    //   2: markdown "## H2"
    //   3: code    `print(a)`
    // Running --start 1 --end 3 executes cells 1-3; markdown cells are skipped.
    // Cell 1 sets `a`, cell 3 prints it → output "1".
    let nb_path = ctx.copy_fixture(
        "for_connect_cell_selection.ipynb",
        "test_remote_range.ipynb",
    );
    let result = ctx
        .run_remote(&[
            "execute",
            nb_path.to_str().unwrap(),
            "--start",
            "1",
            "--end",
            "3",
            "--json",
        ])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    assert_eq!(json["success"], true);

    let cells = json["cells"]
        .as_array()
        .expect("JSON must have 'cells' array");

    // Navigate to the last code cell — that's `print(a)` which must have produced "1".
    // (Do NOT use `contains('1')` on the full JSON: execution_count and cell indices
    // also contain the character '1', producing a false positive even if print never ran.)
    let last_code_cell = cells
        .iter()
        .filter(|c| c["cell_type"] == "code")
        .last()
        .expect("Must have at least one code cell in response");

    let outputs = last_code_cell["outputs"]
        .as_array()
        .expect("Last code cell must have an outputs array");
    assert!(
        !outputs.is_empty(),
        "print(a) must produce at least one output\nFull cells JSON: {}",
        serde_json::to_string(cells).unwrap()
    );

    // Stream outputs use "text"; execute_result outputs use "data"."text/plain".
    // Jupyter serializes MultilineString as a JSON array of strings; handle both.
    let text = {
        let t = join_jupyter_text(&outputs[0]["text"]);
        if !t.is_empty() {
            t
        } else {
            join_jupyter_text(&outputs[0]["data"]["text/plain"])
        }
    };
    assert_eq!(
        text.trim(),
        "1",
        "print(a) where a=1 must output exactly '1'\nOutputs: {:?}",
        outputs
    );
}

/// Execute a notebook, then `nb read` the output cell — verify outputs were persisted.
#[test]
fn test_remote_execute_output_matches_read() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_read_back.ipynb");

    // Execute the full notebook.
    ctx.run_remote(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Read cell index 2 (the print cell) and verify the output was written back.
    // nb read is a local command — use run() (no --server/--token) not run_remote().
    let read_result = ctx
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "2",
            "--json",
        ])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&read_result.stdout).expect("read --json must produce valid JSON");

    // nb read --cell-index returns a single cell object, not {"cells": [...]}.
    let outputs = json["outputs"]
        .as_array()
        .expect("Cell must have outputs array after execution");

    assert!(
        !outputs.is_empty(),
        "Cell outputs must be non-empty after remote execution"
    );

    let output_text = serde_json::to_string(outputs).unwrap();
    assert!(
        output_text.contains("Result: 52"),
        "Persisted output must contain 'Result: 52'\nOutputs: {}",
        output_text
    );
}

/// Execute the last cell using --cell-index -1 (negative indexing).
#[test]
fn test_remote_execute_negative_cell_index() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_neg_idx.ipynb");

    // Execute the full notebook first to set kernel state (x, y in scope).
    ctx.run_remote(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Execute the last cell (index -1: `print(f'Result: {y}')`) — needs x and y in scope.
    let result = ctx
        .run_remote(&["execute", nb_path.to_str().unwrap(), "--cell-index", "-1"])
        .assert_success();

    assert!(
        result.stdout.contains("Result: 52"),
        "Last cell execution via --cell-index -1 must print 'Result: 52'\nStdout: {}",
        result.stdout
    );
}

/// Documents known behavior: --timeout does NOT interrupt a kernel mid-execution.
///
/// The select! loop in mod.rs has no timeout arm when idle_received=false, so
/// A long-running cell (sleep 60) must be interrupted by --timeout.
/// The deadline arm in the select! loop fires after the specified duration,
/// breaking out of the kernel-wait loop and returning partial results.
#[test]
fn test_execute_timeout_interrupts_running_kernel() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // for_connect_timeout.ipynb contains a single `time.sleep(60)` cell.
    // With --timeout 2, the CLI must interrupt the kernel and return within a few seconds.
    let nb_path = ctx.copy_fixture("for_connect_timeout.ipynb", "test_timeout.ipynb");

    let start = std::time::Instant::now();
    let result = ctx.run_remote(&["execute", nb_path.to_str().unwrap(), "--timeout", "2"]);
    let elapsed = start.elapsed();

    // The command must return well before the cell would finish (60s).
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "command must exit within timeout + grace; elapsed: {:?}",
        elapsed
    );
    // Partial execution is returned as success (not a hard error).
    result.assert_success();
}
