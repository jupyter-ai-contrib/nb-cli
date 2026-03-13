# nb - Notebook CLI

A fast, programmatic command-line interface for working with Jupyter notebooks. Designed for AI agents, automation scripts, and developers who need reliable notebook manipulation without opening a browser.

[![BSD-3-Clause License](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)

## Installation

### Quick Install (macOS Apple Silicon only)

```bash
curl -fsSL https://raw.githubusercontent.com/jupyter-ai-contrib/nb-cli/main/install.sh | bash
```

This installs the binary to `~/.nb-cli/bin/nb`. Follow the instructions to add it to your PATH.

**Note**: Pre-built binaries are currently only available for macOS Apple Silicon (M1/M2/M3/M4). For other platforms, please use `cargo install` or build from source.

### Manual Binary Download

**macOS (Apple Silicon - M1/M2/M3/M4)**:
```bash
curl -L https://github.com/jupyter-ai-contrib/nb-cli/releases/download/v0.0.1/nb-macos-arm64 -o nb
chmod +x nb
sudo mv nb /usr/local/bin/
```

**Other platforms**: Use `cargo install nb-cli` or build from source (see below).

### Install from crates.io

```bash
cargo install nb-cli
```

### Build from Source

```bash
git clone https://github.com/jupyter-ai-contrib/nb-cli.git
cd nb-cli
cargo build --release
```

The binary will be available at `target/release/nb`.

## Quick Start

```bash
# Create and build a notebook
nb notebook create analysis.ipynb
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb notebook read analysis.ipynb

# Execute and view results
nb notebook execute analysis.ipynb
nb notebook read analysis.ipynb --with-outputs
```

## Local Mode

**Default behavior. Operations directly modify `.ipynb` files.**

Local mode lets you create, edit, execute, and query notebooks on disk without any server running. All changes are written directly to the `.ipynb` file.

```bash
# Create and edit
nb notebook create notebook.ipynb
nb cell add notebook.ipynb --source "x = 1 + 1"
nb cell update notebook.ipynb --cell 0 --source "x = 2 + 2"

# Read and search
nb notebook read notebook.ipynb              # View structure
nb notebook read notebook.ipynb --cell 0     # View specific cell
nb notebook search notebook.ipynb "import"   # Find patterns

# Execute locally (requires Python dependencies)
nb cell execute notebook.ipynb --cell 0
nb notebook execute notebook.ipynb           # Execute all cells
```

### Python Dependencies for Local Execution

To execute notebooks in local mode, install:

```bash
pip install -r requirements.txt
```

Or manually:
```bash
pip install nbclient nbformat
```

**Note**: These dependencies are **only** required for local execution. Remote mode doesn't need them.

## Remote Mode

**Connect to a running JupyterLab server for real-time synchronization.**

When you connect to a Jupyter server, the CLI uses Y.js for conflict-free real-time updates. Changes appear instantly in your open JupyterLab tabs, and you can execute code using the server's kernel.

### Connect to a Server

**Auto-detection (recommended):**
```bash
nb connect
```

Automatically finds running Jupyter servers using `jupyter server list`, validates them, and connects. If multiple servers are found, you'll get an interactive prompt to choose one.

**Manual connection:**
```bash
nb connect --server http://localhost:8888 --token your-jupyter-token
```

**Connection options:**
- `--server`: Server URL (e.g., `http://localhost:8888`)
- `--token`: Authentication token from Jupyter

### Connection Persistence

Connection info is saved in `.jupyter/cli.json` in the current directory. All subsequent commands automatically use this connection until you disconnect or change directories.

```bash
# Connect once (auto-detect)
nb connect

# Future commands use saved connection
nb cell add notebook.ipynb --source "df.head()"
nb cell execute notebook.ipynb --cell 0

# Check current connection
nb status

# Disconnect when done
nb disconnect
```

**How it works**: When connected, the CLI detects if a notebook is open in JupyterLab and uses Y.js for instant sync. If the notebook isn't open on the server, operations fall back to file-based mode.

### Remote Mode Examples

```bash
# Connect automatically
nb connect

# Add cell - appears instantly in JupyterLab
nb cell add experiment.ipynb --source "df.describe()"

# Update cell in real-time
nb cell update experiment.ipynb --cell 0 --append "\nprint('done')"

# Execute via remote kernel
nb cell execute experiment.ipynb --cell 0

# Disconnect when switching projects
nb disconnect
```

## Commands

| Command | Purpose |
|---------|---------|
| `nb notebook create <path>` | Create a new notebook |
| `nb notebook read <path>` | Read notebook structure and cells |
| `nb notebook execute <path>` | Execute all cells in notebook |
| `nb notebook search <path> <pattern>` | Search for patterns in notebook |
| `nb cell add <path> --source <code>` | Add a new cell |
| `nb cell update <path> --cell <index>` | Update an existing cell |
| `nb cell delete <path> --cell <index>` | Delete a cell |
| `nb cell execute <path> --cell <index>` | Execute a specific cell |
| `nb output clear <path>` | Clear cell outputs |
| `nb connect [--server URL --token TOKEN]` | Connect to Jupyter server (auto-detects if no args) |
| `nb status` | Show current connection status |
| `nb disconnect` | Disconnect from server |

Use `--help` with any command for full details and options.

## Key Features

### Cell Referencing

Two ways to reference cells:
- **Index**: `--cell 0` (position-based, supports negative indexing: `-1` = last cell)
- **ID**: `--cell-id "my-cell"` (stable, doesn't change when cells move)

### Output Format

Control output format for better integration with your workflow:
- **JSON** (default): Structured, nbformat-compliant for programmatic use
- **Text** (`-f text`): Human-readable for terminal viewing

```bash
nb notebook read notebook.ipynb -f text
```

### Multi-line Code

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

**Build notebook programmatically:**
```bash
nb notebook create analysis.ipynb --template basic
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb notebook execute analysis.ipynb
```

**Debug and fix cells:**
```bash
# Find problematic cells
nb notebook search notebook.ipynb --with-errors

# Inspect specific cell with outputs
nb notebook read notebook.ipynb --cell 5 --with-outputs

# Fix the cell
nb cell update notebook.ipynb --cell 5 --source "fixed code"

# Re-execute
nb cell execute notebook.ipynb --cell 5
```

**Extract specific content:**
```bash
nb notebook read notebook.ipynb --only-code      # All code cells
nb notebook read notebook.ipynb --only-markdown  # All markdown
nb notebook read notebook.ipynb --cell -1        # Last cell
```

**For AI agents:**
```bash
# Analyze all code in a notebook
nb notebook read notebook.ipynb --only-code

# Find cells with errors
nb notebook search notebook.ipynb --with-errors

# Add analysis cell and execute
nb cell add experiment.ipynb --source "df.describe()"
nb cell execute experiment.ipynb --cell -1
```

## Examples

See `examples/` directory for sample notebooks demonstrating various cell types and outputs.

## License

[BSD-3-Clause](LICENSE)
