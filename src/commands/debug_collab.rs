use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct DebugCollabArgs {
    /// Jupyter server URL (e.g., http://localhost:8889)
    #[arg(long)]
    pub server: String,

    /// Authentication token for the server
    #[arg(long)]
    pub token: String,

    /// Notebook file path (relative to server root)
    #[arg(long)]
    pub notebook: String,
}

pub fn execute(args: DebugCollabArgs) -> Result<()> {
    // Create Tokio runtime for async execution
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(crate::debug_collab::debug_collaboration_sync(
        args.server,
        args.token,
        args.notebook,
    ))
}
