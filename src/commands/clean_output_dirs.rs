use crate::commands::common::OutputFormat;
use crate::commands::markdown_renderer;
use anyhow::Result;
use clap::Parser;
use serde::Serialize;

#[derive(Parser)]
pub struct CleanOutputDirsArgs {
    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct CleanResult {
    cleaned: bool,
    path: String,
}

pub fn execute(args: CleanOutputDirsArgs) -> Result<()> {
    let nb_cli_dir = std::env::temp_dir().join("nb-cli");
    let path_str = nb_cli_dir.to_string_lossy().to_string();
    let existed = nb_cli_dir.exists();

    markdown_renderer::clean_output_dirs()?;

    let result = CleanResult {
        cleaned: existed,
        path: path_str,
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            if existed {
                println!("Cleaned output directory: {}", result.path);
            } else {
                println!("No output directory to clean ({})", result.path);
            }
        }
    }

    Ok(())
}
