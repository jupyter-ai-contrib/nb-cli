# nb - Notebook CLI

A fast, programmatic command-line interface for working with Jupyter notebooks. Built with Rust and designed for AI agents, automation scripts, and developers who need reliable notebook manipulation.

## Purpose

- **Agent-friendly**: JSON output following nbformat specification for easy parsing
- **Local & Remote**: Work with notebook files directly or sync with running JupyterLab servers
- **Real-time collaboration**: Edit notebooks open in JupyterLab via Y.js with instant sync
- **Reliable**: Built with Rust for performance and correctness

## Installation

```bash
cargo build --release
```

The binary will be available at `target/release/nb`.

## Quick Start

```bash
# Create a notebook
nb notebook create analysis.ipynb

# Add a cell
nb cell add analysis.ipynb --source "print('hello')"

# Read notebook structure
nb notebook read analysis.ipynb

# Read specific cell with outputs
nb notebook read analysis.ipynb --cell 0 --with-outputs

# Update a cell
nb cell update analysis.ipynb --cell 0 --source "print('updated')"

# Execute cells
nb cell execute analysis.ipynb --cell 0
nb notebook execute analysis.ipynb  # Execute all cells

# Search for patterns
nb notebook search analysis.ipynb "import pandas"

# Delete a cell
nb cell delete analysis.ipynb --cell 0

# Clear outputs
nb output clear analysis.ipynb --all
```

## Local vs Remote Mode

### Local Mode (File-based)
Default behavior. Operations directly modify `.ipynb` files:

```bash
nb cell add notebook.ipynb --source "x = 1"
```

### Remote Mode (Real-time sync)
When working with notebooks open in JupyterLab, use `--server` and `--token` for instant synchronization:

```bash
# Connect to server (saves connection for future commands)
nb connect --server http://localhost:8888 --token your-jupyter-token

# Add cell - appears instantly in JupyterLab
nb cell add notebook.ipynb --source "df.head()"

# Update cell in real-time
nb cell update notebook.ipynb --cell 0 --append "\ndf.describe()"

# Check connection status
nb status

# Execute via remote kernel
nb cell execute notebook.ipynb --cell 0

# Disconnect when done
nb disconnect
```

**How it works**: The CLI detects if a notebook is open in JupyterLab and uses Y.js for conflict-free real-time updates. If the notebook isn't open, it falls back to file-based operations.

**Getting your token**:
- From terminal: Look for token in `jupyter lab` startup URL
- From JupyterLab: Help → "Copy Shareable Link"
- From command: `jupyter server list`

## Command Structure

Commands are organized by resource:

```bash
nb notebook <command>  # create, read, execute, search
nb cell <command>      # add, update, delete, execute
nb output <command>    # clear
nb connect/status/disconnect  # Server connection management
```

Use `--help` with any command for details.

## Essential Examples

### For AI Agents

```bash
# Analyze all code in a notebook
nb notebook read notebook.ipynb --only-code

# Find cells with errors
nb notebook search notebook.ipynb --with-errors

# Add analysis cell to running notebook
nb cell add experiment.ipynb \
  --source "df.describe()" \
  --server http://localhost:8888 \
  --token $TOKEN

# Debug: inspect cell with its outputs
nb notebook read notebook.ipynb --cell 5 --with-outputs
```

### Cell Referencing

Two ways to reference cells:
- **Index**: `--cell 0` (position-based, supports negative indexing: `-1` = last cell)
- **ID**: `--cell-id "my-cell"` (stable, doesn't change when cells move)

### Output Format

- **JSON** (default): Structured, nbformat-compliant for programmatic use
- **Text** (`-f text`): Human-readable for terminal viewing

```bash
nb notebook read notebook.ipynb -f text
```

## Multi-line Code

Escape sequences are automatically interpreted:

```bash
# Add cell with proper formatting
nb cell add notebook.ipynb \
  --source 'def hello():\n    print("world")\n\nhello()'

# Append to existing cell
nb cell update notebook.ipynb --cell 0 \
  --append '\n# Added comment\nprint("more")'
```

## Common Workflows

**Build notebook programmatically**:
```bash
nb notebook create analysis.ipynb --template basic
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
```

**Debug and fix**:
```bash
# Find problematic cells
nb notebook search notebook.ipynb --with-errors

# Inspect specific cell with outputs
nb notebook read notebook.ipynb -c 5 --with-outputs

# Fix the cell
nb cell update notebook.ipynb -c 5 --source "fixed code"

# Re-execute
nb cell execute notebook.ipynb -c 5
```

**Extract content**:
```bash
# All code cells
nb notebook read notebook.ipynb --only-code

# All markdown documentation
nb notebook read notebook.ipynb --only-markdown

# Last cell
nb notebook read notebook.ipynb -c -1
```

## Examples

See `examples/sample.ipynb` for a test notebook with various cell types and outputs.

## License

MIT
