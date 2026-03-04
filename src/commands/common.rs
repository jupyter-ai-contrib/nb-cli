use anyhow::{bail, Context, Result};
use nbformat::v4::Cell;
use std::io::{self, Read};

/// Normalize a cell index, supporting negative indexing (e.g., -1 for last cell)
pub fn normalize_index(index: i32, len: usize) -> Result<usize> {
    if index < 0 {
        let abs_index = index.abs() as usize;
        if abs_index > len {
            bail!("Negative index {} out of range (notebook has {} cells)", index, len);
        }
        Ok(len - abs_index)
    } else {
        let idx = index as usize;
        if idx >= len {
            bail!("Cell index {} out of range (notebook has {} cells)", index, len);
        }
        Ok(idx)
    }
}

/// Find a cell by ID, returning its index and an immutable reference
pub fn find_cell_by_id<'a>(cells: &'a [Cell], cell_id: &str) -> Result<(usize, &'a Cell)> {
    for (index, cell) in cells.iter().enumerate() {
        if cell.id().as_str() == cell_id {
            return Ok((index, cell));
        }
    }
    bail!("Cell with ID '{}' not found", cell_id);
}

/// Find a cell by ID, returning its index and a mutable reference
pub fn find_cell_by_id_mut<'a>(cells: &'a mut [Cell], cell_id: &str) -> Result<(usize, &'a mut Cell)> {
    for (index, cell) in cells.iter_mut().enumerate() {
        if cell.id().as_str() == cell_id {
            return Ok((index, cell));
        }
    }
    bail!("Cell with ID '{}' not found", cell_id);
}

/// Parse source text from a string or stdin (when input is "-")
/// Returns a Vec<String> in Jupyter's line format
pub fn parse_source(input: &str) -> Result<Vec<String>> {
    let text = if input == "-" {
        // Read from stdin
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)
            .context("Failed to read from stdin")?;
        buffer
    } else {
        input.to_string()
    };

    Ok(split_source(&text))
}

/// Split text into Jupyter's line format (Vec<String> with newlines preserved)
/// Each line ends with \n except the last line
pub fn split_source(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![];
    }

    let ends_with_newline = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();

    if lines.is_empty() {
        return vec![text.to_string()];
    }

    let mut result: Vec<String> = lines.iter()
        .enumerate()
        .map(|(i, line)| {
            if i < lines.len() - 1 {
                format!("{}\n", line)
            } else if ends_with_newline {
                format!("{}\n", line)
            } else {
                line.to_string()
            }
        })
        .collect();

    // If text ended with newline, add empty string at end
    if ends_with_newline {
        result.push(String::new());
    }

    result
}

/// Convert cell source to a single string
pub fn cell_to_string(cell: &Cell) -> String {
    cell.source().join("")
}

/// Convert cell ID to a string
pub fn cell_id_to_string(cell: &Cell) -> String {
    cell.id().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_index() {
        assert_eq!(normalize_index(0, 5).unwrap(), 0);
        assert_eq!(normalize_index(4, 5).unwrap(), 4);
        assert_eq!(normalize_index(-1, 5).unwrap(), 4);
        assert_eq!(normalize_index(-5, 5).unwrap(), 0);
        assert!(normalize_index(5, 5).is_err());
        assert!(normalize_index(-6, 5).is_err());
    }

    #[test]
    fn test_split_source() {
        assert_eq!(split_source(""), Vec::<String>::new());
        assert_eq!(split_source("single line"), vec!["single line"]);
        assert_eq!(
            split_source("line1\nline2\nline3"),
            vec!["line1\n", "line2\n", "line3"]
        );
        assert_eq!(
            split_source("line1\nline2\n"),
            vec!["line1\n", "line2\n", ""]
        );
    }
}
