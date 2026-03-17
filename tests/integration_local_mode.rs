use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper struct to manage test environment
struct TestEnv {
    temp_dir: TempDir,
    binary_path: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let binary_path = env!("CARGO_BIN_EXE_nb").into();
        Self {
            temp_dir,
            binary_path,
        }
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

    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.binary_path)
            .args(args)
            .current_dir(self.temp_dir.path())
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

    fn json_value(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout).expect("Failed to parse JSON output")
    }
}

/// Helper to join source array from JSON
fn join_source(source: &serde_json::Value) -> String {
    source
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("")
}

// ==================== NOTEBOOK CREATE TESTS ====================

#[test]
fn test_create_empty_notebook() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("test.ipynb");

    let result = env
        .run(&["create", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["template"], "empty");
    assert_eq!(json["kernel"], "python3");
    assert_eq!(json["cell_count"], 0);
    assert!(nb_path.exists());
}

#[test]
fn test_create_basic_notebook() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("basic.ipynb");

    let result = env
        .run(&[
            "create",
            nb_path.to_str().unwrap(),
            "--template",
            "basic", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["template"], "basic");
    assert_eq!(json["cell_count"], 1);
    assert!(nb_path.exists());
}

#[test]
fn test_create_markdown_notebook() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("markdown.ipynb");

    let result = env
        .run(&[
            "create",
            nb_path.to_str().unwrap(),
            "--template",
            "markdown", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["template"], "markdown");
    assert_eq!(json["cell_count"], 2);
}

#[test]
fn test_create_with_custom_kernel() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("custom.ipynb");

    let result = env
        .run(&[
            "create",
            nb_path.to_str().unwrap(),
            "--kernel",
            "python3.11",
            "--language",
            "python", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["kernel"], "python3.11");
}

#[test]
fn test_create_without_ipynb_extension() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("test");

    env.run(&["create", nb_path.to_str().unwrap()])
        .assert_success();

    // Should automatically add .ipynb extension
    assert!(env.notebook_path("test.ipynb").exists());
}

#[test]
fn test_create_fails_if_exists() {
    let env = TestEnv::new();
    env.copy_fixture("basic.ipynb", "exists.ipynb");
    let nb_path = env.notebook_path("exists.ipynb");

    // Creating again without --force should fail
    env.run(&["create", nb_path.to_str().unwrap()])
        .assert_failure();
}

#[test]
fn test_create_with_force_overwrites() {
    let env = TestEnv::new();
    env.copy_fixture("with_code.ipynb", "overwrite.ipynb");
    let nb_path = env.notebook_path("overwrite.ipynb");

    // Creating again with --force should succeed
    let result = env
        .run(&["create", nb_path.to_str().unwrap(), "--force", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_count"], 0); // Should be empty now
}

#[test]
fn test_create_text_format() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("test.ipynb");

    let result = env
        .run(&["create", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(result.stdout.contains("Created notebook:"));
    assert!(result.stdout.contains("Template:"));
    assert!(result.stdout.contains("Kernel:"));
}

// ==================== NOTEBOOK READ TESTS ====================

#[test]
fn test_read_empty_notebook() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_count"], 0);
    assert_eq!(json["cells"].as_array().unwrap().len(), 0);
}

#[test]
fn test_read_notebook_with_cells() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells"].as_array().unwrap().len(), 2);
}

#[test]
fn test_read_specific_cell_by_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_type"], "code");
    assert!(json["source"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("print")));
}

#[test]
fn test_read_last_cell_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert!(json["source"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s.as_str().unwrap().contains("print")));
}

#[test]
fn test_read_cell_by_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "cell-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["id"], "cell-1");
}

#[test]
fn test_read_with_outputs() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--with-outputs", "--json"])
        .assert_success();

    let json = result.json_value();
    let cells = json["cells"].as_array().unwrap();
    assert!(cells[0]["outputs"].as_array().unwrap().len() > 0);
}

#[test]
fn test_read_only_code() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--only-code", "--json"])
        .assert_success();

    let json = result.json_value();
    let cells = json["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2); // Only 2 code cells
    for cell in cells {
        assert_eq!(cell["cell_type"], "code");
    }
}

