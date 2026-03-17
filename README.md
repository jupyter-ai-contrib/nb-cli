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

## AI Agent Integration

To enable AI agents (like Claude Code) to work seamlessly with Jupyter notebooks using `nb`:

### Install the Skill

**Option 1: Using the Vercel Skills Tool (Recommended)**

```bash
npx skills install jupyter-ai-contrib/nb-cli
```

**Option 2: Manual Installation**

Copy the skill directory to your agent's skill location:
- **Claude Code/Cline**: `~/.claude/skills/notebook-cli/` or `~/.cline/skills/notebook-cli/`
- **Other agents**: Consult your agent's documentation for the skills directory

```bash
# Example for Claude Code
mkdir -p ~/.claude/skills/notebook-cli
cp -r skills/notebook-cli/* ~/.claude/skills/notebook-cli/
```

### Configure Your Agent

Add the following instruction to your project's agent configuration file (`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, etc.):

```markdown
## Working with Notebooks (.ipynb files)

When the user asks to read, edit, execute, or work with .ipynb files, use the notebook-cli skill, which provides the `nb` command-line tool. Do not use the built-in Read/Write tools for `.ipynb` files.
```

This ensures your AI agent uses the `nb` CLI for all notebook operations instead of attempting to parse JSON directly.

## Quick Start

```bash
# Create and build a notebook
nb create analysis.ipynb
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb read analysis.ipynb

# Execute and view results
nb execute analysis.ipynb
nb read analysis.ipynb --with-outputs
```

## Local Mode

**Default behavior. Operations directly modify `.ipynb` files.**

Local mode lets you create, edit, execute, and query notebooks on disk without any server running. All changes are written directly to the `.ipynb` file.

```bash
# Create and edit
nb create notebook.ipynb
nb cell add notebook.ipynb --source "x = 1 + 1"
nb cell update notebook.ipynb --cell-index 0 --source "x = 2 + 2"

# Read and search
nb read notebook.ipynb                    # View structure
nb read notebook.ipynb --cell-index 0     # View specific cell
nb search notebook.ipynb "import"         # Find patterns
nb search notebook.ipynb --with-errors    # Find cells with errors

# Execute locally (native Rust implementation)
nb execute notebook.ipynb --cell-index 0  # Execute specific cell
nb execute notebook.ipynb                 # Execute all cells
```

**Note**: Local execution requires a Jupyter kernel to be installed (e.g., `pip install ipykernel` for Python). The CLI communicates directly with kernels via ZeroMQ using native Rust.

## Remote Mode

**Connect to a running JupyterLab server for real-time synchronization.**

When you connect to a Jupyter server, the CLI uses Y.js for conflict-free real-time updates. Changes appear instantly in your open JupyterLab tabs, and you can execute code using the server's kernel.

### Connect to a Server

**Auto-detection (recommended):**
```bash
nb connect
```

Automatically finds running Jupyter servers, validates them, and connects. If multiple servers are found, you'll get an interactive prompt to choose one.

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
nb cell execute notebook.ipynb --cell f9l030

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
nb cell update experiment.ipynb --cell-index 0 --append "\nprint('done')"

# Execute via remote kernel
nb execute experiment.ipynb --cell-index 0

# Disconnect when switching projects
nb disconnect
```

## Commands

| Command | Purpose |
|---------|---------|
| `nb create <path>` | Create a new notebook |
| `nb read <path>` | Read notebook cells and metadata |
| `nb execute <path>` | Execute cells in notebook |
| `nb search <path> <pattern>` | Search text and errors in notebook cells |
| `nb cell add <path> --source <code>` | Add a new cell |
| `nb cell update <path> --cell-index <index>` | Update an existing cell |
| `nb cell delete <path> --cell-index <index>` | Delete a cell |
| `nb execute <path> --cell-index <index>` | Execute a specific cell |
| `nb output clear <path>` | Clear cell outputs |
| `nb connect [--server URL --token TOKEN]` | Connect to Jupyter server (auto-detects if no args) |
| `nb status` | Show current connection status |
| `nb disconnect` | Disconnect from server |

Use `--help` with any command for full details and options.

## Key Features

### Cell Referencing

Two ways to reference cells:
- **Index**: `--cell-index 0` or `-i 0` (position-based, supports negative indexing: `-1` = last cell)
- **ID**: `--cell "my-cell"` or `-c "my-cell"` (stable, doesn't change when cells move)

### Output Format

Control output format for better integration with your workflow:
- **JSON** (default): Structured, nbformat-compliant for programmatic use
- **Text** (`-f text`): Human-readable for terminal viewing

```bash
nb read notebook.ipynb -f text
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
nb create analysis.ipynb --template basic
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb execute analysis.ipynb
```

**Debug and fix cells:**
```bash
# Find problematic cells
nb search notebook.ipynb --with-errors

# Inspect specific cell with outputs
nb read notebook.ipynb --cell-index 5 --with-outputs

# Fix the cell
nb cell update notebook.ipynb --cell-index 5 --source "fixed code"

# Re-execute
nb execute notebook.ipynb --cell-index 5
```

**Extract specific content:**
```bash
nb read notebook.ipynb --only-code         # All code cells
nb read notebook.ipynb --only-markdown     # All markdown
nb read notebook.ipynb --cell-index -1     # Last cell
```

**For AI agents:**
```bash
# Analyze all code in a notebook
nb read notebook.ipynb --only-code

# Find cells with errors
nb search notebook.ipynb --with-errors

# Add analysis cell and execute
nb cell add experiment.ipynb --source "df.describe()"
nb execute experiment.ipynb --cell-index -1
```

## Examples

See `examples/` directory for sample notebooks demonstrating various cell types and outputs.

## License

[BSD-3-Clause](LICENSE)
