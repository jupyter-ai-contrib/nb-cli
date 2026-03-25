use crate::commands::common::{self, OutputFormat};
use crate::commands::markdown_renderer::{self, IndexedCell};
use crate::notebook;
use anyhow::{Context, Result};
use clap::Parser;
use nbformat::v4::Cell;
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
pub struct ReadArgs {
    /// Path to notebook file
    pub file: String,

    /// Output in JSON format (default: AI-Optimized Markdown)
    #[arg(long)]
    pub json: bool,

    /// Get specific cell by ID (stable identifier)
    #[arg(short, long, value_name = "ID", conflicts_with_all = ["cell_index", "only_code", "only_markdown"])]
    pub cell: Option<String>,

    /// Get specific cell by index (supports negative indexing like -1)
    #[arg(short = 'i', long = "cell-index", value_name = "INDEX", allow_negative_numbers = true, conflicts_with_all = ["cell", "only_code", "only_markdown"])]
    pub cell_index: Option<i32>,

    /// Exclude outputs from display
    #[arg(long = "no-output")]
    pub no_output: bool,

    /// Directory for externalized output files (markdown format only)
    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,

    /// Maximum characters for inline output (default: 4000). Outputs exceeding this are externalized to files
    #[arg(long, default_value = "4000")]
    pub limit: usize,

    /// Show only code cells
    #[arg(long = "only-code", alias = "code", conflicts_with_all = ["cell", "cell_index", "only_markdown"])]
    pub only_code: bool,

    /// Show only markdown cells
    #[arg(long = "only-markdown", alias = "markdown-cells", conflicts_with_all = ["cell", "cell_index", "only_code"])]
    pub only_markdown: bool,
}

pub fn execute(args: ReadArgs) -> Result<()> {
    let file_path = common::normalize_notebook_path(&args.file);
    let notebook = notebook::read_notebook(&file_path)?;

    // Determine format: markdown (default) or JSON
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Markdown
    };

    // Default: include outputs, unless --no-output is specified
    let include_outputs = !args.no_output;

    // Setup output directory for markdown format
    let output_dir = if matches!(format, OutputFormat::Markdown) && include_outputs {
        if let Some(dir) = &args.output_dir {
            Some(PathBuf::from(dir))
        } else {
            let dir = markdown_renderer::notebook_output_dir(&file_path);
            std::fs::create_dir_all(&dir)?;
            Some(dir)
        }
    } else {
        None
    };

    // Handle specific cell by ID
    if let Some(ref cell_id) = args.cell {
        let (index, cell) = common::find_cell_by_id(&notebook.cells, cell_id)?;
        output_cell_with_optional_output(
            cell,
            index,
            &notebook,
            &format,
            include_outputs,
            output_dir.as_deref(),
            args.limit,
        )?;
        return Ok(());
    }

    // Handle specific cell by index
    if let Some(cell_index) = args.cell_index {
        let index = common::normalize_index(cell_index, notebook.cells.len())?;
        let cell = notebook.cells.get(index).context(format!(
            "Cell index {} out of range (notebook has {} cells)",
            index,
            notebook.cells.len()
        ))?;

        output_cell_with_optional_output(
            cell,
            index,
            &notebook,
            &format,
            include_outputs,
            output_dir.as_deref(),
            args.limit,
        )?;
        return Ok(());
    }

    // Handle --only-code flag
    if args.only_code {
        output_filtered_cells(
            &notebook,
            &format,
            include_outputs,
            output_dir.as_deref(),
            args.limit,
            |cell| matches!(cell, Cell::Code { .. }),
        )?;
        return Ok(());
    }

    // Handle --only-markdown flag
    if args.only_markdown {
        output_filtered_cells(&notebook, &format, false, None, args.limit, |cell| {
            matches!(cell, Cell::Markdown { .. })
        })?;
        return Ok(());
    }

    // Default: show notebook structure
    output_notebook_structure(
        &notebook,
        &format,
        include_outputs,
        output_dir.as_deref(),
        args.limit,
    )?;
    Ok(())
}

