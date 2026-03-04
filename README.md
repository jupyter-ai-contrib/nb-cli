# Jupyter CLI

A command-line interface tool for working with Jupyter notebooks (`.ipynb` files). Built with Rust for performance and designed for programmatic interaction with notebooks, especially by AI agents.

## Features

- **Create notebooks** - Generate new notebooks with templates
- **Read notebook content** - View cells, outputs, and metadata
- **Add cells** - Insert code, markdown, or raw cells
- **Update cells** - Modify cell content and types
- **Delete cells** - Remove cells by index or ID
- **Clear outputs** - Clean execution outputs and counts
- **Cell access by index or ID** - Reference cells by stable IDs or positional index
- **Filter by cell type** - Extract only code or markdown cells
- **Multiple output formats** - JSON (default) for agents, text for humans
- **Negative indexing** - Use `-1` to access the last cell, like Python
- **Resource-oriented commands** - Organized by resource (notebook, cell, output)
- **Fast and reliable** - Built with Rust using the `nbformat` crate

## Installation

```bash
cargo build --release
```

The binary will be available at `target/release/jupyter-cli`.

## Quick Start

```bash
# Create a new notebook
jupyter-cli notebook create my-notebook.ipynb

# View notebook overview
jupyter-cli notebook read my-notebook.ipynb

# Add a code cell
jupyter-cli cell add my-notebook.ipynb --source "print('hello')" --type code

# Read a specific cell
jupyter-cli notebook read my-notebook.ipynb --cell 0

# Update a cell
jupyter-cli cell update my-notebook.ipynb --cell 0 --source "print('updated')"

# Delete a cell
jupyter-cli cell delete my-notebook.ipynb --cell 0

# Clear outputs
jupyter-cli output clear my-notebook.ipynb --all
```

## Command Structure

The CLI is organized by resource type:

```bash
jupyter-cli notebook <command>  # Notebook operations
jupyter-cli cell <command>      # Cell operations
jupyter-cli output <command>    # Output operations
```

## Usage

### Create a Notebook

Create a new Jupyter notebook:

```bash
# Create empty notebook
jupyter-cli notebook create my-notebook.ipynb

# Create with basic template (one empty code cell)
jupyter-cli notebook create my-notebook.ipynb --template basic

# Create with markdown template (heading + code cell)
jupyter-cli notebook create my-notebook.ipynb --template markdown

# Specify kernel
jupyter-cli notebook create my-notebook.ipynb --kernel python3
```

### View Notebook Structure

Get an overview of the notebook with cell types, IDs, and execution status:

```bash
jupyter-cli notebook read notebook.ipynb
```

**Output (JSON):**
```json
{
  "cell_count": 7,
  "code_cells": 4,
  "markdown_cells": 3,
  "kernel": "python3",
  "cells": [
    {
      "index": 0,
      "id": "intro-cell",
      "type": "markdown",
      "preview": "# Data Analysis Example..."
    },
    {
      "index": 1,
      "id": "imports-cell",
      "type": "code",
      "preview": "import pandas as pd...",
      "executed": true
    }
  ]
}
```

### Read Specific Cell

By index (0-based):
```bash
jupyter-cli notebook read notebook.ipynb --cell 0
# or use short form
jupyter-cli notebook read notebook.ipynb -c 0
```

