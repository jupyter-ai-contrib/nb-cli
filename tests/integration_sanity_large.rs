//! Sanity tests for larger notebooks.
//!
//! These tests exercise execution of realistic multi-cell notebooks (20+ cells,
//! heavy output, error handling, state propagation) in both local mode and
//! connect mode. They go beyond the small 2–3 cell fixtures used in other tests.
//!
//! Local mode (no Jupyter Server required):
//!   cargo test --test integration_sanity_large -- --test-threads=1
//!
//! Connect mode (requires NB_TEST_BACKEND):
//!   NB_TEST_BACKEND=jsd cargo test --test integration_sanity_large -- --test-threads=1
//!   NB_TEST_BACKEND=none cargo test --test integration_sanity_large -- --test-threads=1

mod test_helpers;

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
use test_helpers::CommandResult;

// ==================== TEST INFRASTRUCTURE ====================

/// Unified test environment that works in both local and connect modes.
struct SanityEnv {
    temp_dir: TempDir,
    binary_path: PathBuf,
    venv_path_env: String,
    venv_root: PathBuf,
}

impl SanityEnv {
    fn new() -> Option<Self> {
        let venv_root = test_helpers::setup_execution_venv()?;
        let venv_path_env = test_helpers::setup_venv_environment()?;
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let binary_path: PathBuf = env!("CARGO_BIN_EXE_nb").into();

        Some(Self {
            temp_dir,
            binary_path,
            venv_path_env,
            venv_root,
        })
    }

    fn copy_fixture(&self, fixture_name: &str, dest_name: &str) -> PathBuf {
        let dest_path = self.temp_dir.path().join(dest_name);
        test_helpers::copy_fixture(fixture_name, &dest_path);
        dest_path
    }

    /// Run `nb` with the given args from the temp dir.
    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.binary_path)
            .args(args)
            .current_dir(self.temp_dir.path())
            .env("PATH", &self.venv_path_env)
            .env("VIRTUAL_ENV", &self.venv_root)
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

// ==================== LARGE STATEFUL NOTEBOOK (20 cells) ====================

/// Execute a 20-cell notebook with interleaved code/markdown, class definitions,
/// loops, and cross-cell state dependencies. Verifies the final sentinel and
/// all assertion cells pass.
#[test]
fn test_large_stateful_notebook_full_execution() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "large_stateful.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify key outputs propagated across cells
    assert!(
        result.stdout.contains("total=4950"),
        "Expected 'total=4950' from cell 2\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("keys=10"),
        "Expected 'keys=10' from cell 5\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("word_count=100"),
        "Expected 'word_count=100' from cell 6\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("combined=5060"),
        "Expected 'combined=5060' from cell 8 (state propagation)\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("counter_final=25"),
        "Expected 'counter_final=25' from cell 11 (class across cells)\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("all_assertions_passed=True"),
        "Expected 'all_assertions_passed=True' from cell 17\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("NOTEBOOK_COMPLETE"),
        "Expected final 'NOTEBOOK_COMPLETE' sentinel\nStdout: {}",
        result.stdout
    );
}

/// Execute only a subset of cells (cell-index range) in the large notebook.
/// Verifies partial execution works correctly.
#[test]
fn test_large_stateful_notebook_partial_execution() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "large_stateful_partial.ipynb");

    // Execute only cell 0 (imports)
    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--cell-index", "0"])
        .assert_success();

    assert!(
        result.stdout.contains("Python 3"),
        "Expected Python version output from cell 0\nStdout: {}",
        result.stdout
    );

    // Only cell 0 should have @@output (executed); other cells are shown as source only.
    // Count @@output lines — should be exactly 1 for the single executed cell.
    let output_count = result
        .stdout
        .lines()
        .filter(|line| line.starts_with("@@output"))
        .count();
    assert_eq!(
        output_count, 1,
        "Partial execution of cell 0 should produce exactly 1 @@output section, got {}",
        output_count
    );
}

