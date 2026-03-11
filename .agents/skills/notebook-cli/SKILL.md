---
name: nb
description: Use the custom Rust-based nb for working with Jupyter notebooks instead of built-in tools. Provides programmatic access to notebook operations (read, create, edit cells, execute, search) with JSON output for AI agents. Supports both local file-based and remote real-time collaboration modes. Invoke when working with .ipynb files in this project.
---

# Working with Jupyter Notebooks using nb

Use the custom `nb` tool (Rust-based CLI) for programmatic notebook manipulation instead of Claude Code's built-in notebook operations.

## Project Context

- **Location**: `/Users/pijain/projects/2026/nb`
- **Binary**: `./target/debug/nb` (build with `cargo build` if needed)
- **Output**: JSON by default (ideal for parsing), use `-f text` for human-readable format

## Command Structure

```bash
nb notebook <command>  # create, read, execute, search
nb cell <command>      # add, update, delete, execute
nb output <command>    # clear
nb connect/status/disconnect  # Connection management
```

Use `--help` with any command for detailed options.

## Operating Modes

### Local Mode (Default)
Direct file manipulation:
```bash
nb cell add <file> --source "code"
```

### Remote Mode
Real-time sync with JupyterLab (use after `nb connect` or with `--server`/`--token`):
```bash
nb connect --server http://localhost:8888 --token <token>
nb cell add <file> --source "code"  # Syncs instantly to open notebook
nb status  # Check connection
nb disconnect
```

## Essential Operations

### Reading
```bash
# Overview with all cells
nb notebook read <file>

# Specific cell
nb notebook read <file> --cell 0
nb notebook read <file> --cell-id "my-cell"

# With execution outputs
nb notebook read <file> -c 0 --with-outputs

# Filter by type
nb notebook read <file> --only-code
nb notebook read <file> --only-markdown
```

### Creating & Editing
```bash
# Create
nb notebook create <file> [--template basic|markdown]

# Add cell
nb cell add <file> --source "code" [--type code|markdown]

# Update cell
nb cell update <file> --cell 0 --source "new content"
nb cell update <file> --cell 0 --append "\nmore code"

# Delete
nb cell delete <file> --cell 0
```

### Execution
```bash
# Execute single cell
nb cell execute <file> --cell 0

# Execute notebook
nb notebook execute <file> [--start N --end M]

# With options
nb cell execute <file> -c 0 --timeout 60 --allow-errors
```

### Searching
```bash
# Search in source
nb notebook search <file> <pattern>

# Find errors
nb notebook search <file> --with-errors

# Search in outputs or all
nb notebook search <file> <pattern> --scope output|all
```

### Output Management
```bash
# Clear all
nb output clear <file> --all

# Clear specific cell
nb output clear <file> --cell 0
```

## Cell Referencing

- **By index**: `--cell N` (0-based, supports `-1` for last cell)
- **By ID**: `--cell-id "id"` (stable, doesn't change when cells move)

## Typical Agent Workflows

**Analyze code**:
```bash
nb notebook read <file> --only-code
```

**Debug**:
```bash
nb notebook search <file> --with-errors
nb notebook read <file> -c N --with-outputs
```

**Fix and verify**:
```bash
nb cell update <file> -c N --source "fixed"
nb cell execute <file> -c N
```

**Build notebook**:
```bash
nb notebook create <file>
nb cell add <file> --source "import pandas"
nb cell add <file> --source "# Title" --type markdown
```

## Important Notes

- All commands output JSON following nbformat specification
- Escape sequences (`\n`, `\t`) automatically interpreted in `--source`/`--append`
- Use `connect` command to save server credentials for repeated operations
- Real-time sync via Y.js when working with open JupyterLab notebooks
