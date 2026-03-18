use crate::commands::common::{self, OutputFormat};
use crate::notebook;
use anyhow::{Context, Result};
use clap::Parser;
use jupyter_protocol::media::Media;
use nbformat::v4::{Cell, Output};
use serde_json::json;

#[derive(Parser)]
pub struct ReadArgs {
    /// Path to notebook file
    pub file: String,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,

    /// Get specific cell by ID (stable identifier)
    #[arg(short, long, value_name = "ID", conflicts_with_all = ["cell_index", "only_code", "only_markdown"])]
    pub cell: Option<String>,

    /// Get specific cell by index (supports negative indexing like -1)
    #[arg(short = 'i', long = "cell-index", value_name = "INDEX", allow_negative_numbers = true, conflicts_with_all = ["cell", "only_code", "only_markdown"])]
    pub cell_index: Option<i32>,

    /// Include cell execution outputs in the display
    #[arg(short = 'o', long = "with-outputs")]
    pub with_outputs: bool,

    /// Show only code cells
    #[arg(long = "only-code", alias = "code", conflicts_with_all = ["cell", "cell_index"])]
    pub only_code: bool,

    /// Show only markdown cells
    #[arg(long = "only-markdown", alias = "markdown", conflicts_with_all = ["cell", "cell_index"])]
    pub only_markdown: bool,
}

pub fn execute(args: ReadArgs) -> Result<()> {
    let notebook = notebook::read_notebook(&args.file)?;
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    // Handle specific cell by ID
    if let Some(ref cell_id) = args.cell {
        let (index, cell) = common::find_cell_by_id(&notebook.cells, cell_id)?;
        output_cell_with_optional_output(cell, index, &format, args.with_outputs)?;
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

        output_cell_with_optional_output(&cell, index, &format, args.with_outputs)?;
        return Ok(());
    }

    // Handle --only-code flag
    if args.only_code {
        output_code_cells(&notebook.cells, &format, args.with_outputs)?;
        return Ok(());
    }

    // Handle --only-markdown flag
    if args.only_markdown {
        output_markdown_cells(&notebook.cells, &format)?;
        return Ok(());
    }

    // Default: show notebook structure
    output_notebook_structure(&notebook, &format, args.with_outputs)?;
    Ok(())
}