fn output_cell_with_optional_output(
    cell: &Cell,
    index: usize,
    notebook: &nbformat::v4::Notebook,
    format: &OutputFormat,
    with_outputs: bool,
    output_dir: Option<&std::path::Path>,
    inline_limit: usize,
) -> Result<()> {
    match format {
        OutputFormat::Markdown | OutputFormat::Text => {
            let indexed = [IndexedCell { index, cell }];
            let markdown = markdown_renderer::render_indexed_cells_markdown(
                notebook,
                &indexed,
                with_outputs,
                output_dir,
                inline_limit,
            )?;
            print!("{}", markdown);
        }
        OutputFormat::Json => {
            // Serialize the cell directly from nbformat, preserving all fields
            let mut cell_json = serde_json::to_value(cell)?;

            // Add index as a convenience field (not in standard nbformat)
            if let Some(obj) = cell_json.as_object_mut() {
                obj.insert("index".to_string(), json!(index));
            }

            // If outputs are not requested, remove them from code cells
            if !with_outputs {
                if let Some(obj) = cell_json.as_object_mut() {
                    obj.remove("outputs");
                }
            }

            println!("{}", serde_json::to_string_pretty(&cell_json)?);
        }
    }
    Ok(())
}

fn output_filtered_cells(
    notebook: &nbformat::v4::Notebook,
    format: &OutputFormat,
    with_outputs: bool,
    output_dir: Option<&std::path::Path>,
    inline_limit: usize,
    filter: impl Fn(&Cell) -> bool,
) -> Result<()> {
    match format {
        OutputFormat::Markdown | OutputFormat::Text => {
            let indexed: Vec<IndexedCell> = notebook
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| filter(cell))
                .map(|(i, cell)| IndexedCell { index: i, cell })
                .collect();

            let markdown = markdown_renderer::render_indexed_cells_markdown(
                notebook,
                &indexed,
                with_outputs,
                output_dir,
                inline_limit,
            )?;
            print!("{}", markdown);
        }
        OutputFormat::Json => {
            let filtered: Vec<serde_json::Value> = notebook
                .cells
                .iter()
                .enumerate()
                .filter(|(_, cell)| filter(cell))
                .filter_map(|(index, cell)| {
                    let mut cell_json = serde_json::to_value(cell).ok()?;

                    if let Some(obj) = cell_json.as_object_mut() {
                        obj.insert("index".to_string(), json!(index));
                        if !with_outputs {
                            obj.remove("outputs");
                        }
                    }

                    Some(cell_json)
                })
                .collect();
            let output = json!({ "cells": filtered });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    }
    Ok(())
}

fn output_notebook_structure(
    notebook: &nbformat::v4::Notebook,
    format: &OutputFormat,
    with_outputs: bool,
    output_dir: Option<&std::path::Path>,
    inline_limit: usize,
) -> Result<()> {
    // If with_outputs is true, show full cells with outputs instead of structure
    if with_outputs {
        match format {
            OutputFormat::Markdown | OutputFormat::Text => {
                let markdown = markdown_renderer::render_notebook_markdown(
                    notebook,
                    true,
                    output_dir,
                    inline_limit,
                )?;
                print!("{}", markdown);
            }
            OutputFormat::Json => {
                let cells: Vec<serde_json::Value> = notebook
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        let mut cell_json = serde_json::to_value(cell).unwrap_or(json!(null));

                        if let Some(obj) = cell_json.as_object_mut() {
                            obj.insert("index".to_string(), json!(index));
                        }

                        cell_json
                    })
                    .collect();
                let output = json!({ "cells": cells });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
        return Ok(());
    }

    // Default structure view - serialize cells directly with nbformat
    match format {
        OutputFormat::Markdown | OutputFormat::Text => {
            let markdown =
                markdown_renderer::render_notebook_markdown(notebook, false, None, inline_limit)?;
            print!("{}", markdown);
        }
        OutputFormat::Json => {
            let cells: Vec<serde_json::Value> = notebook
                .cells
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    let mut cell_json = serde_json::to_value(cell).unwrap_or(json!(null));

                    if let Some(obj) = cell_json.as_object_mut() {
                        obj.insert("index".to_string(), json!(index));
                        obj.remove("outputs");
                    }

                    cell_json
                })
                .collect();

            // Count cell types for summary
            let mut code_cells = 0;
            let mut markdown_cells = 0;
            let mut raw_cells = 0;

            for cell in &notebook.cells {
                match cell {
                    Cell::Code { .. } => code_cells += 1,
                    Cell::Markdown { .. } => markdown_cells += 1,
                    Cell::Raw { .. } => raw_cells += 1,
                }
            }

            let kernel = notebook
                .metadata
                .kernelspec
                .as_ref()
                .map(|ks| ks.name.clone());

            let structure = json!({
                "cell_count": notebook.cells.len(),
                "code_cells": code_cells,
                "markdown_cells": markdown_cells,
                "raw_cells": raw_cells,
                "kernel": kernel,
                "cells": cells,
            });

            println!("{}", serde_json::to_string_pretty(&structure)?);
        }
    }
    Ok(())
}
