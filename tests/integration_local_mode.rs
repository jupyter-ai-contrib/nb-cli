mod test_helpers;

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
fn test_create_default_notebook() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("test.ipynb");

    let result = env
        .run(&["create", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["kernel"], "python3");
    assert_eq!(json["cell_count"], 1);
    assert!(nb_path.exists());

    // Verify it's a code cell
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();
    let read_json = read_result.json_value();
    assert_eq!(read_json["cells"][0]["cell_type"], "code");
}

#[test]
fn test_create_with_markdown_flag() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("markdown.ipynb");

    let result = env
        .run(&["create", nb_path.to_str().unwrap(), "--markdown", "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cell_count"], 1);

    // Verify it's a markdown cell
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();
    let read_json = read_result.json_value();
    assert_eq!(read_json["cells"][0]["cell_type"], "markdown");
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
            "python",
            "--json",
        ])
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
    assert_eq!(json["cell_count"], 1); // Should have one code cell now
}

#[test]
fn test_create_text_format() {
    let env = TestEnv::new();
    let nb_path = env.notebook_path("test.ipynb");

    let result = env
        .run(&["create", nb_path.to_str().unwrap()])
        .assert_success();

    assert!(result.stdout.contains("Created notebook:"));
    assert!(result.stdout.contains("Kernel:"));
    assert!(result.stdout.contains("Cells:"));
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
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "1",
            "--json",
        ])
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
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "-1",
            "--json",
        ])
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
            "cell-1",
            "--json",
        ])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["id"], "cell-1");
}

#[test]
fn test_read_with_outputs() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
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
            "--only-markdown",
            "--json",
        ])
        .assert_success();

    let json = result.json_value();
    let cells = json["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 2); // Only 2 markdown cells
    for cell in cells {
        assert_eq!(cell["cell_type"], "markdown");
    }
}

#[test]
fn test_read_only_code_and_only_markdown_conflict() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env.run(&[
        "read",
        nb_path.to_str().unwrap(),
        "--only-code",
        "--only-markdown",
    ]);
    assert!(!result.success);
}

#[test]
fn test_read_markdown_format() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify notebook header
    let header =
        test_helpers::parse_notebook_header(&result.stdout).expect("Should have @@notebook header");
    assert_eq!(header.get_str("format"), Some("ai-notebook"));

    // Verify cells
    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 2);

    // First cell
    assert_eq!(cells[0].get_str("cell_type"), Some("code"));
    assert_eq!(cells[0].get_str("id"), Some("cell-1"));
    assert_eq!(cells[0].get_i64("index"), Some(0));

    // Second cell
    assert_eq!(cells[1].get_str("cell_type"), Some("code"));
    assert_eq!(cells[1].get_str("id"), Some("cell-2"));
    assert_eq!(cells[1].get_i64("index"), Some(1));

    // Verify source content is present
    assert!(result.stdout.contains("x = 1 + 1"));
    assert!(result.stdout.contains("print"));
}

#[test]
fn test_read_markdown_format_with_outputs() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Verify notebook header exists
    let header =
        test_helpers::parse_notebook_header(&result.stdout).expect("Should have @@notebook header");
    assert_eq!(header.get_str("format"), Some("ai-notebook"));

    // Verify cells exist
    let cells = test_helpers::parse_cells(&result.stdout);
    assert!(!cells.is_empty());

    // Verify outputs are present (outputs included by default)
    let outputs = test_helpers::parse_outputs(&result.stdout);
    assert!(!outputs.is_empty(), "Outputs should be included by default");

    // Verify output has expected fields
    let first_output = &outputs[0];
    assert!(
        first_output.get_str("output_type").is_some(),
        "Output should have output_type"
    );
}

#[test]
fn test_read_markdown_format_no_output() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--no-output"])
        .assert_success();

    // Cells should still be present
    let cells = test_helpers::parse_cells(&result.stdout);
    assert!(!cells.is_empty());

    // But no outputs
    let outputs = test_helpers::parse_outputs(&result.stdout);
    assert!(
        outputs.is_empty(),
        "Outputs should be excluded with --no-output"
    );
}

#[test]
fn test_read_markdown_format_only_code() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--only-code"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 2);
    for cell in &cells {
        assert_eq!(cell.get_str("cell_type"), Some("code"));
    }
}

#[test]
fn test_read_markdown_format_only_markdown() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--only-markdown"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 2);
    for cell in &cells {
        assert_eq!(cell.get_str("cell_type"), Some("markdown"));
    }
}

