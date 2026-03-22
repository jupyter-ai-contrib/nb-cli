use crate::commands::common::{self, OutputFormat};
use crate::commands::markdown_renderer::{self, IndexedCell};
use crate::notebook;
use anyhow::{bail, Result};
use clap::{Parser, ValueEnum};
use jupyter_protocol::media::Media;
use nbformat::v4::{Cell, Notebook, Output};
use regex::Regex;
use serde_json::json;
use std::fs;

#[derive(Clone, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum SearchScope {
    /// Search only in cell source code
    Source,
    /// Search only in cell outputs
    Output,
    /// Search in both source and outputs
    All,
}

#[derive(Clone, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum CellTypeFilter {
    Code,
    Markdown,
    Raw,
    All,
}

#[derive(Parser)]
pub struct SearchArgs {
    /// Path to notebook file
    pub file: String,

    /// Pattern to search for (literal string, optional if --with-errors is used)
    pub pattern: Option<String>,

    /// What to search: source code, outputs, or both
    #[arg(long, default_value = "source")]
    pub scope: SearchScope,

    /// Filter by cell type
    #[arg(long, default_value = "all")]
    pub cell_type: CellTypeFilter,

    /// Case-insensitive search
    #[arg(short = 'i', long)]
    pub ignore_case: bool,

    /// Return only cell IDs/indices (compact output)
    #[arg(short = 'l', long)]
    pub list_only: bool,

    /// Return cells with error outputs (pattern optional)
    #[arg(long)]
    pub with_errors: bool,

    /// Output in JSON format instead of text
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug)]
struct Match {
    location: String,
    line_number: usize,
    line_content: String,
}

#[derive(Debug)]
struct SearchResult {
    cell_index: usize,
    cell_id: String,
    cell_type: String,
    matches: Vec<Match>,
}

pub fn execute(args: SearchArgs) -> Result<()> {
    // Validate arguments
    if args.pattern.is_none() && !args.with_errors {
        bail!("Must provide a pattern or use --with-errors flag");
    }

    // Special handling for --with-errors
    if args.with_errors {
        return execute_with_errors(&args);
    }

    // Normal pattern search
    let pattern_str = args.pattern.as_ref().unwrap();

    // Validate pattern is not empty
    if pattern_str.trim().is_empty() {
        bail!("Pattern cannot be empty");
    }

    // Phase 1: Text pre-filter - quick scan of raw file
    let file_content = fs::read_to_string(&args.file)?;

    // Build regex pattern with case sensitivity
    let pattern = if args.ignore_case {
        format!("(?i){}", regex::escape(pattern_str))
    } else {
        regex::escape(pattern_str)
    };

    let re = Regex::new(&pattern)?;

    // Early exit if pattern not found in raw text
    if !re.is_match(&file_content) {
        print_empty_results(&args)?;
        return Ok(());
    }

    // Phase 2: Parse and extract structured matches
    let notebook = notebook::read_notebook(&args.file)?;
    let mut results = Vec::new();

    for (index, cell) in notebook.cells.iter().enumerate() {
        // Filter by cell type
        if !matches_cell_type(cell, &args.cell_type) {
            continue;
        }

        let mut cell_matches = Vec::new();

        // Search in source
        if should_search_source(&args.scope) {
            let source = common::cell_to_string(cell);
            cell_matches.extend(find_matches(&re, &source, "source"));
        }

        // Search in outputs (code cells only)
        if should_search_output(&args.scope) {
            if let Cell::Code { outputs, .. } = cell {
                for output in outputs {
                    let output_text = extract_output_text(output);
                    cell_matches.extend(find_matches(&re, &output_text, "output"));
                }
            }
        }

        if !cell_matches.is_empty() {
            results.push(SearchResult {
                cell_index: index,
                cell_id: common::cell_id_to_string(cell),
                cell_type: get_cell_type_name(cell),
                matches: cell_matches,
            });
        }
    }

    // Format and print results
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Markdown
    };
    match format {
        OutputFormat::Json => print_json(&results, &notebook, &args)?,
        OutputFormat::Markdown | OutputFormat::Text => print_text(&results, &notebook, &args)?,
    }

    Ok(())
}