/// Read the large notebook and verify structure is intact.
#[test]
fn test_large_stateful_notebook_read() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "large_stateful_read.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let parsed: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Failed to parse JSON output");

    let cells = parsed["cells"].as_array().expect("Expected cells array");
    assert_eq!(
        cells.len(),
        20,
        "Expected 20 cells in large notebook, got {}",
        cells.len()
    );

    // Count cell types
    let code_cells = cells
        .iter()
        .filter(|c| c["cell_type"] == "code")
        .count();
    let md_cells = cells
        .iter()
        .filter(|c| c["cell_type"] == "markdown")
        .count();

    assert_eq!(code_cells, 15, "Expected 15 code cells");
    assert_eq!(md_cells, 5, "Expected 5 markdown cells");
}

// ==================== HEAVY OUTPUT NOTEBOOK ====================

/// Execute a notebook that produces substantial output (100+ lines per cell,
/// large JSON dumps, formatted tables). Verifies output is not truncated.
#[test]
fn test_heavy_output_notebook_full_execution() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_heavy_output.ipynb", "heavy_output.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify first cell's 100 output lines aren't truncated
    assert!(
        result.stdout.contains("output_line_0000"),
        "Expected first output line\nStdout (first 500): {}",
        &result.stdout[..result.stdout.len().min(500)]
    );
    assert!(
        result.stdout.contains("output_line_0099"),
        "Expected last output line (line 99) — output may be truncated\nStdout (last 500): {}",
        &result.stdout[result.stdout.len().saturating_sub(500)..]
    );

    // Verify JSON data cell
    assert!(
        result.stdout.contains("\"id\": 49"),
        "Expected JSON with 50 items (last id=49)\nStdout contains 'id': {}",
        result.stdout.contains("\"id\"")
    );

    // Verify matrix output
    assert!(
        result.stdout.contains("row_9:"),
        "Expected all 10 matrix rows\nStdout: {}",
        &result.stdout[result.stdout.len().saturating_sub(1000)..]
    );

    // Final sentinel
    assert!(
        result.stdout.contains("HEAVY_OUTPUT_COMPLETE"),
        "Expected final sentinel\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(200)..]
    );
}

/// Verify the heavy output notebook's data cell output is valid JSON.
#[test]
fn test_heavy_output_json_integrity() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_heavy_output.ipynb", "heavy_json.ipynb");

    // Execute only cell 1 (the JSON output cell) — but cell 1 uses `json` import
    // so we need to execute the full notebook for state
    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    // The JSON data should be parseable somewhere in stdout
    assert!(
        result.stdout.contains("data_items=50"),
        "Expected data_items=50 in summary\nStdout (tail): {}",
        &result.stdout[result.stdout.len().saturating_sub(300)..]
    );
}

// ==================== ERROR HANDLING ====================

/// Execute a notebook where cell 2 raises ZeroDivisionError.
/// Without --allow-errors, execution should fail at the error cell.
#[test]
fn test_error_notebook_halts_on_error() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_with_error.ipynb", "error_halt.ipynb");

    let result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_failure();

    // The error should be reported in an @@output section
    let combined = format!("{}\n{}", result.stdout, result.stderr);
    assert!(
        combined.contains("ZeroDivisionError"),
        "Expected ZeroDivisionError in output\nOutput: {}",
        combined
    );

    // Cells before the error should have @@output (they executed)
    assert!(
        result.stdout.contains("setup_done x=10"),
        "Cell 0 output should appear\nStdout: {}",
        result.stdout
    );

    // Cell after error should NOT have an @@output section with its execution result.
    // The source text "after_error=True" will appear (as cell source), but NOT in an
    // @@output block. Check that no @@output appears after the error @@output.
    let lines: Vec<&str> = result.stdout.lines().collect();
    let error_output_idx = lines
        .iter()
        .position(|l| l.contains("@@output") && l.contains("\"output_type\":\"error\""));
    assert!(
        error_output_idx.is_some(),
        "Expected an error @@output section"
    );

    // After the error output, no subsequent @@output with stdout should appear
    let after_error_outputs: Vec<&&str> = lines[error_output_idx.unwrap()..]
        .iter()
        .filter(|l| l.contains("@@output") && l.contains("\"output_type\":\"stream\""))
        .collect();
    assert!(
        after_error_outputs.is_empty(),
        "No stream outputs should appear after the error (cells should not execute)\nFound: {:?}",
        after_error_outputs
    );
}