#[test]
fn test_read_markdown_format_single_cell() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "0"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].get_str("cell_type"), Some("code"));
    assert_eq!(cells[0].get_str("id"), Some("cell-1"));
}

#[test]
fn test_read_markdown_format_cell_by_id() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell", "cell-2"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].get_str("id"), Some("cell-2"));
}

#[test]
fn test_read_markdown_format_empty_notebook() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Should have notebook header but no cells
    let header =
        test_helpers::parse_notebook_header(&result.stdout).expect("Should have @@notebook header");
    assert_eq!(header.get_str("format"), Some("ai-notebook"));

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 0);
}

#[test]
fn test_search_markdown_format() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["search", nb_path.to_str().unwrap(), "import"])
        .assert_success();

    // Search results should include notebook header and matching cells
    let header = test_helpers::parse_notebook_header(&result.stdout)
        .expect("Search should output @@notebook header");
    assert_eq!(header.get_str("format"), Some("ai-notebook"));

    let cells = test_helpers::parse_cells(&result.stdout);
    assert!(!cells.is_empty(), "Should find matching cells");
    for cell in &cells {
        assert_eq!(cell.get_str("cell_type"), Some("code"));
    }

    // Should include summary comment
    assert!(result.stdout.contains("# Found"));
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
            "x = 1 + 1",
            "--json",
        ])
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
            "# Hello World",
            "--json",
        ])
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
            "Raw content",
            "--json",
        ])
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
        "def hello():\n    print('world')\n\nhello()",
        "--json",
    ])
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
            "0",
            "--json",
        ])
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
            "appended",
            "--json",
        ])
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
            "-1",
            "--json",
        ])
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
            "cell-1",
            "--json",
        ])
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
            "cell-2",
            "--json",
        ])
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
            "my-custom-id",
            "--json",
        ])
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
        "cell-1",
    ])
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

#[test]
fn test_add_consecutive_cells_correct_count() {
    // Regression test for issue #24 - cell count should be correct after each addition
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("empty.ipynb", "test.ipynb");

    // First cell
    let result1 = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "a = 10",
            "--json",
        ])
        .assert_success();
    let json1 = result1.json_value();
    assert_eq!(json1["index"], 0);
    assert_eq!(json1["total_cells"], 1);

    // Second cell
    let result2 = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "b = 20",
            "--json",
        ])
        .assert_success();
    let json2 = result2.json_value();
    assert_eq!(json2["index"], 1);
    assert_eq!(json2["total_cells"], 2);

    // Third cell
    let result3 = env
        .run(&[
            "cell",
            "add",
            nb_path.to_str().unwrap(),
            "--source",
            "c = 30",
            "--json",
        ])
        .assert_success();
    let json3 = result3.json_value();
    assert_eq!(json3["index"], 2);
    assert_eq!(json3["total_cells"], 3);
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
        "y = 2 + 2",
        "--json",
    ])
    .assert_success();

    // Verify the update
    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
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
        "\nprint('appended')",
        "--json",
    ])
    .assert_success();

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
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
        "updated via id",
    ])
    .assert_success();

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell",
            "cell-1",
            "--json",
        ])
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
        "markdown",
        "--json",
    ])
    .assert_success();

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
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
        "updated last cell",
        "--json",
    ])
    .assert_success();

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "-1",
            "--json",
        ])
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
        .run(&[
            "cell",
            "delete",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
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
            "cell-1",
            "--json",
        ])
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
            "2",
            "--json",
        ])
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
        .run(&[
            "cell",
            "delete",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "-1",
            "--json",
        ])
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
        "1",
    ])
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
            "-i",
            "--json",
        ])
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
            "nonexistent_pattern",
            "--json",
        ])
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
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
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
        .run(&[
            "output",
            "clear",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "0",
            "--json",
        ])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells_cleared"], 1);
}

#[test]
fn test_clear_outputs_negative_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&[
            "output",
            "clear",
            nb_path.to_str().unwrap(),
            "--cell-index",
            "-1",
            "--json",
        ])
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
        "# Workflow Test",
    ])
    .assert_success();

    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "x = 42",
        "--json",
    ])
    .assert_success();

    // Read
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();

    let json = result.json_value();
    assert_eq!(json["cells"].as_array().unwrap().len(), 3);
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
        "modified = True",
    ])
    .assert_success();

    // Delete second cell
    env.run(&[
        "cell",
        "delete",
        nb_path.to_str().unwrap(),
        "--cell-index",
        "1",
    ])
    .assert_success();

    // Add new cell
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "new_cell = 123",
        "--json",
    ])
    .assert_success();

    // Verify
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--json"])
        .assert_success();
    let json = result.json_value();
    assert_eq!(json["cells"].as_array().unwrap().len(), 2);
}

