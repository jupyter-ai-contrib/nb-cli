# Jupyter CLI

A command-line interface tool for working with Jupyter notebooks (`.ipynb` files). Built with Rust for performance and designed for programmatic interaction with notebooks, especially by AI agents.

## Features

- **Create notebooks** - Generate new notebooks with templates
- **Read notebook content** - View cells, outputs, and metadata
- **Add cells** - Insert code, markdown, or raw cells
- **Update cells** - Modify cell content and types
- **Delete cells** - Remove cells by index or ID
- **Clear outputs** - Clean execution outputs and counts
- **Real-time collaboration** - Edit notebooks open in JupyterLab with instant sync via Y.js
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

## Real-Time Collaboration

The CLI supports real-time editing of notebooks that are open in JupyterLab using Y.js (Yjs) collaborative editing. When you provide a Jupyter server URL and authentication token, changes are synced instantly without file conflicts.

### How It Works

When you add or update cells with `--server` and `--token` options:

1. **Session Detection** - Checks if the notebook is open in JupyterLab
2. **Smart Routing**:
   - If open: Uses Y.js for real-time updates (changes appear instantly in JupyterLab)
   - If closed: Falls back to file-based updates
3. **Conflict-Free** - JupyterLab handles file persistence, avoiding "out of band change" errors

### Real-Time Cell Operations

```bash
# Add a cell to an open notebook (appears instantly)
jupyter-cli cell add notebook.ipynb \
  --source "print('real-time!')" \
  --server http://localhost:8888 \
  --token your-jupyter-token

# Update a cell in real-time
jupyter-cli cell update notebook.ipynb \
  --cell 0 \
  --source "print('updated live!')" \
  --server http://localhost:8888 \
  --token your-jupyter-token

# Append to a cell (supports escape sequences)
jupyter-cli cell update notebook.ipynb \
  --cell 0 \
  --append '\n# Added via CLI' \
  --server http://localhost:8888 \
  --token your-jupyter-token
```

### Getting Your Jupyter Token

Find your token in one of these ways:

1. **From the terminal** when you start Jupyter:
   ```
   jupyter lab
   # Look for: http://localhost:8888/?token=abc123...
   ```

2. **From JupyterLab** - Help → "Copy Shareable Link" (extract token from URL)

3. **From config**:
   ```bash
   jupyter server list
   ```

### Escape Sequences

When providing source code with `--source` or `--append`, escape sequences are automatically interpreted:

- `\n` - Newline
- `\t` - Tab
- `\r` - Carriage return
- `\\` - Literal backslash
- `\'` - Single quote
- `\"` - Double quote

```bash
# Multi-line code with proper newlines
jupyter-cli cell add notebook.ipynb \
  --source 'def hello():\n    print("world")'
```

### Use Cases

**AI Agents**: Add cells to running notebooks without stopping the kernel or causing conflicts:
```bash
# Agent adds analysis cell to open notebook
jupyter-cli cell add experiment.ipynb \
  --source "df.describe()" \
  --server $JUPYTER_URL \
  --token $JUPYTER_TOKEN
```

**Automation**: Update notebooks from scripts while viewing results in JupyterLab:
```bash
# Script updates config cell
jupyter-cli cell update config.ipynb \
  --cell-id "params" \
  --source "BATCH_SIZE = 64" \
  --server http://localhost:8888 \
  --token $TOKEN
```

**Without Server Args**: Commands work normally with file-based updates:
```bash
# Traditional file-based update (no real-time sync)
jupyter-cli cell add notebook.ipynb --source "print('hello')"
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
  "raw_cells": 0,
  "kernel": "python3",
  "cells": [
    {
      "index": 0,
      "id": "intro-cell",
      "cell_type": "markdown",
      "metadata": {},
      "source": [
        "# Data Analysis Example\n\nThis notebook demonstrates..."
      ]
    },
    {
      "index": 1,
      "id": "imports-cell",
      "cell_type": "code",
      "execution_count": 1,
      "metadata": {},
      "source": [
        "import pandas as pd\nimport numpy as np"
      ],
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

### Read Cells with Outputs

Include execution outputs along with cell content:

```bash
# View a specific cell with its outputs
jupyter-cli notebook read notebook.ipynb -c 3 --with-outputs
# or use short form
jupyter-cli notebook read notebook.ipynb -c 3 -o

