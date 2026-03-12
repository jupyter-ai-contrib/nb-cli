---
name: notebook-cli
description: ALWAYS use the `nb` CLI for ALL Jupyter notebook operations instead of built-in tools (Read, NotebookEdit, etc). This includes reading, creating, editing cells, executing, and searching notebooks. Provides programmatic access with JSON output for AI agents. Supports both local file-based and remote real-time collaboration modes. REQUIRED for all .ipynb files in this project.
---

# Working with Jupyter Notebooks using nb

**IMPORTANT**: Use the custom `nb` tool (Rust-based CLI) for ALL notebook operations instead of built-in tools like Read or NotebookEdit. This includes reading notebooks.

## Quick Reference (Most Common Commands)

```bash
# ALWAYS check --help first if unsure: nb --help, nb notebook --help, nb cell --help

# Read entire notebook (NOT "nb list" - use "notebook read")
nb notebook read notebook.ipynb

# Read specific cell (use --cell or -c, NOT --index)
nb notebook read notebook.ipynb --cell 2
nb notebook read notebook.ipynb -c -1  # last cell

# Execute entire notebook
nb notebook execute notebook.ipynb

# Update cell (use --cell, NOT --index)
nb cell update notebook.ipynb --cell 2 --source "new code"

# Add cell
nb cell add notebook.ipynb --source "print('hello')"
```

## Common Mistakes to Avoid

- ❌ `nb list` → ✅ `nb notebook read`
- ❌ `--index` → ✅ `--cell` or `-c`
- ❌ Forgetting to check `--help` → ✅ Always use `nb <command> --help` when unsure

## Create Notebook

```bash
# Create empty notebook
nb notebook create notebook.ipynb

# Create with template
nb notebook create notebook.ipynb --template basic
nb notebook create notebook.ipynb --template markdown

# Create with specific kernel
nb notebook create notebook.ipynb --kernel python3 --language python

# Force overwrite if exists
nb notebook create notebook.ipynb --force

# Output as text instead of JSON
nb notebook create notebook.ipynb -f text
```

## Read Notebook

```bash
# Read entire notebook
nb notebook read notebook.ipynb

# Read specific cell by index
nb notebook read notebook.ipynb --cell 0
nb notebook read notebook.ipynb -c -1  # Last cell

# Read specific cell by ID
nb notebook read notebook.ipynb --cell-id "abc123"

# Read with outputs included
nb notebook read notebook.ipynb -c 0 --with-outputs

# Filter by cell type
nb notebook read notebook.ipynb --only-code
nb notebook read notebook.ipynb --only-markdown

# Output as text
nb notebook read notebook.ipynb -f text
```

## Read Cell

```bash
# Read specific cell by index
nb notebook read notebook.ipynb --cell 0
nb notebook read notebook.ipynb -c 2
nb notebook read notebook.ipynb -c -1  # Last cell

# Read specific cell by ID (more stable)
nb notebook read notebook.ipynb --cell-id "unique-cell-id"

# Read cell with its outputs
nb notebook read notebook.ipynb -c 0 --with-outputs
```

## Add Cell

```bash
# Add code cell at end
nb cell add notebook.ipynb --source "print('Hello')"

# Add markdown cell
nb cell add notebook.ipynb --type markdown --source "# Title"

# Add at specific position
nb cell add notebook.ipynb --source "import pandas" --insert-at 0
nb cell add notebook.ipynb -s "code" -i 2

# Add after/before specific cell
nb cell add notebook.ipynb --source "code" --after "cell-id-123"
nb cell add notebook.ipynb --source "code" --before "cell-id-456"

# Add with custom ID
nb cell add notebook.ipynb --source "code" --id "my-custom-id"

# Read from stdin
echo "print('Hello')" | nb cell add notebook.ipynb --source -
```

## Update Cell