#[test]
fn test_read_only_markdown() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--only-markdown", "--json"])
        .assert_success();

    let json = result.json_value();
    let cells = json["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2); // Only 2 markdown cells
    for cell in cells {
        assert_eq!(cell["cell_type"], "markdown");
    }
}

#[test]
fn test_read_text_format() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(result.stdout.contains("Cell"));
    assert!(result.stdout.contains("print"));
}

#[test]
fn test_read_nonexistent_notebook_fails() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("nonexistent.ipynb");

    env.run(&["read", nb_path.to_str().unwrap()])
        .assert_failure();
}

// ==================== CELL ADD TESTS ====================

#[test]
fn test_add_code_cell() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "x = 1 + 1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_type"], "code");
    assert_eq!(json["index"], 0);
    assert_eq!(json["total_cells"], 1);
}

#[test]
fn test_add_markdown_cell() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--type",
            "markdown",
            "--source",
            "# Hello World", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_type"], "markdown");
}

#[test]
fn test_add_raw_cell() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--type",
            "raw",
            "--source",
            "Raw content", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_type"], "raw");
}

#[test]
fn test_add_cell_with_multiline_source() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "def hello():\n    print('world')\n\nhello()", "--json"])
    .assert_success();

    // Verify the cell was added correctly
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();
    let json = result.json_value();
    let cells = json["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 1);
}

#[test]
fn test_add_cell_at_beginning() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "inserted at start",
            "--insert-at",
            "0", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 0);
    assert_eq!(json["total_cells"], 3);
}

#[test]
fn test_add_cell_at_end() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "appended", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 2);
}

#[test]
fn test_add_cell_with_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "before last",
            "--insert-at",
            "-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 1); // Should be inserted before the last cell
}

#[test]
fn test_add_cell_after_cell_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "after cell-1",
            "--after",
            "cell-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 1);
}

#[test]
fn test_add_cell_before_cell_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "before cell-2",
            "--before",
            "cell-2", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 1);
}

#[test]
fn test_add_cell_with_custom_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "test",
            "--id",
            "my-custom-id", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_id"], "my-custom-id");
}

#[test]
fn test_add_cell_duplicate_id_fails() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    // cell-1 already exists
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "duplicate",
        "--id",
        "cell-1"])
    .assert_failure();
}

#[test]
fn test_add_cell_empty_source() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&["cell", "add", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["index"], 0);
}

// ==================== CELL UPDATE TESTS ====================

#[test]
fn test_update_cell_source() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
            "0",
        "--source",
        "y = 2 + 2", "--json"])
    .assert_success();

    // Verify the update
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0", "--json"])
        .assert_success();
    let json = result.json_value();
    let source = join_source(&json["source"]);
    assert!(source.contains("y = 2 + 2"));
}

#[test]
fn test_update_cell_append() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
            "0",
        "--append",
        "\nprint('appended')", "--json"])
    .assert_success();

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0", "--json"])
        .assert_success();
    let json = result.json_value();
    let source = join_source(&json["source"]);
    assert!(source.contains("x = 1 + 1"));
    assert!(source.contains("print('appended')"));
}

#[test]
fn test_update_cell_by_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell",
        "cell-1",
        "--source",
        "updated via id"])
    .assert_success();

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "cell-1", "--json"])
        .assert_success();
    let json = result.json_value();
    let source = join_source(&json["source"]);
    assert!(source.contains("updated via id"));
}

#[test]
fn test_update_cell_type() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
            "0",
        "--type",
        "markdown", "--json"])
    .assert_success();

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0", "--json"])
        .assert_success();
    let json = result.json_value();
    assert_eq!(json["cell_type"], "markdown");
}

#[test]
fn test_update_cell_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
        "-1",
        "--source",
        "updated last cell", "--json"])
    .assert_success();

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "-1", "--json"])
        .assert_success();
    let json = result.json_value();
    let source = join_source(&json["source"]);
    assert!(source.contains("updated last cell"));
}

// ==================== CELL DELETE TESTS ====================

#[test]
fn test_delete_cell_by_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["cell", "delete", nb_path.to_str().unwrap(), "--cell-index", "0", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_deleted"], 1);
    assert_eq!(json["remaining_cells"], 1);
}

