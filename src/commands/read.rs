use crate::commands::common;
use crate::notebook;
use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use nbformat::v4::{Cell, Output};
use jupyter_protocol::media::Media;
use serde::Serialize;
use serde_json::json;

#[derive(Clone, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Text,
}

#[derive(Parser)]
pub struct ReadArgs {
    /// Path to notebook file
    pub file: String,

    /// Output format
    #[arg(short, long, default_value = "json", value_name = "FORMAT")]
    pub format: OutputFormat,

    /// Get specific cell by index (supports negative indexing like -1)
    #[arg(short, long, value_name = "INDEX", conflicts_with_all = ["cell_id", "only_code", "only_markdown", "all_outputs"])]
    pub cell: Option<i32>,

    /// Get specific cell by ID (more stable than index)
    #[arg(short = 'i', long, value_name = "ID", conflicts_with_all = ["cell", "only_code", "only_markdown", "all_outputs"])]
    pub cell_id: Option<String>,

    /// Show cell execution output (requires --cell or --cell-id)
    #[arg(short = 'o', long = "with-output")]
    pub with_output: bool,

    /// Show only code cells
    #[arg(long = "only-code", alias = "code", conflicts_with_all = ["cell", "cell_id"])]
    pub only_code: bool,

    /// Show only markdown cells
    #[arg(long = "only-markdown", alias = "markdown", conflicts_with_all = ["cell", "cell_id"])]
    pub only_markdown: bool,

    /// Show all cell outputs
    #[arg(long = "all-outputs", conflicts_with_all = ["cell", "cell_id"])]
    pub all_outputs: bool,
}

#[derive(Serialize)]
struct CellInfo {
    index: usize,
    id: String,
    cell_type: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_output: Option<bool>,
}

#[derive(Serialize)]
struct NotebookStructure {
    cell_count: usize,
    code_cells: usize,
    markdown_cells: usize,
    raw_cells: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    kernel: Option<String>,
    cells: Vec<CellPreview>,
}

#[derive(Serialize)]
struct CellPreview {
    index: usize,
    id: String,
    #[serde(rename = "type")]
    cell_type: String,
    preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    executed: Option<bool>,
}

#[derive(Serialize)]
struct CodeCellsOutput {
    cells: Vec<CodeCellInfo>,
}

#[derive(Serialize)]
struct CodeCellInfo {
    index: usize,
    id: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_count: Option<i32>,
}

#[derive(Serialize)]
struct MarkdownCellsOutput {
    cells: Vec<MarkdownCellInfo>,
}

#[derive(Serialize)]
struct MarkdownCellInfo {
    index: usize,
    id: String,
    source: String,
}

pub fn execute(args: ReadArgs) -> Result<()> {
    // Validate that --with-output requires --cell or --cell-id
    if args.with_output && args.cell.is_none() && args.cell_id.is_none() {
        bail!("--with-output requires --cell or --cell-id");
    }

    let notebook = notebook::read_notebook(&args.file)?;

    // Handle specific cell by index
    if let Some(cell_index) = args.cell {
        let index = common::normalize_index(cell_index, notebook.cells.len())?;
        let cell = notebook.cells.get(index)
            .context(format!("Cell index {} out of range (notebook has {} cells)", index, notebook.cells.len()))?;

        if args.with_output {
            output_cell_output(&cell, index, &args.format)?;
        } else {
            output_cell_info(&cell, index, &args.format)?;
        }
        return Ok(());
    }

    // Handle specific cell by ID
    if let Some(ref cell_id) = args.cell_id {
        let (index, cell) = common::find_cell_by_id(&notebook.cells, cell_id)?;
        output_cell_info(cell, index, &args.format)?;
        return Ok(());
    }

    // Handle --only-code flag
    if args.only_code {
        output_code_cells(&notebook.cells, &args.format)?;
        return Ok(());
    }

    // Handle --only-markdown flag
    if args.only_markdown {
        output_markdown_cells(&notebook.cells, &args.format)?;
        return Ok(());
    }

    // Handle --all-outputs flag
    if args.all_outputs {
        output_all_outputs(&notebook.cells, &args.format)?;
        return Ok(());
    }

    // Default: show notebook structure
    output_notebook_structure(&notebook, &args.format)?;
    Ok(())
}


fn output_cell_info(cell: &Cell, index: usize, format: &OutputFormat) -> Result<()> {
    let source = common::cell_to_string(cell);
    let id = common::cell_id_to_string(cell);

    match format {
        OutputFormat::Json => {
            let info = match cell {
                Cell::Code { execution_count, outputs, .. } => CellInfo {
                    index,
                    id,
                    cell_type: "code".to_string(),
                    source,
                    execution_count: *execution_count,
                    has_output: Some(!outputs.is_empty()),
                },
                Cell::Markdown { .. } => CellInfo {
                    index,
                    id,
                    cell_type: "markdown".to_string(),
                    source,
                    execution_count: None,
                    has_output: None,
                },
                Cell::Raw { .. } => CellInfo {
                    index,
                    id,
                    cell_type: "raw".to_string(),
                    source,
                    execution_count: None,
                    has_output: None,
                },
            };
            println!("{}", serde_json::to_string_pretty(&info)?);
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
            if let Cell::Code { execution_count, .. } = cell {
                if let Some(count) = execution_count {
                    println!("\nExecution count: {}", count);
                }
            }
        }
    }
    Ok(())
}

