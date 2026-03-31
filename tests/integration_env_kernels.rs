mod test_helpers;

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper struct to manage test environment with uv project
struct UvTestEnv {
    temp_dir: TempDir,
    binary_path: PathBuf,
}

impl UvTestEnv {
    fn new() -> Option<Self> {
        // Check if uv is available
        if !has_uv() {
            eprintln!("⚠️  Skipping test: uv not available");
            return None;
        }

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let binary_path = env!("CARGO_BIN_EXE_nb").into();

        Some(Self {
            temp_dir,
            binary_path,
        })
    }

    fn project_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }

    fn notebook_path(&self, name: &str) -> PathBuf {
        self.project_path().join(name)
    }

    /// Initialize a uv project in the test directory
    fn init_uv_project(&self) -> bool {
        let status = Command::new("uv")
            .args(["init", "--bare", "--name", "test-project"])
            .current_dir(self.project_path())
            .status()
            .expect("Failed to run uv init");

        status.success()
    }

    /// Install ipykernel in the uv environment
    fn install_ipykernel(&self) -> bool {
        let status = Command::new("uv")
            .args(["add", "ipykernel"])
            .current_dir(self.project_path())
            .status()
            .expect("Failed to run uv add");

        status.success()
    }

    /// Install a custom kernel in the uv environment
    fn install_custom_kernel(&self, kernel_name: &str) -> bool {
        let status = Command::new("uv")
            .args([
                "run",
                "python",
                "-m",
                "ipykernel",
                "install",
                "--sys-prefix",
                "--name",
                kernel_name,
            ])
            .current_dir(self.project_path())
            .status()
            .expect("Failed to install kernel");

        status.success()
    }

    fn run(&self, args: &[&str]) -> CommandResult {
        let output = Command::new(&self.binary_path)
            .args(args)
            .current_dir(self.project_path())
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

/// Check if uv is installed
fn has_uv() -> bool {
    Command::new("uv")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ==================== ENVIRONMENT KERNEL TESTS ====================

#[test]
fn test_create_with_uv_kernel_succeeds() {
    let Some(env) = UvTestEnv::new() else {
        eprintln!("⚠️  Skipping test: uv not available");
        return;
    };

    // Setup uv project
    if !env.init_uv_project() {
        eprintln!("⚠️  Skipping test: failed to initialize uv project");
        return;
    }

    // Install ipykernel
    if !env.install_ipykernel() {
        eprintln!("⚠️  Skipping test: failed to install ipykernel");
        return;
    }

    // Install custom kernel
    if !env.install_custom_kernel("uv-test-kernel") {
        eprintln!("⚠️  Skipping test: failed to install custom kernel");
        return;
    }

    let nb_path = env.notebook_path("test.ipynb");

    // Create notebook with --uv flag and custom kernel
    let _result = env
        .run(&[
            "create",
            nb_path.to_str().unwrap(),
            "--uv",
            "--kernel",
            "uv-test-kernel",
        ])
        .assert_success();

    // Verify notebook was created
    assert!(nb_path.exists(), "Notebook should be created");

    // Read the notebook and verify kernel is set correctly
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap()])
        .assert_success();

    // Parse notebook metadata to verify kernel
    let notebook = test_helpers::parse_notebook_header(&read_result.stdout)
        .expect("Should have notebook header");

    // Check that kernelspec.name is set to our custom kernel
    let kernelspec = notebook
        .metadata
        .get("metadata")
        .and_then(|m| m.get("kernelspec"))
        .expect("Should have kernelspec in metadata");

    let kernel_name = kernelspec
        .get("name")
        .and_then(|n| n.as_str())
        .expect("Should have kernel name");

    assert_eq!(
        kernel_name, "uv-test-kernel",
        "Notebook should use uv-test-kernel"
    );
}

