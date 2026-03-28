use crate::commands::common::OutputFormat;
use crate::commands::env_manager::EnvConfig;
use crate::execution::local::discovery::find_kernel;
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::Parser;
use nbformat::v4::{Cell, CellId, CellMetadata, KernelSpec, Metadata, Notebook};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

#[derive(Parser)]
pub struct CreateArgs {
    /// Path to create notebook file
    pub file: String,

    /// Kernel name
    #[arg(
        short = 'k',
        long = "kernel",
        default_value = "python3",
        value_name = "NAME"
    )]
    pub kernel: String,

    /// Create notebook with a markdown cell instead of code cell
    #[arg(long)]
    pub markdown: bool,

    /// Overwrite if file exists
    #[arg(long = "force")]
    pub force: bool,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,

    /// Use uv to discover kernel metadata
    #[arg(long, conflicts_with = "pixi")]
    pub uv: bool,

    /// Use pixi to discover kernel metadata
    #[arg(long, conflicts_with = "uv")]
    pub pixi: bool,
}

#[derive(Serialize)]
struct CreateResult {
    file: String,
    kernel: String,
    cell_count: usize,
}

pub fn execute(args: CreateArgs) -> Result<()> {
    // Ensure path ends with .ipynb
    let path = if args.file.ends_with(".ipynb") {
        args.file.clone()
    } else {
        format!("{}.ipynb", args.file)
    };

    // Validate notebook name and show warnings (but don't fail)
    let filename = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&path);

    let warnings = validate_notebook_name(filename);
    for warning in &warnings {
        eprintln!("Warning: {}", warning);
    }

    let path_obj = Path::new(&path);

    // Check if file exists
    if path_obj.exists() && !args.force {
        bail!("File '{}' already exists. Use --force to overwrite.", path);
    }

    // Create parent directories if they don't exist
    if let Some(parent) = path_obj.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .context(format!("Failed to create directory '{}'", parent.display()))?;
        }
    }

    // Create notebook with specified kernel
    let notebook = create_notebook(&args)?;

    // Write notebook to file
    notebook::write_notebook_atomic(&path, &notebook).context("Failed to write notebook")?;

    // Output result
    let result = CreateResult {
        file: path.clone(),
        kernel: args.kernel.clone(),
        cell_count: notebook.cells.len(),
    };

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    output_result(&result, &format)?;

    Ok(())
}

fn create_notebook(args: &CreateArgs) -> Result<Notebook> {
    // Create environment configuration if using uv/pixi
    let env_config = if args.uv || args.pixi {
        Some(EnvConfig::from_flags(args.uv, args.pixi)?)
    } else {
        None
    };

    // Find and validate the kernel exists
    let (kernel_name, kernel_spec_path) = find_kernel(
        Some(&args.kernel),
        None,
        env_config.as_ref(),
        Some("create"),
    )?;

    // Read kernel.json from the kernelspec directory
    let kernel_json_path = kernel_spec_path.join("kernel.json");
    let content = std::fs::read_to_string(&kernel_json_path).context(format!(
        "Failed to read kernel spec from {}",
        kernel_json_path.display()
    ))?;

    // Parse the kernelspec
    let spec = serde_json::from_str::<jupyter_protocol::JupyterKernelspec>(&content)
        .context("Failed to parse kernel.json")?;

    // Use the actual kernel metadata
    let kernelspec = KernelSpec {
        name: kernel_name,
        display_name: spec.display_name,
        language: Some(spec.language),
        additional: HashMap::new(),
    };

    // Create metadata
    let metadata = Metadata {
        kernelspec: Some(kernelspec),
        language_info: None, // Will be populated on first execution
        ..Default::default()
    };

    // Create cells based on flags
    let empty_metadata = create_empty_metadata();

    let cells = if args.markdown {
        vec![Cell::Markdown {
            id: CellId::from(Uuid::new_v4()),
            metadata: empty_metadata,
            source: vec![], // Empty markdown cell
            attachments: None,
        }]
    } else {
        vec![Cell::Code {
            id: CellId::from(Uuid::new_v4()),
            metadata: empty_metadata,
            execution_count: None,
            source: vec![],
            outputs: vec![],
        }]
    };

    Ok(Notebook {
        cells,
        metadata,
        nbformat: 4,
        nbformat_minor: 5,
    })
}

fn create_empty_metadata() -> CellMetadata {
    CellMetadata {
        id: None,
        collapsed: None,
        scrolled: None,
        deletable: None,
        editable: None,
        format: None,
        name: None,
        tags: None,
        jupyter: None,
        execution: None,
        additional: HashMap::new(),
    }
}

fn output_result(result: &CreateResult, format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text | OutputFormat::Markdown => {
            println!("Created notebook: {}", result.file);
            println!("Kernel: {}", result.kernel);
            println!("Cells: {}", result.cell_count);
        }
    }
    Ok(())
}

/// Validate notebook name and return warnings for poor practices
fn validate_notebook_name(name: &str) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check for "Untitled" pattern
    if name.to_lowercase().contains("untitled") {
        warnings.push(
            "Consider using a descriptive name instead of 'Untitled' (e.g., 'data_analysis.ipynb')"
                .to_string(),
        );
    }

    // Check for "copy" pattern
    if name.to_lowercase().contains("copy") {
        warnings.push("Consider renaming from 'Copy' to a descriptive name".to_string());
    }

    // Check for special characters that may cause issues
    let problematic_chars = ['?', '*', '<', '>', '|', ':', '"'];
    if name.chars().any(|c| problematic_chars.contains(&c)) {
        warnings.push(format!(
            "Filename contains special characters that may cause issues: {}",
            name
        ));
    }

    // Check for spaces in filename
    if name.contains(' ') {
        warnings.push(
            "Consider using underscores or hyphens instead of spaces (e.g., 'my_notebook.ipynb')"
                .to_string(),
        );
    }

    // Check if name is too generic
    if name.to_lowercase() == "notebook.ipynb" || name.to_lowercase() == "test.ipynb" {
        warnings.push(
            "Consider using a more specific name that describes the notebook's purpose".to_string(),
        );
    }

    warnings
}
