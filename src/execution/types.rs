use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::execution::env::EnvConfig;

/// Execution mode: local (direct kernel), remote (Jupyter server), or remote-kernel (gateway)
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionMode {
    /// Local execution via direct kernel connection
    Local,
    /// Remote execution via Jupyter Server API
    Remote { server_url: String, token: String },
    /// Remote kernel execution via Jupyter Kernel Gateway
    RemoteKernel {
        gateway_url: String,
        token: String,
        kernel_id: Option<String>,
        /// Authorization scheme used with `token`, e.g. "token" or "Bearer".
        auth_scheme: String,
    },
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

    /// Restart kernel before execution (remote mode only; applies to any cell range)
    pub restart_kernel: bool,

    /// Whether Y.js backend is available (None = unknown, try and fall back)
    pub ydoc_available: Option<bool>,
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
            ydoc_available: None,
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
