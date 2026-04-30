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

impl Drop for TestCtx {
    fn drop(&mut self) {
        // Delete all sessions to kill idle kernels and prevent Y.js room accumulation.
        if let Ok(output) = Command::new("curl")
            .args([
                "-sf",
                &format!(
                    "{}/api/sessions?token={}",
                    self.info.server_url, self.info.token
                ),
            ])
            .output()
        {
            if let Ok(sessions) = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout) {
                for session in &sessions {
                    if let Some(id) = session["id"].as_str() {
                        let _ = Command::new("curl")
                            .args([
                                "-sf",
                                "-X",
                                "DELETE",
                                &format!(
                                    "{}/api/sessions/{}?token={}",
                                    self.info.server_url, id, self.info.token
                                ),
                            ])
                            .output();
                    }
                }
            }
        }
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

    // Initial execution creates the session.
    ctx.run_remote(&["execute", nb_str]).assert_success();

    // Re-execution with --restart-kernel must succeed:
    // the kernel restarts and all cells run from scratch.
    ctx.run_remote(&["execute", nb_str, "--restart-kernel"])
        .assert_success();
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

/// An error cell must produce an error output in the notebook.
/// Verified via --json: the executor waits for Y.js sync internally (default 30s timeout).
#[test]
fn test_remote_execute_with_error_produces_output() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("with_error.ipynb", "test_remote_error.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx.run_remote(&["execute", nb_str, "--json"]);
    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let cells = json["cells"].as_array().expect("cells array must exist");
    let cell1_outputs = cells[1]["outputs"].as_array();
    assert!(
        cell1_outputs.is_some_and(|o| o.iter().any(|x| x["output_type"] == "error")),
        "Cell 1 must have error output\nJSON: {}",
        result.stdout
    );
}

/// `--allow-errors` continues executing cells after an error (does not stop early).
/// Verified via --json: both cells must have execution_count set.
#[test]
fn test_remote_execute_with_allow_errors() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("with_error.ipynb", "test_remote_allow_err.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx.run_remote(&["execute", nb_str, "--allow-errors", "--json"]);
    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    assert_eq!(
        json["executed_cells"].as_u64(),
        Some(2),
        "--allow-errors must execute both cells\nJSON: {}",
        result.stdout
    );
    let cells = json["cells"].as_array().expect("cells array must exist");
    assert!(
        cells[0]["execution_count"].is_number(),
        "Cell 0 must have execution_count"
    );
    assert!(
        cells[1]["execution_count"].is_number(),
        "Cell 1 must have execution_count"
    );
}

/// An error mid-notebook must produce an error output for the failing cell.
/// Verified via --json: cell 0 ran (execution_count set), cell 1 has error output.
///
/// Uses `for_connect_error_stop.ipynb`: cell-0 `x=100`, cell-1 `raise ValueError`, cell-2 `print(x)`.
/// Cell 2 state is not asserted — whether it runs depends on Y.js error detection timing.
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
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx.run_remote(&["execute", nb_str, "--json"]);
    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let cells = json["cells"].as_array().expect("cells array must exist");
    assert!(
        cells[0]["execution_count"].is_number(),
        "Cell 0 must have execution_count (ran before error)\nJSON: {}",
        result.stdout
    );
    let cell1_outputs = cells[1]["outputs"].as_array();
    assert!(
        cell1_outputs.is_some_and(|o| o.iter().any(|x| x["output_type"] == "error")),
        "Cell 1 must have error output\nJSON: {}",
        result.stdout
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
/// Verified via --json output: execution_count comes from the kernel (no Y.js/disk dependency).
#[test]
fn test_remote_execute_cell_by_id() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_cell_by_id.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx
        .run_remote(&["execute", nb_str, "--cell", "cell-1", "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    assert_eq!(
        json["executed_cells"].as_u64(),
        Some(1),
        "Expected 1 cell executed\nJSON: {}",
        result.stdout
    );
    let cells = json["cells"].as_array().expect("cells array must exist");
    assert!(
        cells[0]["execution_count"].is_number(),
        "Cell 0 must have execution_count — proves --cell cell-1 ran it"
    );
}

/// Execute a range of cells (--start / --end) in remote mode.
/// Verified via --json: cell 3 (print(a)) must have output "1",
/// proving both code cells ran in order (cell 1 set a=1, cell 3 printed it).
#[test]
fn test_remote_execute_cell_range() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // Layout: [0] markdown, [1] code `a = 1`, [2] markdown, [3] code `print(a)`
    let nb_path = ctx.copy_fixture(
        "for_connect_cell_selection.ipynb",
        "test_remote_range.ipynb",
    );
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx
        .run_remote(&["execute", nb_str, "--start", "1", "--end", "3", "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let cells = json["cells"].as_array().expect("cells array must exist");
    let cell3_outputs = serde_json::to_string(&cells[3]["outputs"]).unwrap();
    assert!(
        cell3_outputs.contains("1"),
        "print(a) where a=1 must output '1'\nOutputs: {}",
        cell3_outputs
    );
}

/// Execute a full notebook in remote mode and verify the final cell produced correct output.
/// Verified via --json: the executor waits for Y.js sync internally (default 30s timeout).
#[test]
fn test_remote_execute_full_notebook_produces_output() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_full.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx
        .run_remote(&["execute", nb_str, "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let cells = json["cells"].as_array().expect("cells array must exist");
    let cell2_outputs = serde_json::to_string(&cells[2]["outputs"]).unwrap();
    assert!(
        cell2_outputs.contains("Result: 52"),
        "Final cell must output 'Result: 52'\nOutputs: {}",
        cell2_outputs
    );
}

/// Execute the last cell using --cell-index -1 (negative indexing).
/// Uses a self-contained fixture so only ONE execution is needed (no kernel state setup).
/// Verifies: (1) the last cell produced the expected output, (2) earlier cells were NOT executed.
#[test]
fn test_remote_execute_negative_cell_index() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    // Fixture: [0] a=1, [1] b=2, [2] print('negative-index-works')
    // Cell 2 is self-contained — no dependency on prior cells.
    let nb_path = ctx.copy_fixture("for_connect_neg_index.ipynb", "test_remote_neg_idx.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    let result = ctx
        .run_remote(&["execute", nb_str, "--cell-index", "-1", "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let cells = json["cells"].as_array().expect("cells array must exist");
    let cell2_outputs = serde_json::to_string(&cells[2]["outputs"]).unwrap();
    assert!(
        cell2_outputs.contains("negative-index-works"),
        "Last cell must output 'negative-index-works'\nOutputs: {}",
        cell2_outputs
    );
    assert!(
        cells[0]["execution_count"].is_null(),
        "Cell 0 must not have been executed — --cell-index -1 should only run the last cell"
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

/// Verify `nb add-cell` writes a new cell via the Y.js remote path.
///
/// Uses `nb read --json` with polling to wait for the async server auto-save.
#[test]
fn test_remote_add_cell() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_add_cell.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    ctx.run_remote(&["cell", "add", nb_str, "--source", "z = 42"])
        .assert_success();

    // Poll until Y.js syncs the new cell to disk (observed: ~1-2s under load).
    let deadline = Instant::now() + Duration::from_secs(10);
    let json = loop {
        let r = ctx.run(&["read", nb_str, "--json"]).assert_success();
        let json: serde_json::Value =
            serde_json::from_str(&r.stdout).expect("read --json must produce valid JSON");
        if json["cells"].as_array().map_or(0, |c| c.len()) == 4 {
            break json;
        }
        assert!(
            Instant::now() < deadline,
            "Y.js did not sync add-cell to disk within 10s"
        );
        std::thread::sleep(Duration::from_millis(200));
    };

    let cells = json["cells"].as_array().unwrap();
    let last_src = cells[3]["source"]
        .as_str()
        .or_else(|| {
            cells[3]["source"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    assert!(
        last_src.contains("z = 42"),
        "added cell source must contain 'z = 42'\nsource: {:?}",
        last_src
    );
}

/// Verify `nb delete-cell` removes a cell via the Y.js remote path.
#[test]
fn test_remote_delete_cell() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_delete_cell.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    // Delete cell at index 1 (y = x + 10) — leaves cells 0 and 2.
    ctx.run_remote(&["cell", "delete", nb_str, "--cell-index", "1"])
        .assert_success();

    let deadline = Instant::now() + Duration::from_secs(10);
    let json = loop {
        let r = ctx.run(&["read", nb_str, "--json"]).assert_success();
        let json: serde_json::Value =
            serde_json::from_str(&r.stdout).expect("read --json must produce valid JSON");
        if json["cells"].as_array().map_or(3, |c| c.len()) == 2 {
            break json;
        }
        assert!(
            Instant::now() < deadline,
            "Y.js did not sync delete-cell to disk within 10s"
        );
        std::thread::sleep(Duration::from_millis(200));
    };

    let cells = json["cells"].as_array().unwrap();
    let src0 = cells[0]["source"]
        .as_str()
        .or_else(|| {
            cells[0]["source"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    let src1 = cells[1]["source"]
        .as_str()
        .or_else(|| {
            cells[1]["source"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    assert!(
        src0.contains("x = 42"),
        "cell 0 must still be 'x = 42'\nsource: {:?}",
        src0
    );
    assert!(
        src1.contains("Result"),
        "cell 1 (shifted from index 2) must contain 'Result'\nsource: {:?}",
        src1
    );
}

/// Verify `nb update-cell` replaces cell source via the Y.js remote path
/// and resets execution_count to null.
#[test]
fn test_remote_update_cell() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let nb_path = ctx.copy_fixture("for_execution.ipynb", "test_remote_update_cell.ipynb");
    let nb_str = nb_path.to_str().unwrap();

    // First execute to set execution_count, then update to verify reset.
    ctx.run_remote(&["execute", nb_str]).assert_success();

    ctx.run_remote(&[
        "cell",
        "update",
        nb_str,
        "--cell-index",
        "0",
        "--source",
        "x = 99",
    ])
    .assert_success();

    let deadline = Instant::now() + Duration::from_secs(10);
    let json = loop {
        let r = ctx.run(&["read", nb_str, "--json"]).assert_success();
        let json: serde_json::Value =
            serde_json::from_str(&r.stdout).expect("read --json must produce valid JSON");
        let src = json["cells"][0]["source"]
            .as_str()
            .or_else(|| {
                json["cells"][0]["source"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");
        if src.contains("x = 99") {
            break json;
        }
        assert!(
            Instant::now() < deadline,
            "Y.js did not sync update-cell to disk within 10s"
        );
        std::thread::sleep(Duration::from_millis(200));
    };

    assert!(
        json["cells"][0]["execution_count"].is_null(),
        "execution_count must be reset to null after source update\nvalue: {:?}",
        json["cells"][0]["execution_count"]
    );
}