fn execute_with_errors(args: &SearchArgs) -> Result<()> {
    let notebook = notebook::read_notebook(&args.file)?;
    let mut results = Vec::new();

    // Build regex if pattern is provided
    let re = if let Some(ref pattern_str) = args.pattern {
        if pattern_str.trim().is_empty() {
            bail!("Pattern cannot be empty");
        }
        let pattern = if args.ignore_case {
            format!("(?i){}", regex::escape(pattern_str))
        } else {
            regex::escape(pattern_str)
        };
        Some(Regex::new(&pattern)?)
    } else {
        None
    };

    for (index, cell) in notebook.cells.iter().enumerate() {
        // Filter by cell type
        if !matches_cell_type(cell, &args.cell_type) {
            continue;
        }

        // Only look at code cells with outputs
        if let Cell::Code { outputs, .. } = cell {
            let mut cell_matches = Vec::new();

            for output in outputs {
                // Only process error outputs
                if let Output::Error(error) = output {
                    let error_text = format!(
                        "{}\n{}\n{}",
                        error.ename,
                        error.evalue,
                        error.traceback.join("\n")
                    );

                    // If pattern provided, search in error text
                    if let Some(ref regex) = re {
                        cell_matches.extend(find_matches(regex, &error_text, "error"));
                    } else {
                        // No pattern - just mark that this cell has an error
                        cell_matches.push(Match {
                            location: "error".to_string(),
                            line_number: 0,
                            line_content: format!("{}: {}", error.ename, error.evalue),
                        });
                    }
                }
            }

            if !cell_matches.is_empty() {
                results.push(SearchResult {
                    cell_index: index,
                    cell_id: common::cell_id_to_string(cell),
                    cell_type: get_cell_type_name(cell),
                    matches: cell_matches,
                });
            }
        }
    }

    // Format and print results
    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Markdown
    };
    match format {
        OutputFormat::Json => print_json(&results, &notebook, args)?,
        OutputFormat::Markdown | OutputFormat::Text => print_text(&results, &notebook, args)?,
    }

    Ok(())
}

fn matches_cell_type(cell: &Cell, filter: &CellTypeFilter) -> bool {
    match filter {
        CellTypeFilter::All => true,
        CellTypeFilter::Code => matches!(cell, Cell::Code { .. }),
        CellTypeFilter::Markdown => matches!(cell, Cell::Markdown { .. }),
        CellTypeFilter::Raw => matches!(cell, Cell::Raw { .. }),
    }
}

fn should_search_source(scope: &SearchScope) -> bool {
    matches!(scope, SearchScope::Source | SearchScope::All)
}

fn should_search_output(scope: &SearchScope) -> bool {
    matches!(scope, SearchScope::Output | SearchScope::All)
}

fn get_cell_type_name(cell: &Cell) -> String {
    match cell {
        Cell::Code { .. } => "code".to_string(),
        Cell::Markdown { .. } => "markdown".to_string(),
        Cell::Raw { .. } => "raw".to_string(),
    }
}

fn find_matches(re: &Regex, text: &str, location: &str) -> Vec<Match> {
    let mut matches = Vec::new();

    for (line_number, line) in text.lines().enumerate() {
        if re.is_match(line) {
            matches.push(Match {
                location: location.to_string(),
                line_number,
                line_content: line.to_string(),
            });
        }
    }

    matches
}

fn extract_output_text(output: &Output) -> String {
    match output {
        Output::ExecuteResult(result) => extract_media_text(&result.data),
        Output::DisplayData(data) => extract_media_text(&data.data),
        Output::Stream { text, .. } => text.0.clone(),
        Output::Error(error) => {
            format!(
                "{}\n{}\n{}",
                error.ename,
                error.evalue,
                error.traceback.join("\n")
            )
        }
    }
}

fn extract_media_text(media: &Media) -> String {
    if let Ok(json_val) = serde_json::to_value(media) {
        if let Some(obj) = json_val.as_object() {
            for mime in &["text/plain", "text/html", "application/json"] {
                if let Some(content) = obj.get(*mime) {
                    return extract_string_from_json(content);
                }
            }
        }
    }
    String::new()
}

fn extract_string_from_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => value.to_string(),
    }
}