# View entire notebook with all outputs
jupyter-cli notebook read notebook.ipynb --with-outputs

# View only code cells with their outputs
jupyter-cli notebook read notebook.ipynb --only-code --with-outputs
```

This shows both the cell source code and any execution outputs together.

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
      "cell_type": "code",
      "execution_count": 1,
      "metadata": {},
      "source": [
        "import pandas as pd\nimport numpy as np"
      ]
    }
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

# Real-time update to open notebook
jupyter-cli cell add notebook.ipynb \
  --source "print('live!')" \
  --server http://localhost:8888 \
  --token your-token
```

### Update a Cell

Modify an existing cell:

```bash
# Replace cell content
jupyter-cli cell update notebook.ipynb --cell 0 --source "new content"

# Append to cell (supports escape sequences like \n for newlines)
jupyter-cli cell update notebook.ipynb --cell 0 --append "\nmore code"

# Change cell type
jupyter-cli cell update notebook.ipynb --cell 0 --type markdown

# Update by cell ID
jupyter-cli cell update notebook.ipynb --cell-id "my-cell" --source "updated"

# Real-time update to open notebook
jupyter-cli cell update notebook.ipynb \
  --cell 0 \
  --source "print('updated live!')" \
  --server http://localhost:8888 \
  --token your-token

# Append with newlines in real-time
jupyter-cli cell update notebook.ipynb \
  --cell 0 \
  --append '\n# New comment\nprint("more code")' \
  --server http://localhost:8888 \
  --token your-token
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

**JSON Format:** All JSON output follows the [nbformat specification](https://nbformat.readthedocs.io/), preserving all cell fields (`cell_type`, `source`, `metadata`, `execution_count`, `outputs`, etc.). This ensures compatibility with other Jupyter tools and APIs. An additional `index` field is included for convenience when working with positional references.

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
# See what a cell produced (includes both source and outputs)
jupyter-cli notebook read notebook.ipynb -c 5 --with-outputs

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
  - `add_cell.rs` - Add cells (with Y.js support)
  - `update_cell.rs` - Update cells (with Y.js support)
  - `delete_cell.rs` - Delete cells
  - `clear_outputs.rs` - Clear outputs
  - `search.rs` - Search notebook cells
  - `execute_cell.rs` / `execute_notebook.rs` - Cell execution
  - `common.rs` - Shared utilities
- **`src/execution/`** - Cell execution and real-time collaboration
  - `remote/` - Y.js and Jupyter server integration
    - `ydoc.rs` - Y.js document connection and sync
    - `ydoc_notebook_ops.rs` - Y.js notebook operations (add/update cells)
    - `session_check.rs` - Detect if notebook is open in JupyterLab
- **`examples/`** - Sample notebooks for testing

## Dependencies

- **nbformat** - Jupyter notebook parsing (nbformat v4 specification)
- **jupyter-protocol** - Output data structures
- **yrs** - Y.js CRDT for real-time collaboration
- **tokio** - Async runtime for real-time operations
- **tokio-tungstenite** - WebSocket client for Jupyter server
- **clap** - CLI argument parsing
- **serde/serde_json** - JSON serialization
- **anyhow** - Error handling
- **uuid** - Cell ID generation
- **reqwest** - HTTP client for Jupyter API

## Roadmap

Completed:
- ✅ Real-time collaboration via Y.js (cell add/update)
- ✅ `notebook search` - Find patterns in notebooks
- ✅ `notebook execute` / `cell execute` - Run cells with kernel

Future enhancements:
- `notebook convert` - Export to .py, .md, etc.
- `notebook merge` - Combine multiple notebooks
- `cell move` - Reorder cells
- `cell copy` - Duplicate cells
- Multiple cell operations (ranges, comma-separated indices)
- Real-time cell delete operation

## License

MIT