// ==================== INDEX PRESERVATION TESTS ====================

#[test]
fn test_read_only_code_preserves_original_indices() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    // mixed_cells has: markdown(0), code(1), markdown(2), code(3), raw(4)
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--only-code"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 2);

    // Indices should be the ORIGINAL notebook indices, not 0,1
    assert_eq!(cells[0].get_i64("index"), Some(1));
    assert_eq!(cells[1].get_i64("index"), Some(3));
}

#[test]
fn test_read_only_markdown_preserves_original_indices() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--only-markdown"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 2);

    // Original indices: markdown(0), markdown(2)
    assert_eq!(cells[0].get_i64("index"), Some(0));
    assert_eq!(cells[1].get_i64("index"), Some(2));
}

#[test]
fn test_read_single_cell_preserves_original_index() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_code.ipynb", "test.ipynb");

    // Read cell at index 1 (second cell)
    let result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "1"])
        .assert_success();

    let cells = test_helpers::parse_cells(&result.stdout);
    assert_eq!(cells.len(), 1);
    // Should report index 1, not 0
    assert_eq!(cells[0].get_i64("index"), Some(1));
}

// ==================== OUTPUT EXTERNALIZATION TESTS ====================

#[test]
fn test_read_with_output_dir_externalizes_large_output() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_rich_outputs.ipynb", "test.ipynb");
    let output_dir = env.temp_dir.path().join("outputs");

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--limit",
            "100", // very small limit to force externalization
        ])
        .assert_success();

    // The large output cell should have an externalized path
    let outputs = test_helpers::parse_outputs(&result.stdout);
    assert!(!outputs.is_empty());

    // Find the stream output - it should have a path since it exceeds --limit
    let stream_output = outputs
        .iter()
        .find(|o| o.get_str("output_type") == Some("stream"));
    assert!(stream_output.is_some(), "Should have a stream output");
    let path = stream_output.unwrap().get_str("path");
    assert!(
        path.is_some(),
        "Large output should be externalized with a path"
    );

    // The file should actually exist on disk
    let path_str = path.unwrap();
    assert!(
        std::path::Path::new(path_str).exists(),
        "Externalized file should exist at: {}",
        path_str
    );
}

#[test]
fn test_read_inline_limit_controls_externalization() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");
    let output_dir = env.temp_dir.path().join("outputs");

    // With a high limit, small outputs stay inline (no path field)
    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
            "--limit",
            "100000",
        ])
        .assert_success();

    let outputs = test_helpers::parse_outputs(&result.stdout);
    for o in &outputs {
        assert!(
            o.get_str("path").is_none(),
            "Small outputs should stay inline with high --limit"
        );
    }
}

#[test]
fn test_read_binary_output_externalized() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_rich_outputs.ipynb", "test.ipynb");
    let output_dir = env.temp_dir.path().join("outputs");

    let result = env
        .run(&[
            "read",
            nb_path.to_str().unwrap(),
            "--output-dir",
            output_dir.to_str().unwrap(),
        ])
        .assert_success();

    // Find the image output
    let outputs = test_helpers::parse_outputs(&result.stdout);
    let image_output = outputs.iter().find(|o| {
        o.get_str("mime")
            .map(|m| m.starts_with("image/"))
            .unwrap_or(false)
    });
    assert!(image_output.is_some(), "Should have an image output");

    let path = image_output.unwrap().get_str("path");
    assert!(
        path.is_some(),
        "Binary output should always be externalized"
    );
    assert!(
        std::path::Path::new(path.unwrap()).exists(),
        "Externalized image file should exist"
    );
}

// ==================== ERROR OUTPUT IN MARKDOWN TESTS ====================

#[test]
fn test_read_error_output_markdown() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_rich_outputs.ipynb", "test.ipynb");

    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Find the error output
    let outputs = test_helpers::parse_outputs(&result.stdout);
    let error_output = outputs
        .iter()
        .find(|o| o.get_str("output_type") == Some("error"));
    assert!(error_output.is_some(), "Should have an error output");
    assert_eq!(error_output.unwrap().get_str("ename"), Some("ValueError"));
    assert_eq!(
        error_output.unwrap().get_str("evalue"),
        Some("invalid literal")
    );

    // Traceback should be present inline
    assert!(result.stdout.contains("Traceback"));
}

