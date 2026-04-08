//! Execution module for running notebook cells
//!
//! Supports two execution modes:
//! - **Local**: Direct kernel connection using runtimelib + ZMQ
//! - **Remote**: Jupyter Server API using HTTP + WebSocket

pub mod local;
pub mod remote;
pub mod types;

use anyhow::Result;
use types::{ExecutionConfig, ExecutionMode, ExecutionResult};

/// Callback invoked when an output is produced during execution
pub type OutputCallback = Box<dyn Fn(&nbformat::v4::Output) + Send + Sync>;

/// Backend for executing code
///
/// Implementations provide either local (direct kernel) or remote (Jupyter server) execution
#[async_trait::async_trait]
pub trait ExecutionBackend: Send {
    /// Start the backend (spawn kernel or create session)
    async fn start(&mut self) -> Result<()>;

    /// Execute code and return result with outputs
    ///
    /// # Arguments
    /// * `code` - The code to execute
    /// * `cell_id` - Optional cell ID for remote execution (used by Jupyter Server)
    /// * `cell_index` - Optional cell index for Y.js document observation in remote mode
    /// * `on_output` - Optional callback invoked as each output arrives (for streaming)
    async fn execute_code(
        &mut self,
        code: &str,
        cell_id: Option<&str>,
        cell_index: Option<usize>,
        on_output: Option<&OutputCallback>,
    ) -> Result<ExecutionResult>;

    /// Stop the backend (cleanup kernel or close session)
    async fn stop(&mut self) -> Result<()>;
}

/// Create an execution backend based on configuration
pub fn create_backend(config: ExecutionConfig) -> Result<Box<dyn ExecutionBackend>> {
    match config.mode.clone() {
        ExecutionMode::Local => Ok(Box::new(local::LocalExecutor::new(config)?)),
        ExecutionMode::Remote { server_url, token } => Ok(Box::new(remote::RemoteExecutor::new(
            config, server_url, token,
        )?)),
    }
}
