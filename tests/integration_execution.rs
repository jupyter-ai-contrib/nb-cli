mod test_helpers;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper struct to manage test environment
struct TestEnv {
    temp_dir: TempDir,
    binary_path: PathBuf,
    venv_path_env: String,
    venv_root: PathBuf,
}

impl TestEnv {
    fn new() -> Option<Self> {
        // Setup venv and check if execution tests can run
        let venv_root = test_helpers::setup_execution_venv()?;
        let venv_path_env = test_helpers::setup_venv_environment()?;

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let binary_path = env!("CARGO_BIN_EXE_nb").into();

        Some(Self {
            temp_dir,
            binary_path,
            venv_path_env,
            venv_root,
        })
    }

    fn notebook_path(&self, name: &str) -> PathBuf {
        self.temp_dir.path().join(name)
    }

    /// Copy a fixture notebook to the test environment
    fn copy_fixture(&self, fixture_name: &str, dest_name: &str) -> PathBuf {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(fixture_name);
        let dest_path = self.notebook_path(dest_name);
        fs::copy(&fixture_path, &dest_path)
            .unwrap_or_else(|_| panic!("Failed to copy fixture {}", fixture_name));
        dest_path
    }

    /// Copy an entire fixture directory (with subdirectories) to the test environment
    fn copy_fixture_dir(&self, fixture_subdir: &str, dest_name: &str) -> PathBuf {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(fixture_subdir);
        let dest_path = self.notebook_path(dest_name);

        fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
            fs::create_dir_all(dst)?;
            for entry in fs::read_dir(src)? {
                let entry = entry?;
                let ty = entry.file_type()?;
                let src_path = entry.path();
                let dst_path = dst.join(entry.file_name());
                if ty.is_dir() {
                    copy_dir_recursive(&src_path, &dst_path)?;
                } else {
                    fs::copy(&src_path, &dst_path)?;
                }
            }
            Ok(())
        }

        copy_dir_recursive(&fixture_path, &dest_path)
            .unwrap_or_else(|_| panic!("Failed to copy fixture directory {}", fixture_subdir));
        dest_path
    }

    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.binary_path)
            .args(args)
            .current_dir(self.temp_dir.path())
            .env("PATH", &self.venv_path_env)
            .env("VIRTUAL_ENV", &self.venv_root)
            .env_remove("PYTHONHOME") // Remove if set, as it conflicts with venv
            .output()
            .expect("Failed to execute command");

        CommandResult {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            success: output.status.success(),
        }
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

    fn contains(&self, text: &str) -> bool {
        self.stdout.contains(text) || self.stderr.contains(text)
    }
}

// ==================== EXECUTION TESTS ====================

#[test]
fn test_execute_single_cell() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&["cell", "execute", nb_path.to_str().unwrap(), "--cell", "0"])
        .assert_success();

    assert!(result.contains("executed") || result.contains("success"));
}

#[test]
fn test_execute_cell_with_output() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute cell 0 first (defines x)
    env.run(&["cell", "execute", nb_path.to_str().unwrap(), "--cell", "0"])
        .assert_success();

    // Verify output was captured
    let result = env
        .run(&[
            "notebook",
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "0",
            "--with-outputs",
        ])
        .assert_success();

    assert!(result.stdout.contains("outputs"));
}

#[test]
fn test_execute_cell_by_id() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute cell-1 which doesn't depend on other cells
    let result = env
        .run(&[
            "cell",
            "execute",
            nb_path.to_str().unwrap(),
            "--cell-id",
            "cell-1",
        ])
        .assert_success();

    assert!(result.contains("executed") || result.contains("success"));
}

#[test]
fn test_execute_notebook_preserves_state() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute entire notebook to preserve state across cells
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify the output from the last cell
    let result = env
        .run(&[
            "notebook",
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "2",
            "--with-outputs",
        ])
        .assert_success();

    assert!(result.stdout.contains("Result: 52"));
}

#[test]
fn test_execute_entire_notebook() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(
        result.contains("executed") || result.contains("Executed") || result.contains("success")
    );

    // Verify all cells have execution counts
    let read_result = env
        .run(&["notebook", "read", nb_path.to_str().unwrap()])
        .assert_success();

    // Check that execution counts were set
    assert!(read_result.stdout.contains("execution_count"));
}

#[test]
fn test_execute_notebook_with_range() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute only cells 0-1
    env.run(&[
        "notebook",
        "execute",
        nb_path.to_str().unwrap(),
        "--start",
        "0",
        "--end",
        "1",
    ])
    .assert_success();
}