// ==================== OUTPUT CLEAN TESTS ====================

#[test]
fn test_output_clean_command() {
    let env = TestEnv::new();

    // First, create some externalized output by reading a notebook
    let nb_path = env.copy_fixture("with_outputs.ipynb", "test.ipynb");
    env.run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Run the clean command
    let result = env.run(&["output", "clean", "--json"]).assert_success();

    let json = result.json_value();
    // It should report that it cleaned (or that there was nothing to clean)
    assert!(json["cleaned"].is_boolean());
}

#[test]
fn test_output_clean_when_empty() {
    let env = TestEnv::new();

    let result = env.run(&["output", "clean", "--json"]).assert_success();

    let json = result.json_value();
    // When there's no nb-cli dir in temp, cleaned should be false
    assert!(json["cleaned"].is_boolean());
}

// ==================== DEFAULT OUTPUT DIR TESTS ====================

#[test]
fn test_read_default_output_dir_uses_nb_cli_prefix() {
    let env = TestEnv::new();
    let nb_path = env.copy_fixture("with_rich_outputs.ipynb", "test.ipynb");

    // Read without --output-dir, which should use the default nb-cli/<name>/ path
    let result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Find externalized outputs (binary image should always be externalized)
    let outputs = test_helpers::parse_outputs(&result.stdout);
    let image_output = outputs.iter().find(|o| {
        o.get_str("mime")
            .map(|m| m.starts_with("image/"))
            .unwrap_or(false)
    });

    if let Some(img) = image_output {
        if let Some(path) = img.get_str("path") {
            // The path should contain nb-cli somewhere in it
            assert!(
                path.contains("nb-cli"),
                "Default output dir should use nb-cli prefix, got: {}",
                path
            );
        }
    }
}

// ==================== EXTENSION-OPTIONAL TESTS ====================

#[test]
fn test_read_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("basic.ipynb", "test.ipynb");

    // Read with extension
    let result_with_ext = env.run(&["read", "test.ipynb", "--json"]).assert_success();

    // Read without extension
    let result_without_ext = env.run(&["read", "test", "--json"]).assert_success();

    // Both should produce the same output
    let json_with = result_with_ext.json_value();
    let json_without = result_without_ext.json_value();
    assert_eq!(json_with["cell_count"], json_without["cell_count"]);
}

#[test]
fn test_create_without_extension() {
    let env = TestEnv::new();

    // Create without extension - should add .ipynb automatically
    let result = env.run(&["create", "notebook", "--json"]).assert_success();

    let json = result.json_value();
    assert_eq!(json["file"], "notebook.ipynb");
    assert!(env.notebook_path("notebook.ipynb").exists());
}

#[test]
fn test_cell_add_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("basic.ipynb", "test.ipynb");

    // Add cell without extension
    let result = env
        .run(&["cell", "add", "test", "-s", "print('hello')", "--json"])
        .assert_success();

    assert!(result.success);
    let json = result.json_value();
    assert_eq!(json["cell_type"], "code");
}

#[test]
fn test_cell_update_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("basic.ipynb", "test.ipynb");

    // Update cell without extension
    let result = env
        .run(&[
            "cell", "update", "test", "-i", "0", "-s", "updated", "--json",
        ])
        .assert_success();

    assert!(result.success);
    let json = result.json_value();
    assert_eq!(json["index"], 0);
}

#[test]
fn test_cell_delete_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("mixed_cells.ipynb", "test.ipynb");

    // Delete cell without extension
    let result = env
        .run(&["cell", "delete", "test", "-i", "0", "--json"])
        .assert_success();

    assert!(result.success);
    let json = result.json_value();
    assert_eq!(json["cells_deleted"], 1);
}

#[test]
fn test_search_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("with_code.ipynb", "test.ipynb");

    // Search without extension
    let result = env.run(&["search", "test", "print"]).assert_success();

    assert!(result.stdout.contains("match"));
}

#[test]
fn test_output_clear_without_extension() {
    let env = TestEnv::new();
    env.copy_fixture("with_outputs.ipynb", "test.ipynb");

    // Clear outputs without extension
    let result = env
        .run(&["output", "clear", "test", "--json"])
        .assert_success();

    assert!(result.success);
    let json = result.json_value();
    assert!(json["cells_cleared"].is_number());
}