```bash
# Update cell by index
nb cell update notebook.ipynb --cell 0 --source "new code"
nb cell update notebook.ipynb -c -1 -s "updated last cell"

# Update cell by ID
nb cell update notebook.ipynb --cell-id "abc123" --source "new code"

# Append to existing content
nb cell update notebook.ipynb -c 0 --append "\nprint('more code')"

# Change cell type
nb cell update notebook.ipynb -c 0 --type markdown

# Read from stdin
echo "new content" | nb cell update notebook.ipynb -c 0 --source -
```

## Delete Cell

```bash
# Delete by index
nb cell delete notebook.ipynb --cell 0
nb cell delete notebook.ipynb -c -1  # Last cell

# Delete by cell ID
nb cell delete notebook.ipynb --cell-id "abc123"

# Delete range (exclusive end)
nb cell delete notebook.ipynb --range 0:3  # Deletes cells 0, 1, 2

# Delete multiple cells by index
nb cell delete notebook.ipynb -c 0 -c 2 -c 5
```

## Execute Notebook

```bash
# Execute entire notebook
nb notebook execute notebook.ipynb

# Execute with specific kernel
nb notebook execute notebook.ipynb --kernel python3

# Execute with custom timeout per cell
nb notebook execute notebook.ipynb --timeout 60

# Continue on errors
nb notebook execute notebook.ipynb --allow-errors

# Execute cell range
nb notebook execute notebook.ipynb --start 0 --end 5

# Execute with remote server
nb notebook execute notebook.ipynb --server http://localhost:8888 --token "token123"

# Output as JSON
nb notebook execute notebook.ipynb --format json
```

## Execute Cell

```bash
# Execute cell by index
nb cell execute notebook.ipynb --cell 0
nb cell execute notebook.ipynb -c -1  # Last cell

# Execute cell by ID
nb cell execute notebook.ipynb --cell-id "abc123"

# Execute with specific kernel
nb cell execute notebook.ipynb -c 0 --kernel python3

# Execute with custom timeout
nb cell execute notebook.ipynb -c 0 --timeout 60

# Continue on errors
nb cell execute notebook.ipynb -c 0 --allow-errors

# Dry run (don't update file)
nb cell execute notebook.ipynb -c 0 --dry-run

# Execute with remote server
nb cell execute notebook.ipynb -c 0 --server http://localhost:8888 --token "token123"
```

## Clear Outputs

```bash
# Clear all outputs
nb output clear notebook.ipynb --all

# Clear specific cell by index
nb output clear notebook.ipynb --cell 0
nb output clear notebook.ipynb -c -1  # Last cell

# Clear specific cell by ID
nb output clear notebook.ipynb --cell-id "abc123"

# Preserve execution count when clearing
nb output clear notebook.ipynb --all --keep-execution-count
```

## Delete Outputs

```bash
# Same as clear - use output clear command
nb output clear notebook.ipynb --all
nb output clear notebook.ipynb -c 0
```

## Search Notebook

```bash
# Search in source code (default)
nb notebook search notebook.ipynb "pattern"

# Search in outputs
nb notebook search notebook.ipynb "pattern" --scope output

# Search in both source and outputs
nb notebook search notebook.ipynb "pattern" --scope all

# Case-insensitive search
nb notebook search notebook.ipynb "pattern" --ignore-case

# Filter by cell type
nb notebook search notebook.ipynb "pattern" --cell-type code
nb notebook search notebook.ipynb "pattern" --cell-type markdown

# Find cells with errors
nb notebook search notebook.ipynb --with-errors

# Return only cell IDs/indices
nb notebook search notebook.ipynb "pattern" --list-only
```

## Cell Referencing

- **By index**: `--cell N` or `-c N` (0-based, supports negative like `-1` for last)
- **By ID**: `--cell-id "id"` or `-i "id"` (stable, doesn't change when cells move)

## Output Format

All commands support `--format` or `-f` flag:
- `json` (default): Machine-readable JSON output
- `text`: Human-readable formatted output