fn print_empty_results(args: &SearchArgs) -> Result<()> {
    let pattern_display = args.pattern.as_deref().unwrap_or("(no pattern)");

    let format = if args.json {
        OutputFormat::Json
    } else {
        OutputFormat::Markdown
    };
    match format {
        OutputFormat::Json => {
            if args.list_only {
                let output = json!({
                    "pattern": args.pattern,
                    "total_matches": 0,
                    "cells_matched": 0,
                    "cell_indices": [],
                    "cell_ids": []
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                let output = json!({
                    "pattern": args.pattern,
                    "total_matches": 0,
                    "cells_matched": 0,
                    "results": []
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
        OutputFormat::Markdown | OutputFormat::Text => {
            if args.with_errors {
                println!("# No cells with errors found");
            } else {
                println!("# No matches found for pattern: {}", pattern_display);
            }
        }
    }
    Ok(())
}

fn print_json(results: &[SearchResult], notebook: &Notebook, args: &SearchArgs) -> Result<()> {
    let total_matches: usize = results.iter().map(|r| r.matches.len()).sum();
    let cells_matched = results.len();

    if args.list_only {
        let cell_indices: Vec<usize> = results.iter().map(|r| r.cell_index).collect();
        let cell_ids: Vec<String> = results.iter().map(|r| r.cell_id.clone()).collect();

        let output = json!({
            "pattern": args.pattern,
            "with_errors": args.with_errors,
            "total_matches": total_matches,
            "cells_matched": cells_matched,
            "cell_indices": cell_indices,
            "cell_ids": cell_ids
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let results_json: Vec<serde_json::Value> = results
            .iter()
            .map(|result| {
                let source = notebook
                    .cells
                    .get(result.cell_index)
                    .map(|c| c.source().to_vec())
                    .unwrap_or_default();
                json!({
                    "cell_index": result.cell_index,
                    "cell_id": result.cell_id,
                    "cell_type": result.cell_type,
                    "source": source,
                    "match_count": result.matches.len(),
                    "matches": result.matches.iter().map(|m| {
                        json!({
                            "location": m.location,
                            "line_number": m.line_number,
                            "line_content": m.line_content
                        })
                    }).collect::<Vec<_>>()
                })
            })
            .collect();

        let output = json!({
            "pattern": args.pattern,
            "with_errors": args.with_errors,
            "total_matches": total_matches,
            "cells_matched": cells_matched,
            "results": results_json
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

fn print_text(results: &[SearchResult], notebook: &Notebook, args: &SearchArgs) -> Result<()> {
    let total_matches: usize = results.iter().map(|r| r.matches.len()).sum();
    let cells_matched = results.len();

    if results.is_empty() {
        if args.with_errors {
            println!("No cells with errors found");
        } else {
            println!(
                "No matches found for pattern: {}",
                args.pattern.as_deref().unwrap_or("(no pattern)")
            );
        }
        return Ok(());
    }

    // Print summary as a comment
    if args.with_errors && args.pattern.is_none() {
        println!("# Found {} cell(s) with errors\n", cells_matched);
    } else {
        println!(
            "# Found {} match(es) in {} cell(s)\n",
            total_matches, cells_matched
        );
    }

    if args.list_only {
        println!("# Matched cells:");
        for result in results {
            println!(
                "# - Cell {} [{}] (ID: {})",
                result.cell_index, result.cell_type, result.cell_id
            );
        }
    } else {
        // Render matching cells in AI-Optimized Markdown format with original indices
        let indexed: Vec<IndexedCell> = results
            .iter()
            .filter_map(|r| {
                notebook.cells.get(r.cell_index).map(|cell| IndexedCell {
                    index: r.cell_index,
                    cell,
                })
            })
            .collect();

        let markdown = markdown_renderer::render_indexed_cells_markdown(
            notebook, &indexed, true, // include outputs
            None, // no output dir for inline display
            4000, // default inline limit
        )?;

        print!("{}", markdown);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nbformat::v4::{CellId, CellMetadata};
    use std::collections::HashMap;
    use uuid::Uuid;

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

    #[test]
    fn test_extract_string_from_json() {
        let string_val = serde_json::json!("hello");
        assert_eq!(extract_string_from_json(&string_val), "hello");

        let array_val = serde_json::json!(["line1\n", "line2\n"]);
        assert_eq!(extract_string_from_json(&array_val), "line1\nline2\n");
    }

    #[test]
    fn test_matches_cell_type() {
        let code_cell = Cell::Code {
            id: CellId::from(Uuid::new_v4()),
            metadata: create_empty_metadata(),
            execution_count: None,
            source: vec![],
            outputs: vec![],
        };
        assert!(matches_cell_type(&code_cell, &CellTypeFilter::All));
        assert!(matches_cell_type(&code_cell, &CellTypeFilter::Code));
        assert!(!matches_cell_type(&code_cell, &CellTypeFilter::Markdown));
    }

    #[test]
    fn test_get_cell_type_name() {
        let code_cell = Cell::Code {
            id: CellId::from(Uuid::new_v4()),
            metadata: create_empty_metadata(),
            execution_count: None,
            source: vec![],
            outputs: vec![],
        };
        assert_eq!(get_cell_type_name(&code_cell), "code");
    }
}