fn output_cell_with_optional_output(
    cell: &Cell,
    index: usize,
    format: &OutputFormat,
    with_outputs: bool,
) -> Result<()> {
    let source = common::cell_to_string(cell);
    let id = common::cell_id_to_string(cell);

    match format {
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
        OutputFormat::Text => {
            let cell_type = match cell {
                Cell::Code { .. } => "Code",
                Cell::Markdown { .. } => "Markdown",
                Cell::Raw { .. } => "Raw",
            };
            println!("Cell {} [{}] (ID: {})", index, cell_type, id);
            println!("---");
            println!("{}", source);

            if let Cell::Code {
                execution_count,
                outputs,
                ..
            } = cell
            {
                if let Some(count) = execution_count {
                    println!("\nExecution count: {}", count);
                }

                if with_outputs && !outputs.is_empty() {
                    println!("\nOutputs:");
                    println!("---");
                    for (i, output) in outputs.iter().enumerate() {
                        if i > 0 {
                            println!("\n---\n");
                        }
                        print_output_text(output);
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_output_text(output: &Output) {
    match output {
        Output::ExecuteResult(result) => {
            println!(
                "Execute Result (execution_count: {:?}):",
                result.execution_count
            );
            print_output_data(&result.data);
        }
        Output::DisplayData(data) => {
            println!("Display Data:");
            print_output_data(&data.data);
        }
        Output::Stream { name, text } => {
            println!("Stream ({}):", name);
            print!("{}", text.0);
        }
        Output::Error(error) => {
            println!("Error: {}", error.ename);
            println!("Message: {}", error.evalue);
            if !error.traceback.is_empty() {
                println!("Traceback:");
                for line in &error.traceback {
                    println!("  {}", line);
                }
            }
        }
    }
}

fn print_output_data(data: &Media) {
    // Media is an opaque type from jupyter_protocol, we need to serialize it to access
    if let Ok(json_val) = serde_json::to_value(data) {
        if let Some(obj) = json_val.as_object() {
            for (mime_type, content) in obj {
                println!("  [{}]:", mime_type);
                match content {
                    serde_json::Value::String(s) => println!("{}", s),
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            if let serde_json::Value::String(s) = item {
                                print!("{}", s);
                            }
                        }
                        println!();
                    }
                    _ => println!(
                        "{}",
                        serde_json::to_string_pretty(content).unwrap_or_default()
                    ),
                }
            }
        }
    }
}

fn output_code_cells(cells: &[Cell], format: &OutputFormat, with_outputs: bool) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let code_cells: Vec<serde_json::Value> = cells
                .iter()
                .enumerate()
                .filter_map(|(index, cell)| {
                    if let Cell::Code { .. } = cell {
                        let mut cell_json = serde_json::to_value(cell).ok()?;

                        // Add index as convenience field
                        if let Some(obj) = cell_json.as_object_mut() {
                            obj.insert("index".to_string(), json!(index));

                            // Remove outputs if not requested
                            if !with_outputs {
                                obj.remove("outputs");
                            }
                        }

                        Some(cell_json)
                    } else {
                        None
                    }
                })
                .collect();
            let output = json!({ "cells": code_cells });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Text => {
            for (index, cell) in cells.iter().enumerate() {
                if let Cell::Code {
                    execution_count,
                    outputs,
                    ..
                } = cell
                {
                    println!(
                        "=== Cell {} (ID: {}) ===",
                        index,
                        common::cell_id_to_string(cell)
                    );
                    if let Some(count) = execution_count {
                        println!("Execution count: {}", count);
                    }
                    println!("{}", common::cell_to_string(cell));

                    if with_outputs && !outputs.is_empty() {
                        println!("\nOutputs:");
                        println!("---");
                        for (i, output) in outputs.iter().enumerate() {
                            if i > 0 {
                                println!("\n---\n");
                            }
                            print_output_text(output);
                        }
                    }
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn output_markdown_cells(cells: &[Cell], format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let markdown_cells: Vec<serde_json::Value> = cells
                .iter()
                .enumerate()
                .filter_map(|(index, cell)| {
                    if let Cell::Markdown { .. } = cell {
                        let mut cell_json = serde_json::to_value(cell).ok()?;

                        // Add index as convenience field
                        if let Some(obj) = cell_json.as_object_mut() {
                            obj.insert("index".to_string(), json!(index));
                        }

                        Some(cell_json)
                    } else {
                        None
                    }
                })
                .collect();

            let output = json!({ "cells": markdown_cells });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Text => {
            for (index, cell) in cells.iter().enumerate() {
                if let Cell::Markdown { .. } = cell {
                    println!(
                        "=== Cell {} (ID: {}) ===",
                        index,
                        common::cell_id_to_string(cell)
                    );
                    println!("{}", common::cell_to_string(cell));
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn output_notebook_structure(
    notebook: &nbformat::v4::Notebook,
    format: &OutputFormat,
    with_outputs: bool,
) -> Result<()> {
    // If with_outputs is true, show full cells with outputs instead of structure
    if with_outputs {
        match format {
            OutputFormat::Json => {
                let cells: Vec<serde_json::Value> = notebook
                    .cells
                    .iter()
                    .enumerate()
                    .map(|(index, cell)| {
                        let mut cell_json = serde_json::to_value(cell).unwrap_or(json!(null));

                        // Add index as convenience field
                        if let Some(obj) = cell_json.as_object_mut() {
                            obj.insert("index".to_string(), json!(index));
                        }

                        cell_json
                    })
                    .collect();
                let output = json!({ "cells": cells });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            OutputFormat::Text => {
                for (index, cell) in notebook.cells.iter().enumerate() {
                    let cell_type = match cell {
                        Cell::Code { .. } => "Code",
                        Cell::Markdown { .. } => "Markdown",
                        Cell::Raw { .. } => "Raw",
                    };
                    println!(
                        "=== Cell {} [{}] (ID: {}) ===",
                        index,
                        cell_type,
                        common::cell_id_to_string(cell)
                    );
                    println!("{}", common::cell_to_string(cell));

                    if let Cell::Code {
                        execution_count,
                        outputs,
                        ..
                    } = cell
                    {
                        if let Some(count) = execution_count {
                            println!("\nExecution count: {}", count);
                        }
                        if !outputs.is_empty() {
                            println!("\nOutputs:");
                            println!("---");
                            for (i, output) in outputs.iter().enumerate() {
                                if i > 0 {
                                    println!("\n---\n");
                                }
                                print_output_text(output);
                            }
                        }
                    }
                    println!();
                }
            }
        }
        return Ok(());
    }

    // Default structure view - serialize cells directly with nbformat
    match format {
        OutputFormat::Json => {
            let cells: Vec<serde_json::Value> = notebook
                .cells
                .iter()
                .enumerate()
                .map(|(index, cell)| {
                    let mut cell_json = serde_json::to_value(cell).unwrap_or(json!(null));

                    // Add index as convenience field and remove outputs
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
        OutputFormat::Text => {
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

            println!("Notebook Structure");
            println!("==================");
            println!("Total cells: {}", notebook.cells.len());
            println!("Code cells: {}", code_cells);
            println!("Markdown cells: {}", markdown_cells);
            if raw_cells > 0 {
                println!("Raw cells: {}", raw_cells);
            }
            if let Some(k) = kernel {
                println!("Kernel: {}", k);
            }
            println!("\nCells:");
            for (index, cell) in notebook.cells.iter().enumerate() {
                let source = common::cell_to_string(cell);

                let (cell_type, executed) = match cell {
                    Cell::Code {
                        execution_count, ..
                    } => ("code", Some(execution_count.is_some())),
                    Cell::Markdown { .. } => ("markdown", None),
                    Cell::Raw { .. } => ("raw", None),
                };

                let executed_marker = match executed {
                    Some(true) => " [✓]",
                    Some(false) => " [ ]",
                    None => "",
                };

                println!(
                    "  {} [{}]{} (ID: {}):",
                    index,
                    cell_type,
                    executed_marker,
                    common::cell_id_to_string(cell),
                );

                // Indent cell content for better readability
                for line in source.lines() {
                    println!("    {}", line);
                }
                println!();
            }
        }
    }
    Ok(())
}
