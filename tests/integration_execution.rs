mod test_helpers;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use test_helpers::CommandResult;

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

// ==================== EXECUTION TESTS ====================

#[test]
fn test_execute_single_cell() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--cell-index", "0"])
        .assert_success();

    // Execute now returns notebook markdown on stdout
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Execute stdout should contain @@notebook header"
    );
}

#[test]
fn test_execute_cell_with_output() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute entire notebook so cell 2 can print the result
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify output directly in execute stdout
    let outputs = test_helpers::parse_outputs(&exec_result.stdout);
    assert!(
        !outputs.is_empty(),
        "Execute stdout should contain @@output sentinels"
    );
    assert!(
        exec_result.stdout.contains("Result: 52"),
        "Execute stdout should contain the print output"
    );

    // Persistence check: verify via nb read that outputs were saved
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "2"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].get_str("cell_type"), Some("code"));

    let read_outputs = test_helpers::parse_outputs(&result.stdout);
    assert!(
        !read_outputs.is_empty(),
        "Cell should have outputs after execution"
    );

    assert!(result.stdout.contains("Result: 52"));
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
        .run(&["execute", nb_path.to_str().unwrap(), "--cell", "cell-1"])
        .assert_success();

    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Execute stdout should contain @@notebook header"
    );
}

#[test]
fn test_execute_notebook_preserves_state() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute entire notebook to preserve state across cells
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify output directly in execute stdout
    assert!(
        exec_result.stdout.contains("Result: 52"),
        "Execute stdout should contain the computed result"
    );

    // Persistence check: verify via nb read
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "2"])
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

    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Parse cells directly from execute stdout and verify execution counts
    let cells = test_helpers::parse_cells(&exec_result.stdout);
    assert!(!cells.is_empty(), "Should have cells in execute stdout");

    let code_cells: Vec<_> = cells
        .iter()
        .filter(|c| c.get_str("cell_type") == Some("code"))
        .collect();
    assert!(!code_cells.is_empty(), "Should have code cells");

    for cell in &code_cells {
        assert!(
            cell.get_i64("execution_count").is_some(),
            "Code cell at index {:?} should have execution_count after full notebook execution",
            cell.get_i64("index")
        );
    }

    // Persistence check: verify via nb read
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    let read_cells = test_helpers::parse_cells(&read_result.stdout);
    let read_code_cells: Vec<_> = read_cells
        .iter()
        .filter(|c| c.get_str("cell_type") == Some("code"))
        .collect();
    for cell in &read_code_cells {
        assert!(
            cell.get_i64("execution_count").is_some(),
            "Persisted code cell should have execution_count"
        );
    }
}

#[test]
fn test_execute_notebook_with_range() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // for_execution.ipynb: cell 0=(x=42), cell 1=(y=x+10), cell 2=(print result)
    // --start 0 --end 1 must execute cells 0 and 1 only; cell 2 must not run.
    let result = env
        .run(&[
            "execute",
            nb_path.to_str().unwrap(),
            "--start",
            "0",
            "--end",
            "1",
            "--json",
        ])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("--json must produce valid JSON");
    let code_cells: Vec<_> = json["cells"]
        .as_array()
        .expect("cells must be an array")
        .iter()
        .filter(|c| c["cell_type"] == "code")
        .collect();
    assert_eq!(code_cells.len(), 3, "fixture must have 3 code cells");
    assert!(
        code_cells[0]["execution_count"].is_number(),
        "cell 0 must have executed"
    );
    assert!(
        code_cells[1]["execution_count"].is_number(),
        "cell 1 must have executed"
    );
    assert!(
        code_cells[2]["execution_count"].is_null(),
        "cell 2 must NOT have executed (outside range)\nexecution_count: {:?}",
        code_cells[2]["execution_count"]
    );
}

#[test]
fn test_execute_with_restart_kernel() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--restart-kernel"])
        .assert_success();

    assert!(
        result.stdout.contains("Result: 52"),
        "--restart-kernel must still produce correct output\nStdout: {}",
        result.stdout
    );
}

#[test]
fn test_execute_with_error() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("with_error.ipynb", "test.ipynb");

    // Should fail without --allow-errors but still output notebook content
    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_failure();

    // Verify partial results appear in stdout despite failure
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Failed execute should still output @@notebook header"
    );

    let outputs = test_helpers::parse_outputs(&result.stdout);
    assert!(
        !outputs.is_empty(),
        "Failed execute should include outputs (error or successful cells)"
    );
}

