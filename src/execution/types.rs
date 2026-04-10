use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::commands::env_manager::EnvConfig;

/// Execution mode: local (direct kernel) or remote (Jupyter server)
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionMode {
    /// Local execution via direct kernel connection
    Local,
    /// Remote execution via Jupyter Server API
    Remote { server_url: String, token: String },
}

/// Configuration for code execution
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Execution mode (local or remote)
    pub mode: ExecutionMode,

    /// Timeout for cell execution (default: 30s)
    pub timeout: Duration,

    /// Kernel name to use (None = use notebook metadata or default)
    pub kernel_name: Option<String>,

    /// Continue execution even if errors occur
    pub allow_errors: bool,

    /// Notebook path (for remote mode session matching)
    pub notebook_path: Option<String>,

    /// Environment manager configuration (for local mode kernel discovery)
    pub env_config: Option<EnvConfig>,

    /// Restart kernel before execution (remote mode, full notebook)
    pub restart_kernel: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Local,
            timeout: Duration::from_secs(30),
            kernel_name: None,
            allow_errors: false,
            notebook_path: None,
            env_config: None,
            restart_kernel: false,
        }
    }
}

/// Result of executing code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Whether execution completed successfully (no errors)
    pub success: bool,

    /// Outputs generated during execution
    pub outputs: Vec<nbformat::v4::Output>,

    /// Execution count assigned by kernel
    pub execution_count: Option<i64>,

    /// Error information (if execution failed)
    pub error: Option<ExecutionError>,
}

/// Execution error details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionError {
    /// Exception name (e.g., "ZeroDivisionError")
    pub ename: String,

    /// Exception value/message
    pub evalue: String,

    /// Stack trace lines
    pub traceback: Vec<String>,
}

impl ExecutionResult {
    /// Create a successful execution result
    pub fn success(outputs: Vec<nbformat::v4::Output>, execution_count: Option<i64>) -> Self {
        Self {
            success: true,
            outputs,
            execution_count,
            error: None,
        }
    }

    /// Create a failed execution result
    pub fn error(
        outputs: Vec<nbformat::v4::Output>,
        execution_count: Option<i64>,
        error: ExecutionError,
    ) -> Self {
        Self {
            success: false,
            outputs,
            execution_count,
            error: Some(error),
        }
    }
}

/// Output from a single execution message
#[allow(dead_code)]
#[derive(Debug)]
pub enum MessageOutput {
    /// stdout/stderr stream
    Stream { name: String, text: String },

    /// Display data (images, HTML, etc.)
    DisplayData {
        data: serde_json::Value,
        metadata: serde_json::Value,
    },

    /// Execute result (output of cell)
    ExecuteResult {
        data: serde_json::Value,
        metadata: serde_json::Value,
        execution_count: i64,
    },

    /// Error/exception
    Error {
        ename: String,
        evalue: String,
        traceback: Vec<String>,
    },
}

impl MessageOutput {
    /// Convert to nbformat Output
    #[allow(dead_code)]
    pub fn to_nbformat_output(&self) -> Result<nbformat::v4::Output> {
        // Use serde to convert between the types
        match self {
            MessageOutput::Stream { name, text } => Ok(nbformat::v4::Output::Stream {
                name: name.clone(),
                text: nbformat::v4::MultilineString(text.clone()),
            }),
            MessageOutput::DisplayData { data, metadata } => {
                // Create a temporary JSON object with the right structure
                let json = serde_json::json!({
                    "output_type": "display_data",
                    "data": data,
                    "metadata": metadata
                });
                Ok(serde_json::from_value(json)?)
            }
            MessageOutput::ExecuteResult {
                data,
                metadata,
                execution_count,
            } => {
                // Create a temporary JSON object with the right structure
                let json = serde_json::json!({
                    "output_type": "execute_result",
                    "execution_count": execution_count,
                    "data": data,
                    "metadata": metadata
                });
                Ok(serde_json::from_value(json)?)
            }
            MessageOutput::Error {
                ename,
                evalue,
                traceback,
            } => Ok(nbformat::v4::Output::Error(nbformat::v4::ErrorOutput {
                ename: ename.clone(),
                evalue: evalue.clone(),
                traceback: traceback.clone(),
            })),
        }
    }
}
