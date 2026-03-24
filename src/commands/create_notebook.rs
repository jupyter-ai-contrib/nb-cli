use crate::commands::common::OutputFormat;
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

    /// Kernel language
    #[arg(long = "language", default_value = "python", value_name = "LANG")]
    pub language: String,

    /// Create notebook with a markdown cell instead of code cell
    #[arg(long)]
    pub markdown: bool,

    /// Overwrite if file exists
    #[arg(long = "force")]
    pub force: bool,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,
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
    // Create kernel spec matching Jupyter conventions
    let display_name = match (args.language.as_str(), args.kernel.as_str()) {
        ("python", "python3") => "Python 3 (ipykernel)".to_string(),
        ("python", kernel) => format!("Python 3 ({})", kernel),
        (lang, kernel) if lang == kernel => kernel.to_string(),
        (lang, kernel) => format!("{} ({})", lang, kernel),
    };
    let kernelspec = KernelSpec {
        name: args.kernel.clone(),
        display_name,
        language: Some(args.language.clone()),
        additional: HashMap::new(),
    };

    // Create metadata
    let metadata = Metadata {
        kernelspec: Some(kernelspec),
        language_info: None, // Will be populated on first execution
        ..Default::default()
    };

    // Create cells based on markdown flag
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