#[test]
fn test_execute_with_allow_errors() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("with_error.ipynb", "test.ipynb");

    // Execute with --allow-errors (still exits with error code but updates notebook)
    let result = env.run(&["execute", nb_path.to_str().unwrap(), "--allow-errors"]);

    // Stdout should contain notebook markdown with outputs from both cells
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Should have @@notebook header in stdout"
    );

    let cells = test_helpers::parse_cells(&result.stdout);
    let code_cells: Vec<_> = cells
        .iter()
        .filter(|c| c.get_str("cell_type") == Some("code"))
        .collect();
    assert!(
        !code_cells.is_empty(),
        "Should have code cells in execute output"
    );

    // First cell should have execution_count (it succeeded)
    assert!(
        code_cells[0].get_i64("execution_count").is_some(),
        "First code cell should have execution_count"
    );

    // Persistence check
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0"])
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
        "execute",
        nb_path.to_str().unwrap(),
        "--cell-index",
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
    env.run(&["create", nb_path.to_str().unwrap()])
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

    // Execute last cell using --cell-index -1 (space-separated style)
    // This tests that allow_negative_numbers is properly configured
    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--cell-index", "-1"])
        .assert_success();

    // Execute stdout should contain notebook markdown
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Execute stdout should contain @@notebook header"
    );
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
            "execute",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
        .assert_success();

    // Should output valid JSON with cells and summary
    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Should output valid JSON");
    assert_eq!(json["success"], true, "JSON success field must be true");
    assert!(json.get("cells").is_some(), "JSON should have cells array");
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
    env.run(&["create", nb_path.to_str().unwrap()])
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
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify output directly in execute stdout
    assert!(
        exec_result.stdout.contains("Answer: 4"),
        "Execute stdout should contain the computed answer"
    );

    // Persistence check (cell index 2 because create adds an empty cell at index 0)
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "2"])
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
    env.run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Modify a cell
    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
        "0",
        "--source",
        "x = 100",
    ])
    .assert_success();

    // Re-execute the notebook
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify the new value directly in execute stdout
    assert!(
        exec_result.stdout.contains("Result: 110"),
        "Execute stdout should show Result: 110 after modifying x to 100"
    );

    // Persistence check
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "2"])
        .assert_success();

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
    env.run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify the file was loaded successfully
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0"])
        .assert_success();

    // Check that it loaded the file and printed the expected output
    assert!(result.stdout.contains("Hello from relative path!"));
}

// ==================== OUTPUT FORMAT TESTS ====================

#[test]
fn test_execute_output_matches_read() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    // Execute notebook — stdout should be notebook markdown
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Read the same notebook — stdout should match execute
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Both should produce the same notebook markdown format
    // Compare sentinel structure (headers, cells, outputs) rather than byte-for-byte
    // since output dir paths may differ
    let exec_cells = test_helpers::parse_cells(&exec_result.stdout);
    let read_cells = test_helpers::parse_cells(&read_result.stdout);
    assert_eq!(
        exec_cells.len(),
        read_cells.len(),
        "Execute and read should produce the same number of cells"
    );

    let exec_outputs = test_helpers::parse_outputs(&exec_result.stdout);
    let read_outputs = test_helpers::parse_outputs(&read_result.stdout);
    assert_eq!(
        exec_outputs.len(),
        read_outputs.len(),
        "Execute and read should produce the same number of outputs"
    );

    // Verify output types match
    for (exec_out, read_out) in exec_outputs.iter().zip(read_outputs.iter()) {
        assert_eq!(
            exec_out.get_str("output_type"),
            read_out.get_str("output_type"),
            "Output types should match between execute and read"
        );
    }
}

#[test]
fn test_execute_error_shows_partial_results_and_error() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("with_error.ipynb", "test.ipynb");

    // Execute without --allow-errors: cell-1 (valid_code = 123) succeeds, cell-2 (undefined_variable) fails
    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_failure();

    // Stdout should contain notebook markdown with partial results
    assert!(
        test_helpers::parse_notebook_header(&result.stdout).is_some(),
        "Should have @@notebook header despite failure"
    );

    // Cell-1 should have execution_count (it ran successfully)
    let cells = test_helpers::parse_cells(&result.stdout);
    let code_cells: Vec<_> = cells
        .iter()
        .filter(|c| c.get_str("cell_type") == Some("code"))
        .collect();
    assert!(
        code_cells.len() >= 2,
        "Should have at least 2 code cells in output"
    );
    assert!(
        code_cells[0].get_i64("execution_count").is_some(),
        "First code cell should have execution_count (it succeeded)"
    );

    // Cell-2 should have an error output
    let outputs = test_helpers::parse_outputs(&result.stdout);
    let error_outputs: Vec<_> = outputs
        .iter()
        .filter(|o| o.get_str("output_type") == Some("error"))
        .collect();
    assert!(
        !error_outputs.is_empty(),
        "Should have an error output from the failed cell"
    );
    assert_eq!(
        error_outputs[0].get_str("ename"),
        Some("NameError"),
        "Error should be a NameError"
    );

    // Persistence check: partial results should be saved
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    let read_cells = test_helpers::parse_cells(&read_result.stdout);
    let read_code_cells: Vec<_> = read_cells
        .iter()
        .filter(|c| c.get_str("cell_type") == Some("code"))
        .collect();
    assert!(
        read_code_cells[0].get_i64("execution_count").is_some(),
        "Persisted first cell should have execution_count"
    );
}

#[test]
fn test_execute_json_includes_outputs() {
    let Some(env) = TestEnv::new() else {
        eprintln!("⚠️  Skipping test: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("for_execution.ipynb", "test.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Should output valid JSON");

    // Verify summary fields
    assert_eq!(json["success"], true);
    assert!(json["executed_cells"].as_u64().unwrap() > 0);

    // Verify cells array has outputs
    let cells = json["cells"].as_array().expect("Should have cells array");
    let code_cells: Vec<_> = cells.iter().filter(|c| c["cell_type"] == "code").collect();

    // Last code cell (print) should have outputs
    let last_cell = code_cells.last().expect("Should have code cells");
    let outputs = last_cell["outputs"]
        .as_array()
        .expect("Last code cell should have outputs");
    assert!(!outputs.is_empty(), "Should have at least one output");

    // Verify the actual output content contains the computed result
    let output_text = serde_json::to_string(outputs).unwrap();
    assert!(
        output_text.contains("Result: 52"),
        "Output should contain 'Result: 52'"
    );
}
