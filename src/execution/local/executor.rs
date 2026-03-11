use super::discovery::find_kernel;
use crate::execution::types::{ExecutionConfig, ExecutionError, ExecutionResult};
use crate::execution::ExecutionBackend;
use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Local execution backend using nbclient
///
/// This implementation uses a Python script with nbclient to execute code.
/// nbclient is the official Jupyter library for notebook execution and provides
/// a high-level API that handles all kernel management automatically.
///
/// For multiple cells, this executes each cell immediately but passes all previous
/// cells as context to preserve kernel state (variables from earlier cells remain available).
pub struct LocalExecutor {
    config: ExecutionConfig,
    kernel_name: String,
    batch_script_path: std::path::PathBuf,
    // Track executed cells to maintain kernel state
    executed_cells: Vec<String>,
}

// Embed the Python batch script at compile time
const BATCH_SCRIPT: &str = include_str!("../../../scripts/execute_batch.py");

impl LocalExecutor {
    /// Create a new local executor
    pub fn new(config: ExecutionConfig) -> Result<Self> {
        // Determine Python batch script path
        // First try using the script from the source tree (for development)
        let dev_batch_script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("execute_batch.py");

        let batch_script_path = if dev_batch_script.exists() {
            dev_batch_script
        } else {
            // Fall back to writing embedded script to temp directory (for distribution)
            let temp_dir = std::env::temp_dir();
            let batch_path = temp_dir.join("nb-cli-execute_batch.py");

            // Write embedded script if it doesn't exist
            if !batch_path.exists() {
                std::fs::write(&batch_path, BATCH_SCRIPT)
                    .context("Failed to write batch script to temp directory")?;
            }

            batch_path
        };

        Ok(Self {
            config,
            kernel_name: String::new(),
            batch_script_path,
            executed_cells: Vec::new(),
        })
    }
}

/// Execute multiple cells as a batch using the Python script
fn execute_batch(
    script_path: &std::path::Path,
    cells: &[String],
    kernel_name: &str,
    timeout: std::time::Duration,
) -> Result<Vec<ExecutionResult>> {
    // Create JSON input with all cell codes
    let cells_json = serde_json::to_string(cells).context("Failed to serialize cells")?;

    // Execute Python script with JSON input via stdin
    let mut child = Command::new("python3")
        .arg(script_path)
        .arg("--from-json")
        .arg("--kernel")
        .arg(kernel_name)
        .arg("--timeout")
        .arg(timeout.as_secs().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn Python process")?;

    // Write cells JSON to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(cells_json.as_bytes())
            .context("Failed to write cells to Python stdin")?;
    }

    // Wait for completion
    let output = child
        .wait_with_output()
        .context("Failed to wait for Python process")?;

    // Parse JSON output (array of results)
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Note: allow non-zero exit if we got valid JSON output (cells can have errors)
    let results_json: Vec<serde_json::Value> = serde_json::from_str(&stdout)
        .with_context(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!(
                "Failed to parse Python output.\nStdout: {}\nStderr: {}",
                stdout, stderr
            )
        })?;

    // Convert JSON results to ExecutionResult
    let mut results = Vec::new();
    for result_json in results_json {
        let success = result_json["success"]
            .as_bool()
            .context("Missing 'success' field")?;

        let outputs_json = result_json["outputs"]
            .as_array()
            .context("Missing 'outputs' field")?;

        let mut outputs = Vec::new();
        for output_json in outputs_json {
            outputs.push(serde_json::from_value(output_json.clone())?);
        }

        let execution_count = result_json["execution_count"].as_i64();

        let error = if let Some(error_json) = result_json.get("error") {
            if !error_json.is_null() {
                Some(ExecutionError {
                    ename: error_json["ename"].as_str().unwrap_or("").to_string(),
                    evalue: error_json["evalue"].as_str().unwrap_or("").to_string(),
                    traceback: error_json["traceback"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            } else {
                None
            }
        } else {
            None
        };

        if success {
            results.push(ExecutionResult::success(outputs, execution_count));
        } else if let Some(error) = error {
            results.push(ExecutionResult::error(outputs, execution_count, error));
        } else {
            anyhow::bail!("Execution failed but no error information provided");
        }
    }

    Ok(results)
}

#[async_trait::async_trait]
impl ExecutionBackend for LocalExecutor {
    async fn start(&mut self) -> Result<()> {
        // Find kernel
        let (kernel_name, _kernel_spec_path) = find_kernel(
            self.config.kernel_name.as_deref(),
            None, // Notebook kernel will be passed from command
        )?;

        self.kernel_name = kernel_name;

        // Check that Python and nbclient are available
        let check = Command::new("python3")
            .arg("-c")
            .arg("import nbclient")
            .output()
            .context("Failed to check for nbclient")?;

        if !check.status.success() {
            let stderr = String::from_utf8_lossy(&check.stderr);
            anyhow::bail!(
                "nbclient not found. Install it with: pip install nbclient\nError: {}",
                stderr
            );
        }

        // Also check for nbformat (required by nbclient)
        let check = Command::new("python3")
            .arg("-c")
            .arg("import nbformat")
            .output()
            .context("Failed to check for nbformat")?;

        if !check.status.success() {
            anyhow::bail!("nbformat not found. Install it with: pip install nbformat");
        }

        Ok(())
    }

    async fn execute_code(
        &mut self,
        code: &str,
        _cell_id: Option<&str>,
    ) -> Result<ExecutionResult> {
        // Add this cell to the list
        self.executed_cells.push(code.to_string());

        // Execute all cells accumulated so far as a batch
        // This preserves kernel state across cells
        let cells = self.executed_cells.clone();
        let batch_script_path = self.batch_script_path.clone();
        let kernel_name = self.kernel_name.clone();
        let timeout = self.config.timeout;

        // Execute in a blocking task
        let results = tokio::task::spawn_blocking(move || {
            execute_batch(&batch_script_path, &cells, &kernel_name, timeout)
        })
        .await
        .context("Task join error")??;

        // Return only the result for the current (last) cell
        let current_cell_index = self.executed_cells.len() - 1;
        results
            .get(current_cell_index)
            .cloned()
            .context("Missing result for current cell")
    }

    async fn stop(&mut self) -> Result<()> {
        // Clear executed cells for next session
        self.executed_cells.clear();
        Ok(())
    }
}