#[test]
fn test_create_without_uv_flag_fails() {
    let Some(env) = UvTestEnv::new() else {
        eprintln!("⚠️  Skipping test: uv not available");
        return;
    };

    // Setup uv project
    if !env.init_uv_project() {
        eprintln!("⚠️  Skipping test: failed to initialize uv project");
        return;
    }

    // Install ipykernel
    if !env.install_ipykernel() {
        eprintln!("⚠️  Skipping test: failed to install ipykernel");
        return;
    }

    // Install custom kernel
    if !env.install_custom_kernel("uv-test-kernel") {
        eprintln!("⚠️  Skipping test: failed to install custom kernel");
        return;
    }

    let nb_path = env.notebook_path("test.ipynb");

    // Try to create notebook WITHOUT --uv flag - should fail
    let result = env
        .run(&[
            "create",
            nb_path.to_str().unwrap(),
            "--kernel",
            "uv-test-kernel",
        ])
        .assert_failure();

    // Verify error message mentions kernel not found
    assert!(
        result.contains("not found") || result.contains("Kernel"),
        "Error should mention kernel not found"
    );

    // Notebook should not be created
    assert!(
        !nb_path.exists(),
        "Notebook should not be created on failure"
    );
}

#[test]
fn test_execute_with_uv_kernel_succeeds() {
    let Some(env) = UvTestEnv::new() else {
        eprintln!("⚠️  Skipping test: uv not available");
        return;
    };

    // Setup uv project
    if !env.init_uv_project() {
        eprintln!("⚠️  Skipping test: failed to initialize uv project");
        return;
    }

    // Install ipykernel
    if !env.install_ipykernel() {
        eprintln!("⚠️  Skipping test: failed to install ipykernel");
        return;
    }

    // Install custom kernel
    if !env.install_custom_kernel("uv-test-kernel") {
        eprintln!("⚠️  Skipping test: failed to install custom kernel");
        return;
    }

    let nb_path = env.notebook_path("test.ipynb");

    // Create notebook with --uv flag
    env.run(&[
        "create",
        nb_path.to_str().unwrap(),
        "--uv",
        "--kernel",
        "uv-test-kernel",
    ])
    .assert_success();

    // Add a cell with code
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "result = 2 + 2",
    ])
    .assert_success();

    // Execute with --uv flag
    let result = env
        .run(&["execute", nb_path.to_str().unwrap(), "--uv"])
        .assert_success();

    assert!(
        result.contains("executed") || result.contains("Executed") || result.contains("success"),
        "Execution should succeed"
    );

    // Verify cell was executed
    let read_result = env
        .run(&["read", nb_path.to_str().unwrap(), "--cell-index", "1"])
        .assert_success();

    // Check for execution count
    assert!(
        read_result.stdout.contains("execution_count")
            || read_result.stdout.contains("Execution count"),
        "Cell should have execution count after execution"
    );
}

#[test]
fn test_execute_without_uv_flag_with_uv_kernel_fails() {
    let Some(env) = UvTestEnv::new() else {
        eprintln!("⚠️  Skipping test: uv not available");
        return;
    };

    // Setup uv project
    if !env.init_uv_project() {
        eprintln!("⚠️  Skipping test: failed to initialize uv project");
        return;
    }

    // Install ipykernel
    if !env.install_ipykernel() {
        eprintln!("⚠️  Skipping test: failed to install ipykernel");
        return;
    }

    // Install custom kernel
    if !env.install_custom_kernel("uv-test-kernel") {
        eprintln!("⚠️  Skipping test: failed to install custom kernel");
        return;
    }

    let nb_path = env.notebook_path("test.ipynb");

    // Create notebook with --uv flag
    env.run(&[
        "create",
        nb_path.to_str().unwrap(),
        "--uv",
        "--kernel",
        "uv-test-kernel",
    ])
    .assert_success();

    // Add a cell with code
    env.run(&[
        "cell",
        "add",
        nb_path.to_str().unwrap(),
        "--source",
        "result = 2 + 2",
    ])
    .assert_success();

    // Try to execute WITHOUT --uv flag - should fail because kernel isn't globally available
    let result = env.run(&["execute", nb_path.to_str().unwrap()]);

    // Execution should fail
    assert!(
        !result.success,
        "Execution should fail without --uv flag when using uv-specific kernel"
    );

    // Error should mention kernel issue
    assert!(
        result.contains("not found") || result.contains("Kernel") || result.contains("kernel"),
        "Error should mention kernel not found"
    );
}
