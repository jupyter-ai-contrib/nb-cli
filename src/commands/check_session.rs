use anyhow::Result;
use clap::Args;

#[derive(Args)]
pub struct CheckSessionArgs {
    /// Notebook file path
    pub notebook: String,

    /// Jupyter server URL
    #[arg(long)]
    pub server: String,

    /// Authentication token
    #[arg(long)]
    pub token: String,
}

pub fn execute(args: CheckSessionArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(execute_async(args))
}

async fn execute_async(args: CheckSessionArgs) -> Result<()> {
    use crate::execution::remote::session_check;

    println!("Checking session for notebook: {}", args.notebook);
    println!("Server: {}", args.server);
    println!();

    let has_session =
        session_check::has_active_session(&args.server, &args.token, &args.notebook).await?;

    if has_session {
        println!("✅ Notebook IS open in JupyterLab");
    } else {
        println!("❌ Notebook is NOT open in JupyterLab");
    }

    Ok(())
}