#[test]
fn test_execute_with_error() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("with_error.ipynb", "test.ipynb");

    // Should fail without --allow-errors
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_failure();
}

#[test]
fn test_execute_with_allow_errors() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("with_error.ipynb", "test.ipynb");

    // Execute with --allow-errors (still exits with error code but updates notebook)
    let result = env.run(&[
        "notebook",
        "execute",
        nb_path.to_str().unwrap(),
        "--allow-errors",
    ]);

    // Command fails but should show it executed cells
    assert!(result.contains("Executed") || result.contains("completed"));

    // Verify first cell executed successfully
    let read_result = env
        .run(&["notebook", "read", nb_path.to_str().unwrap(), "--cell", "0"])
        .assert_success();

    assert!(read_result.stdout.contains("execution_count"));
}

#[test]
fn test_execute_with_timeout() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute with custom timeout
    env.run(&[
        "cell",
        "execute",
        nb_path.to_str().unwrap(),
        "--cell",
        "0",
        "--timeout",
        "60",
    ])
    .assert_success();
}

#[test]
fn test_execute_last_cell_with_negative_index() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.notebook_path("test.ipynb");

    // Create a notebook with independent cells
    env.run(&["notebook", "create", nb_path.to_str().unwrap()])
        .assert_success();

    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "x = 10",
    ])
    .assert_success();

    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "result = 2 + 2",
    ])
    .assert_success();

    // Execute last cell using -c -1 (space-separated style)
    // This tests that allow_negative_numbers is properly configured
    let result = env
        .run(&["cell", "execute", nb_path.to_str().unwrap(), "-c", "-1"])
        .assert_success();

    assert!(result.contains("executed") || result.contains("success"));
    assert!(result.contains("Cell index: 1"));
}

#[test]
fn test_execute_dry_run() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute with --dry-run
    env.run(&[
        "cell",
        "execute",
        nb_path.to_str().unwrap(),
        "--cell",
        "0",
        "--dry-run",
    ])
    .assert_success();

    // Verify notebook wasn't modified (no execution count)
    let result = env
        .run(&["notebook", "read", nb_path.to_str().unwrap()])
        .assert_success();

    // Execution count should still be null for dry run
    assert!(result.stdout.contains("\"execution_count\": null"));
}

#[test]
fn test_execute_json_format() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "execute",
            nb_path.to_str().unwrap(),
            "--cell",
            "0",
            "--format",
            "json",
        ])
        .assert_success();

    // Should output valid JSON
    assert!(serde_json::from_str::<serde_json::Value>(&result.stdout).is_ok());
}

// ==================== WORKFLOW TESTS ====================

#[test]
fn test_workflow_create_add_execute() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.notebook_path("workflow.ipynb");

    // Create notebook
    env.run(&["notebook", "create", nb_path.to_str().unwrap()])
        .assert_success();

    // Add cell with code
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "result = 2 + 2",
    ])
    .assert_success();

    // Add another cell that uses the result
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "print(f'Answer: {result}')",
    ])
    .assert_success();

    // Execute entire notebook to preserve state
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify output
    let result = env
        .run(&[
            "notebook",
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "1",
            "--with-outputs",
        ])
        .assert_success();

    assert!(result.stdout.contains("Answer: 4"));
}

#[test]
fn test_workflow_modify_and_reexecute() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute notebook
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Modify a cell
    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell",
        "0",
        "--source",
        "x = 100",
    ])
    .assert_success();

    // Re-execute the notebook
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify the new value propagated
    let result = env
        .run(&[
            "notebook",
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "2",
            "--with-outputs",
        ])
        .assert_success();

    // Should show Result: 110 instead of Result: 52
    assert!(result.stdout.contains("Result: 110"));
}

#[test]
fn test_execute_with_relative_paths() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    // Copy the entire subdir fixture (includes notebook and data/test.txt)
    let subdir_path = env.copy_fixture_dir("subdir", "subdir");
    let nb_path = subdir_path.join("with_relative_path.ipynb");

    // Execute notebook from parent directory (not from subdir)
    // This tests that relative paths in the notebook work correctly
    env.run(&["notebook", "execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify the file was loaded successfully
    let result = env
        .run(&[
            "notebook",
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "0",
            "--with-outputs",
        ])
        .assert_success();

    // Check that it loaded the file and printed the expected output
    assert!(result.stdout.contains("Hello from relative path!"));
}