By cell ID (more stable - IDs don't change when cells are added/removed):
```bash
jupyter-cli notebook read notebook.ipynb --cell-id "intro-cell"
# or use short form
jupyter-cli notebook read notebook.ipynb -i "intro-cell"
```

Negative indexing (last cell):
```bash
jupyter-cli notebook read notebook.ipynb -c -1
```

### Read Cell Output

View the execution output of a code cell:

```bash
jupyter-cli notebook read notebook.ipynb -c 3 --with-output
# or use short form
jupyter-cli notebook read notebook.ipynb -c 3 -o
```

### Extract All Code Cells

Get all code cells for analysis:

```bash
jupyter-cli notebook read notebook.ipynb --only-code
# backward compatible alias
jupyter-cli notebook read notebook.ipynb --code
```

**Output:**
```json
{
  "cells": [
    {
      "index": 1,
      "id": "imports-cell",
      "source": "import pandas as pd\nimport numpy as np",
      "execution_count": 1
    },
    ...
  ]
}
```

### Extract All Markdown Cells

Get all documentation from the notebook:

```bash
jupyter-cli notebook read notebook.ipynb --only-markdown
# backward compatible alias
jupyter-cli notebook read notebook.ipynb --markdown
```

### Get All Outputs

Extract all execution outputs:

```bash
jupyter-cli notebook read notebook.ipynb --all-outputs
```

### Add a Cell

Add a new cell to a notebook:

```bash
# Add code cell at the end
jupyter-cli cell add notebook.ipynb --source "print('hello')" --type code

# Add markdown cell
jupyter-cli cell add notebook.ipynb --source "# Heading" --type markdown

# Insert at specific position
jupyter-cli cell add notebook.ipynb --source "x = 1" --insert-at 0

# Insert after a specific cell
jupyter-cli cell add notebook.ipynb --source "y = 2" --after "cell-id"

# Insert before a specific cell
jupyter-cli cell add notebook.ipynb --source "z = 3" --before "cell-id"

# Read from stdin
echo "import pandas" | jupyter-cli cell add notebook.ipynb --source -
```

### Update a Cell

Modify an existing cell:

```bash
# Replace cell content
jupyter-cli cell update notebook.ipynb --cell 0 --source "new content"

# Append to cell
jupyter-cli cell update notebook.ipynb --cell 0 --append "\nmore code"

# Change cell type
jupyter-cli cell update notebook.ipynb --cell 0 --type markdown

# Update by cell ID
jupyter-cli cell update notebook.ipynb --cell-id "my-cell" --source "updated"
```

### Delete a Cell

Remove cells from a notebook:

```bash
# Delete by index
jupyter-cli cell delete notebook.ipynb --cell 0

# Delete by cell ID
jupyter-cli cell delete notebook.ipynb --cell-id "my-cell"

# Delete last cell
jupyter-cli cell delete notebook.ipynb --cell -1
```

### Clear Outputs

Clear execution outputs from code cells:

```bash
# Clear all outputs
jupyter-cli output clear notebook.ipynb --all

# Clear specific cell output
jupyter-cli output clear notebook.ipynb --cell 0

# Clear by cell ID
jupyter-cli output clear notebook.ipynb --cell-id "my-cell"

# Keep execution counts (only clear output)
jupyter-cli output clear notebook.ipynb --all --keep-execution-count
```

### Output Formats

Use `-f text` or `--format text` for human-readable output:

```bash
jupyter-cli notebook read notebook.ipynb -f text
jupyter-cli notebook read notebook.ipynb -c 0 -f text
jupyter-cli cell add notebook.ipynb --source "x=1" -f text
```

## Command Reference

### Notebook Commands

```bash
jupyter-cli notebook create [OPTIONS] <FILE>
jupyter-cli notebook read [OPTIONS] <FILE>
```

### Cell Commands

```bash
jupyter-cli cell add [OPTIONS] <FILE>
jupyter-cli cell update [OPTIONS] <FILE>
jupyter-cli cell delete [OPTIONS] <FILE>
```

### Output Commands

```bash
jupyter-cli output clear [OPTIONS] <FILE>
```

Use `--help` with any command for detailed options:

```bash
jupyter-cli notebook read --help
jupyter-cli cell add --help
```

## Cell IDs vs Indexes

Jupyter notebooks support two ways to reference cells:

- **Index** (`--cell`): Position-based (0, 1, 2, ..., -1 for last). Simple but changes when cells are reordered.
- **ID** (`--cell-id`): Stable identifier (e.g., "intro-cell", "abc123"). Doesn't change when cells are moved.

**Recommendation:** Use `--cell-id` when you need stable references across notebook edits. Use `--cell` for quick interactive access.

## Agent Workflows

Common patterns for AI agents working with notebooks:

### Create and Build
```bash
# Create new notebook
jupyter-cli notebook create analysis.ipynb --template basic

# Add cells
jupyter-cli cell add analysis.ipynb --source "import pandas as pd" --type code
jupyter-cli cell add analysis.ipynb --source "# Data Analysis" --type markdown
```

### Analyze Code
```bash
# Get all code for analysis
jupyter-cli notebook read notebook.ipynb --only-code

# Check specific cell
jupyter-cli notebook read notebook.ipynb -i "data-processing"
```

### Debug and Modify
```bash
# See what a cell produced
jupyter-cli notebook read notebook.ipynb -c 5 -o

# Update problematic cell
jupyter-cli cell update notebook.ipynb -c 5 --source "fixed code"

# Clear outputs for re-execution
jupyter-cli output clear notebook.ipynb --all
```

### Extract Documentation
```bash
# Get markdown content
jupyter-cli notebook read notebook.ipynb --only-markdown
```

### Quick Inspection
```bash
# Overview
jupyter-cli notebook read notebook.ipynb

# Last cell
jupyter-cli notebook read notebook.ipynb -c -1
```

## Examples

See `examples/sample.ipynb` for a test notebook demonstrating various cell types and outputs.

## Architecture

- **`src/main.rs`** - CLI entry point with resource-based command structure
- **`src/notebook.rs`** - Notebook I/O using `nbformat` crate
- **`src/commands/`** - Command implementations
  - `read.rs` - Read/query operations
  - `create_notebook.rs` - Notebook creation
  - `add_cell.rs` - Add cells
  - `update_cell.rs` - Update cells
  - `delete_cell.rs` - Delete cells
  - `clear_outputs.rs` - Clear outputs
  - `common.rs` - Shared utilities
- **`examples/`** - Sample notebooks for testing

## Dependencies

- **nbformat** - Jupyter notebook parsing (nbformat v4 specification)
- **jupyter-protocol** - Output data structures
- **clap** - CLI argument parsing
- **serde/serde_json** - JSON serialization
- **anyhow** - Error handling
- **uuid** - Cell ID generation

## Roadmap

Future enhancements:

- `notebook search` - Find patterns in notebooks
- `notebook execute` - Run cells (requires kernel protocol)
- `notebook convert` - Export to .py, .md, etc.
- `notebook merge` - Combine multiple notebooks
- `cell move` - Reorder cells
- `cell copy` - Duplicate cells
- Multiple cell operations (ranges, comma-separated indices)

## License

MIT