#[test]
fn test_delete_cell_by_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "delete",
            nb_path.to_str().unwrap(),
            "--cell",
            "cell-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_deleted"], 1);
    assert_eq!(json["remaining_cells"], 1);
}

#[test]
fn test_delete_multiple_cells() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "cell",
            "delete",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--cell-index",
            "2", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_deleted"], 2);
    assert_eq!(json["remaining_cells"], 3);
}

#[test]
fn test_delete_with_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["cell", "delete", nb_path.to_str().unwrap(), "--cell-index", "-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["remaining_cells"], 1);
}

#[test]
fn test_delete_all_cells_fails() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    // Should fail because we can't delete all cells
    env.run(&[
        "cell",
        "delete",
        nb_path.to_str().unwrap(),
        "--cell-index",
            "0",
        "--cell-index",
            "1"])
    .assert_failure();
}

// ==================== SEARCH TESTS ====================

#[test]
fn test_search_finds_pattern() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["search", nb_path.to_str().unwrap(), "import", "--json"])
        .assert_success();

    let json = result.json_value();
    assert!(json["results"].as_array().unwrap().len() > 0);
    assert!(json["total_matches"].as_u64().unwrap() > 0);
}

#[test]
fn test_search_case_insensitive() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "search",
            nb_path.to_str().unwrap(),
            "PANDAS",
            "-i", "--json"])
        .assert_success();

    let json = result.json_value();
    assert!(json["results"].as_array().unwrap().len() > 0);
}

#[test]
fn test_search_no_matches() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "search",
            nb_path.to_str().unwrap(),
            "nonexistent_pattern", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["results"].as_array().unwrap().len(), 0);
    assert_eq!(json["total_matches"], 0);
}

#[test]
fn test_search_multiple_matches() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    // Search for "import" which appears twice in code-1 cell
    let result = env
        .run(&["search", nb_path.to_str().unwrap(), "import", "--json"])
        .assert_success();

    let json = result.json_value();
    assert!(json["results"].as_array().unwrap().len() > 0);
    assert_eq!(json["total_matches"], 2);
}

// ==================== OUTPUT CLEAR TESTS ====================

#[test]
fn test_clear_outputs() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["output", "clear", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_cleared"], 2);

    // Verify outputs are cleared
    let read_result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--with-outputs", "--json"])
        .assert_success();
    let read_json = read_result.json_value();
    let cells = read_json["cells"].as_array().unwrap();
    for cell in cells {
        if cell["cell_type"] == "code" {
            assert_eq!(cell["outputs"].as_array().unwrap().len(), 0);
        }
    }
}

#[test]
fn test_clear_outputs_empty_notebook() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&["output", "clear", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_cleared"], 0);
}

#[test]
fn test_clear_outputs_specific_cell() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["output", "clear", nb_path.to_str().unwrap(), "--cell-index", "0", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_cleared"], 1);
}

#[test]
fn test_clear_outputs_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["output", "clear", nb_path.to_str().unwrap(), "--cell-index", "-1", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_cleared"], 1);
}

// ==================== COMPLEX WORKFLOWS ====================

#[test]
fn test_workflow_create_add_read() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("workflow.ipynb");

    // Create
    env.run(&["create", nb_path.to_str().unwrap()])
        .assert_success();

    // Add cells
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--type",
        "markdown",
        "--source",
        "# Workflow Test"])
    .assert_success();

    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "x = 42", "--json"])
    .assert_success();

    // Read
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells"].as_array().unwrap().len(), 2);
}

#[test]
fn test_workflow_modify_and_verify() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    // Update first cell
    env.run(&[
        "cell",
        "update",
        nb_path.to_str().unwrap(),
        "--cell-index",
            "0",
        "--source",
        "modified = True"])
    .assert_success();

    // Delete second cell
    env.run(&["cell", "delete", nb_path.to_str().unwrap(), "--cell-index", "1"])
        .assert_success();

    // Add new cell
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "new_cell = 123", "--json"])
    .assert_success();

    // Verify
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();
    let json = result.json_value();
    assert_eq!(json["cells"].as_array().unwrap().len(), 2);
}