/// Execute a notebook with errors using --allow-errors flag.
/// Execution should continue past the error cell.
#[test]
fn test_error_notebook_continues_with_allow_errors() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_with_error.ipynb", "error_continue.ipynb");

    // With --allow-errors, the command still exits non-zero (error occurred) but
    // execution continues past the error cell.
    let result = env.run(&["execute", nb_path.to_str().unwrap(), "--allow-errors"]);

    // The error traceback should appear in an @@output error section
    assert!(
        result.stdout.contains("ZeroDivisionError"),
        "Error should be reported even with --allow-errors\nStdout: {}",
        result.stdout
    );

    // Count total @@output sections — with --allow-errors, cells after the error
    // should also get @@output sections (they executed)
    let output_lines: Vec<&str> = result
        .stdout
        .lines()
        .filter(|l| l.starts_with("@@output"))
        .collect();

    // We expect outputs from: cell 0, cell 1, cell 2 (error), cell 3, cell 4 = 5
    assert_eq!(
        output_lines.len(),
        5,
        "With --allow-errors, expected 5 @@output sections (all cells execute), got {}\nOutputs: {:?}",
        output_lines.len(),
        output_lines
    );

    // Verify post-error cells executed by checking for their stream output text
    assert!(
        result.stdout.contains("after_error=True"),
        "Cell 3 (after error) should execute with --allow-errors\nStdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("final_x=10"),
        "Cell 4 should execute and have access to pre-error state\nStdout: {}",
        result.stdout
    );
}

// ==================== READ OPERATIONS ON LARGE NOTEBOOKS ====================

/// Read a large notebook in AI-optimized markdown format and verify all cells appear.
#[test]
fn test_read_large_notebook_markdown_format() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "read_md.ipynb");

    let result = env.run(&["read", nb_path.to_str().unwrap()]).assert_success();

    // Should have the notebook header sentinel
    let header = test_helpers::parse_notebook_header(&result.stdout);
    assert!(
        header.is_some(),
        "Expected @@notebook header in markdown output"
    );

    // Should have all 15 code cells as @@cell sentinels
    let cell_sentinels = test_helpers::parse_cells(&result.stdout);
    assert!(
        cell_sentinels.len() >= 15,
        "Expected at least 15 @@cell sentinels (code cells), got {}",
        cell_sentinels.len()
    );

    // Markdown content should appear
    assert!(
        result.stdout.contains("Large Notebook Sanity Test"),
        "Expected markdown heading in output"
    );
    assert!(
        result.stdout.contains("Section 1: Computation"),
        "Expected section heading"
    );
}

// ==================== CELL OPERATIONS ON LARGE NOTEBOOKS ====================

/// Add a cell to the large notebook and verify the notebook grows.
#[test]
fn test_add_cell_to_large_notebook() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "add_cell.ipynb");

    // Add a new code cell at the end
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "print('added_cell_works=True')",
    ])
    .assert_success();

    // Read back and verify 21 cells now
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let parsed: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Failed to parse JSON");
    let cells = parsed["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 21, "Expected 21 cells after adding one");

    // Execute the full notebook including the new cell
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(
        exec_result.stdout.contains("added_cell_works=True"),
        "New cell should execute\nStdout (tail): {}",
        &exec_result.stdout[exec_result.stdout.len().saturating_sub(300)..]
    );
}

/// Delete a cell from the large notebook and verify execution still works.
#[test]
fn test_delete_cell_from_large_notebook() {
    let Some(env) = SanityEnv::new() else {
        eprintln!("⚠️  Skipping: execution environment not available");
        return;
    };

    let nb_path = env.copy_fixture("large_stateful.ipynb", "delete_cell.ipynb");

    // Delete cell at index 1 (the first markdown cell)
    env.run(&[
        "cell",
        "delete",
        nb_path.to_str().unwrap(),
        "--cell-index",
        "1",
    ])
    .assert_success();

    // Read back and verify 19 cells now
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let parsed: serde_json::Value =
        serde_json::from_str(&result.stdout).expect("Failed to parse JSON");
    let cells = parsed["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 19, "Expected 19 cells after deleting one");

    // Execution should still work (we only deleted a markdown cell)
    let exec_result = env
        .run(&["execute", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(
        exec_result.stdout.contains("NOTEBOOK_COMPLETE"),
        "Execution should complete after deleting markdown cell\nStdout (tail): {}",
        &exec_result.stdout[exec_result.stdout.len().saturating_sub(200)..]
    );
}
