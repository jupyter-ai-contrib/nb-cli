//! Connect-mode integration tests.
//!
//! These tests spin up a real Jupyter Server and must be run **single-threaded**
//! to avoid races on the shared server. Always invoke with:
//!
//!   cargo test --test integration_connect_mode -- --test-threads=1
//!
//! By default the server runs jupyter-server-documents (the same venv local-mode
//! tests use). To run against jupyter-collaboration instead, set NB_TEST_BACKEND
//! and use its separate pinned venv (see tests/setup_test_env.sh):
//!
//!   ./tests/setup_test_env.sh jupyter-collaboration
//!   NB_TEST_BACKEND=jupyter-collaboration cargo test --test integration_connect_mode -- --test-threads=1
//!
//! jupyter-collaboration and jupyter-server-documents must never be installed in
//! the same venv (they are competing collaborative-editing extensions), so each
//! backend gets its own venv directory (.test-venv vs .test-venv-collab).

mod test_helpers;

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

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
    // Backend is selected via NB_TEST_BACKEND ("jsd" [default] or
    // "jupyter-collaboration"); this also picks the venv directory
    // (test_helpers::setup_execution_venv), since the two backends must
    // never be installed into the same venv.
    let backend = test_helpers::test_backend();
    let venv_root = test_helpers::setup_execution_venv()?;
    let venv_path_env = test_helpers::setup_venv_environment()?;

    let venv_bin = if cfg!(windows) {
        venv_root.join("Scripts")
    } else {
        venv_root.join("bin")
    };

    // Ensure jupyter_server and the selected collaboration backend are installed
    // (idempotent). Pinned to versions verified to work together (see AGENTS.md):
    // jupyter-server-documents provides the FileID / Y.js API that nb's remote
    // executor relies on for real-time output observation; jupyter-collaboration
    // is the alternate backend PR #99 fixed connect-mode execute against.
    let packages: &[&str] = match backend.as_str() {
        "jupyter-collaboration" | "collab" => {
            &["jupyter_server==2.20.0", "jupyter-collaboration==4.4.1"]
        }
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

    // Verify the `jupyter` binary exists in the venv.
    let jupyter_bin = venv_bin.join("jupyter");
    if !jupyter_bin.exists() {
        eprintln!(
            "⚠️  jupyter binary not found at {} — skipping connect-mode tests",
            jupyter_bin.display()
        );
        return None;
    }

    // Pick a free port.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        listener.local_addr().ok()?.port()
    };

    // Leak the TempDir so the directory persists for the lifetime of the process.
    // The OS will clean up the temp files on process exit.
    let server_root_tmp: &'static TempDir = Box::leak(Box::new(
        TempDir::new().expect("Failed to create server root tmpdir"),
    ));
    let server_root = server_root_tmp.path().to_path_buf();

    let token = "nbtest123".to_string();

    // Spawn the server.
    let mut cmd = Command::new(&jupyter_bin);
    cmd.args([
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
    // jupyter-collaboration's Y-store (.jupyter_ystore.db) is written relative to
    // the process cwd, not --ServerApp.root_dir; without this the shared test
    // server pollutes the crate root with that file on every run.
    .current_dir(&server_root)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());

    // Put the server in its own process group so cleanup can kill the whole
    // group (server + any kernels it spawned), not just the top-level process.
    // Without this, an interrupted test run leaks orphaned kernel processes.
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    let child = cmd.spawn().ok()?;

    // Leak the process guard so it lives until process exit (and kills the server).
    let _guard: &'static mut ServerKillGuard = Box::leak(Box::new(ServerKillGuard { child }));

    let server_url = format!("http://127.0.0.1:{}", port);

    // Poll until the server is ready (max 15 s).
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

/// Kills the child process (and, on Unix, its whole process group) when dropped.
struct ServerKillGuard {
    child: std::process::Child,
}

impl Drop for ServerKillGuard {
    fn drop(&mut self) {
        // The server was spawned with process_group(0), so its pid is also its
        // pgid. Signal the group first to take any kernels it spawned down with
        // it; fall back to killing just the direct child in case `kill` (or the
        // group signal) is unavailable, e.g. on non-Unix platforms.
        #[cfg(unix)]
        {
            let pgid = self.child.id().to_string();
            let _ = Command::new("kill")
                .args(["-TERM", &format!("-{}", pgid)])
                .status();
            std::thread::sleep(Duration::from_millis(300));
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{}", pgid)])
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Per-test helper that wraps the shared server and provides convenience methods.
struct TestCtx {
    info: &'static SharedServerInfo,
}

impl TestCtx {
    fn new() -> Option<Self> {
        shared_server().map(|info| TestCtx { info })
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

    /// Copy a fixture like [`copy_fixture`](Self::copy_fixture), and return a guard
    /// that explicitly deletes the notebook's Jupyter session (and its kernel) via
    /// `DELETE /api/sessions/{id}` when dropped. Tests execute notebooks, which
    /// creates a session/kernel that (by design, matching production semantics)
    /// nb never deletes on its own; without this, kernels accumulate across a
    /// long-running shared-server test run instead of being torn down between tests.
    fn copy_fixture_with_teardown(
        &self,
        fixture_name: &str,
        dest_name: &str,
    ) -> NotebookSession<'_> {
        let path = self.copy_fixture(fixture_name, dest_name);
        NotebookSession {
            ctx: self,
            notebook_path: path,
            dest_name: dest_name.to_string(),
        }
    }

    /// Delete the Jupyter session (and its kernel) for `notebook_name`, if one
    /// exists on the shared server. Best-effort: errors are swallowed since this
    /// is cleanup, not the thing under test.
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

    /// Run `nb` with arbitrary args, automatically appending `--server` and `--token`.
    fn run(&self, args: &[&str]) -> CommandResult {
        self.run_from_dir(args, &self.info.server_root)
    }

    /// Run `nb` from a specific working directory.
    fn run_from_dir(&self, args: &[&str], cwd: &std::path::Path) -> CommandResult {
        let output = Command::new(&self.info.binary_path)
            .args(args)
            .args([
                "--server",
                &self.info.server_url,
                "--token",
                &self.info.token,
            ])
            .current_dir(cwd)
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

/// Guard returned by [`TestCtx::copy_fixture_with_teardown`]. Derefs to the
/// notebook's path; deletes its Jupyter session/kernel on drop so tests don't
/// leak kernels into later tests on the shared server.
struct NotebookSession<'a> {
    ctx: &'a TestCtx,
    notebook_path: PathBuf,
    dest_name: String,
}

impl std::ops::Deref for NotebookSession<'_> {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.notebook_path
    }
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

    fn assert_failure(self) -> Self {
        if self.success {
            panic!(
                "Expected command to fail but it succeeded:\nStdout: {}\nStderr: {}",
                self.stdout, self.stderr
            );
        }
        self
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

    let _notebook =
        ctx.copy_fixture_with_teardown("for_connect_restart.ipynb", "test_preserve.ipynb");

    // First: execute the full notebook to establish kernel state.
    let result = ctx
        .run(&["execute", "test_preserve.ipynb"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );

    // Second: execute only cell-use (index 1) — no restart.
    // The kernel should still have `persistent_var` in scope.
    let result = ctx
        .run(&["execute", "test_preserve.ipynb", "--cell-index", "1"])
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
    let _notebook =
        ctx.copy_fixture_with_teardown("for_connect_restart.ipynb", "test_restart.ipynb");

    // Step 1: run the full notebook to create the session and set state.
    let result = ctx.run(&["execute", "test_restart.ipynb"]).assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );

    // Step 2: run cell-use without restart — variable should still be in scope.
    let result = ctx
        .run(&["execute", "test_restart.ipynb", "--cell-index", "1"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Without restart, cell-use should still find persistent_var\nStdout: {}",
        result.stdout
    );

    // Step 3: run cell-use *with* restart → NameError because the kernel was restarted
    // and `persistent_var` was never re-defined.
    let result = ctx
        .run(&[
            "execute",
            "test_restart.ipynb",
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
    let _notebook =
        ctx.copy_fixture_with_teardown("for_connect_restart.ipynb", "test_restart_full.ipynb");

    // Step 1: initial full execution to create the session.
    ctx.run(&["execute", "test_restart_full.ipynb"])
        .assert_success();

    // Step 2: full re-execution with --restart-kernel.
    // All cells are run in order from scratch, so cell-set runs before cell-use.
    let result = ctx
        .run(&["execute", "test_restart_full.ipynb", "--restart-kernel"])
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Full notebook execution after restart should print 'persistent_var = 999'\nStdout: {}",
        result.stdout
    );
}

/// Execute from a CWD that differs from the server root.
#[test]
fn test_execute_from_different_cwd() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    let _notebook = ctx.copy_fixture_with_teardown("for_connect_restart.ipynb", "test_cwd.ipynb");

    // Run from a temporary directory that is NOT the server root.
    let other_dir = TempDir::new().expect("Failed to create temp dir");
    let result = ctx
        .run_from_dir(&["execute", "test_cwd.ipynb"], other_dir.path())
        .assert_success();

    assert!(
        result.stdout.contains("persistent_var = 999"),
        "Execution from different CWD should read notebook from server and succeed\nStdout: {}",
        result.stdout
    );
}

// ==================== CLEAR OUTPUTS TESTS ====================

/// Clear all outputs from a notebook in connect mode.
///
/// Ignored against jupyter-collaboration: `nb output clear` correctly edits the
/// Y.js room, but jupyter_server_ydoc only flushes the room to disk on a ~1s
/// debounced timer, so the immediate `nb read` below races that debounce and
/// observes stale content. Distinct from #90 (jupyter-server-documents' clear
/// never persists at all, permanently, due to externalized-output
/// re-materialization); see #100 for the jupyter-collaboration mechanism.
#[test]
#[ignore = "jupyter-collaboration: read-after-write races jupyter_server_ydoc's ~1s save debounce, see jupyter-ai-contrib/nb-cli#100"]
fn test_clear_outputs_in_connect_mode() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.copy_fixture("with_outputs.ipynb", "test_clear_all.ipynb");

    // Clear all outputs
    ctx.run(&["output", "clear", "test_clear_all.ipynb"])
        .assert_success();

    // Read back and verify outputs are gone
    let result = ctx
        .run(&["read", "test_clear_all.ipynb", "--json"])
        .assert_success();

    let parsed: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Failed to parse JSON output");

    // All code cells should have empty outputs and null execution_count
    for cell in parsed["cells"].as_array().unwrap() {
        if cell["cell_type"] == "code" {
            let outputs = cell["outputs"].as_array().unwrap();
            assert!(
                outputs.is_empty(),
                "Expected empty outputs after clear, got: {:?}",
                outputs
            );
            assert!(
                cell["execution_count"].is_null(),
                "Expected null execution_count after clear"
            );
        }
    }
}

/// Clear outputs from a specific cell by index in connect mode.
///
/// Ignored against jupyter-collaboration for the same reason as
/// `test_clear_outputs_in_connect_mode` above: see jupyter-ai-contrib/nb-cli#100.
#[test]
#[ignore = "jupyter-collaboration: read-after-write races jupyter_server_ydoc's ~1s save debounce, see jupyter-ai-contrib/nb-cli#100"]
fn test_clear_outputs_specific_cell_in_connect_mode() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.copy_fixture("with_outputs.ipynb", "test_clear_one.ipynb");

    // Clear only cell at index 0
    ctx.run(&[
        "output",
        "clear",
        "test_clear_one.ipynb",
        "--cell-index",
        "0",
    ])
    .assert_success();

    // Read back and verify only cell 0 is cleared
    let result = ctx
        .run(&["read", "test_clear_one.ipynb", "--json"])
        .assert_success();

    let parsed: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Failed to parse JSON output");

    let cells = parsed["cells"].as_array().unwrap();

    // Cell 0 should be cleared
    assert!(
        cells[0]["outputs"].as_array().unwrap().is_empty(),
        "Cell 0 outputs should be cleared"
    );
    assert!(
        cells[0]["execution_count"].is_null(),
        "Cell 0 execution_count should be null"
    );

    // Cell 1 should still have outputs
    assert!(
        !cells[1]["outputs"].as_array().unwrap().is_empty(),
        "Cell 1 outputs should still be present"
    );
}

/// Error when clearing outputs for a non-existent cell ID.
#[test]
fn test_clear_outputs_invalid_cell_id_in_connect_mode() {
    let Some(ctx) = TestCtx::new() else {
        eprintln!("⚠️  Skipping connect-mode test: jupyter server not available");
        return;
    };

    ctx.copy_fixture("with_outputs.ipynb", "test_clear_bad_id.ipynb");

    let result = ctx
        .run(&[
            "output",
            "clear",
            "test_clear_bad_id.ipynb",
            "--cell",
            "nonexistent-id",
        ])
        .assert_failure();

    assert!(
        result.stderr.contains("not found"),
        "Expected 'not found' error message, got stderr: {}",
        result.stderr
    );
}