fn output_cell_output(cell: &Cell, index: usize, format: &OutputFormat) -> Result<()> {
    let outputs = match cell {
        Cell::Code { outputs, .. } => outputs,
        _ => bail!("Cell {} is not a code cell and has no outputs", index),
    };

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(outputs)?);
        }
        OutputFormat::Text => {
            if outputs.is_empty() {
                println!("No output");
            } else {
                for (i, output) in outputs.iter().enumerate() {
                    if i > 0 {
                        println!("\n---\n");
                    }
                    print_output_text(output);
                }
            }
        }
    }
    Ok(())
}

fn print_output_text(output: &Output) {
    match output {
        Output::ExecuteResult(result) => {
            println!("Execute Result (execution_count: {:?}):", result.execution_count);
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
                    _ => println!("{}", serde_json::to_string_pretty(content).unwrap_or_default()),
                }
            }
        }
    }
}

fn output_code_cells(cells: &[Cell], format: &OutputFormat) -> Result<()> {
    let code_cells: Vec<CodeCellInfo> = cells
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| {
            if let Cell::Code { execution_count, .. } = cell {
                Some(CodeCellInfo {
                    index,
                    id: common::cell_id_to_string(cell),
                    source: common::cell_to_string(cell),
                    execution_count: *execution_count,
                })
            } else {
                None
            }
        })
        .collect();

    match format {
        OutputFormat::Json => {
            let output = CodeCellsOutput { cells: code_cells };
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Text => {
            for cell in code_cells {
                println!("=== Cell {} (ID: {}) ===", cell.index, cell.id);
                if let Some(count) = cell.execution_count {
                    println!("Execution count: {}", count);
                }
                println!("{}", cell.source);
                println!();
            }
        }
    }
    Ok(())
}

fn output_markdown_cells(cells: &[Cell], format: &OutputFormat) -> Result<()> {
    let markdown_cells: Vec<MarkdownCellInfo> = cells
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| {
            if let Cell::Markdown { .. } = cell {
                Some(MarkdownCellInfo {
                    index,
                    id: common::cell_id_to_string(cell),
                    source: common::cell_to_string(cell),
                })
            } else {
                None
            }
        })
        .collect();

    match format {
        OutputFormat::Json => {
            let output = MarkdownCellsOutput { cells: markdown_cells };
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Text => {
            for cell in markdown_cells {
                println!("=== Cell {} (ID: {}) ===", cell.index, cell.id);
                println!("{}", cell.source);
                println!();
            }
        }
    }
    Ok(())
}

fn output_all_outputs(cells: &[Cell], format: &OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            let outputs: Vec<_> = cells
                .iter()
                .enumerate()
                .filter_map(|(index, cell)| {
                    if let Cell::Code { execution_count, outputs, .. } = cell {
                        if !outputs.is_empty() {
                            return Some(json!({
                                "index": index,
                                "id": common::cell_id_to_string(cell),
                                "execution_count": execution_count,
                                "outputs": outputs,
                            }));
                        }
                    }
                    None
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&outputs)?);
        }
        OutputFormat::Text => {
            for (index, cell) in cells.iter().enumerate() {
                if let Cell::Code { execution_count, outputs, .. } = cell {
                    if !outputs.is_empty() {
                        println!("=== Cell {} (ID: {}) ===", index, common::cell_id_to_string(cell));
                        if let Some(count) = execution_count {
                            println!("Execution count: {}", count);
                        }
                        for (i, output) in outputs.iter().enumerate() {
                            if i > 0 {
                                println!("\n---\n");
                            }
                            print_output_text(output);
                        }
                        println!();
                    }
                }
            }
        }
    }
    Ok(())
}

fn output_notebook_structure(notebook: &nbformat::v4::Notebook, format: &OutputFormat) -> Result<()> {
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

    let cells: Vec<CellPreview> = notebook.cells.iter().enumerate().map(|(index, cell)| {
        let source = common::cell_to_string(cell);
        let preview = if source.len() > 80 {
            format!("{}...", &source[..77])
        } else {
            source
        }.replace('\n', " ");

        let (cell_type, executed) = match cell {
            Cell::Code { execution_count, .. } => ("code".to_string(), Some(execution_count.is_some())),
            Cell::Markdown { .. } => ("markdown".to_string(), None),
            Cell::Raw { .. } => ("raw".to_string(), None),
        };

        CellPreview {
            index,
            id: common::cell_id_to_string(cell),
            cell_type,
            preview,
            executed,
        }
    }).collect();

    match format {
        OutputFormat::Json => {
            let structure = NotebookStructure {
                cell_count: notebook.cells.len(),
                code_cells,
                markdown_cells,
                raw_cells,
                kernel,
                cells,
            };
            println!("{}", serde_json::to_string_pretty(&structure)?);
        }
        OutputFormat::Text => {
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
            for cell_preview in cells {
                let executed_marker = match cell_preview.executed {
                    Some(true) => " [✓]",
                    Some(false) => " [ ]",
                    None => "",
                };
                println!("  {} [{}]{} (ID: {}): {}",
                    cell_preview.index,
                    cell_preview.cell_type,
                    executed_marker,
                    cell_preview.id,
                    cell_preview.preview
                );
            }
        }
    }
    Ok(())
}
