# nb - Notebook CLI

A fast, command-line interface for working with Jupyter notebooks. Designed for both humans and AI agents, with AI-Optimized Markdown format by default and JSON output available for programmatic use. Enables reliable notebook manipulation without opening a browser.

## AI-Optimized Markdown Format

The default output format uses line-oriented sentinels with JSON metadata, specifically designed for AI agents:

````markdown
@@notebook {"format":"ai-notebook","metadata":{"kernelspec":{...}}}

@@cell {"index":0,"id":"cell-id","cell_type":"code","execution_count":1}
```python
import pandas as pd
```
@@output {"output_type":"stream","name":"stdout"}
```text
Hello, world!
```
````

**Key Features:**
- **Line-oriented sentinels** (`@@notebook`, `@@cell`, `@@output`) for deterministic parsing
- **JSON metadata** with nbformat v4.5 compliant property names (`cell_type`, `output_type`)
- **Cell index field** for reliable positional references (even when IDs are missing)
- **Content-based hashing** using SHA256 for externalized output filenames
  - Prevents AI agents from guessing filenames
  - Same content always maps to same file (automatic deduplication)
- **Absolute paths** for all externalized outputs
- **40+ MIME types** with JupyterLab-compatible priority

**Format Structure:**
```
Line starts with @@   →  Parse as sentinel (notebook/cell/output)
Following JSON       →  Contains metadata (index, id, type, execution_count, etc.)
Content after JSON   →  Cell source or output content
Code/outputs         →  Wrapped in fenced code blocks with language hint
Markdown cells       →  Raw markdown text (no fence)
Large outputs        →  Externalized to files, path in @@output JSON
```

[![BSD-3-Clause License](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)

## Installation

### Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/jupyter-ai-contrib/nb-cli/main/install.sh | bash
```

This installs the binary to `~/.nb-cli/bin/nb`. Follow the instructions to add it to your PATH.

**Note**: If you get an error while installation, where your platform is not supported, please use `cargo install` or build from source.

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

> [!IMPORTANT]
> For Codex, `nb` must be allowed by Codex command rules, or the sandbox may block the command in `connect` mode. You can do this by adding `prefix_rule(pattern=["nb"], decision="allow")` to your `default.rules` file usually located at `~/.codex/rules/default.rules`. 

## Quick Start

```bash
# Create a notebook (starts with one empty code cell)
nb create analysis.ipynb

# Add cells
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb read analysis.ipynb

# Execute and view results (outputs included by default)
nb execute analysis.ipynb
nb read analysis.ipynb

# Control output externalization
nb read analysis.ipynb --limit 8000 --output-dir ./outputs
```

## Local Mode

**Default behavior. Operations directly modify `.ipynb` files.**

Local mode lets you create, edit, execute, and query notebooks on disk without any server running. All changes are written directly to the `.ipynb` file.

```bash
# Create and edit (creates notebook with single code cell)
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

**Environment-aware detection:**
```bash
nb connect --uv    # Detect servers running via uv
nb connect --pixi  # Detect servers running via pixi
```

When working with isolated project environments (uv or pixi), use these flags to discover Jupyter servers running within those environments. The CLI will automatically detect your project root and run `jupyter server list` through the appropriate environment manager.

**Manual connection:**
```bash
nb connect --server http://localhost:8888 --token your-jupyter-token
```

**Connection options:**
- `--server`: Server URL (e.g., `http://localhost:8888`)
- `--token`: Authentication token from Jupyter
- `--uv`: Use uv to run jupyter commands (mutually exclusive with `--pixi`)
- `--pixi`: Use pixi to run jupyter commands (mutually exclusive with `--uv`)

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

**How it works**: When connected, the CLI always uses Y.js for real-time collaborative editing. Changes sync instantly if the notebook is open in JupyterLab, or will appear when you open it later.

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
| `nb create <path>` | Create a new notebook with a single code cell |
| `nb read <path>` | Read notebook cells and metadata |
| `nb execute <path>` | Execute cells in notebook |
| `nb search <path> <pattern>` | Search text and errors in notebook cells |
| `nb cell add <path> --source <code>` | Add a new cell |
| `nb cell update <path> --cell-index <index>` | Update an existing cell |
| `nb cell delete <path> --cell-index <index>` | Delete a cell |
| `nb execute <path> --cell-index <index>` | Execute a specific cell |
| `nb output clear <path>` | Clear cell outputs |
| `nb connect [--server URL --token TOKEN] [--uv\|--pixi]` | Connect to Jupyter server (auto-detects if no args) |
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
- **AI-Optimized Markdown** (default): Line-oriented sentinels with JSON metadata for reliable parsing by AI agents
- **JSON** (`--json`): Structured, nbformat-compliant for programmatic use

```bash
# Default AI-Optimized Markdown output
nb read notebook.ipynb

# JSON output for programmatic use
nb read notebook.ipynb --json
```

### Output Externalization

Outputs are included by default. Large outputs (>4000 characters by default) are automatically externalized to separate files:

```bash
# Control externalization threshold (default: 4000)
nb read notebook.ipynb --limit 8000

# Specify output directory for externalized files
nb read notebook.ipynb --output-dir ./notebook-outputs

# Exclude outputs when not needed
nb read notebook.ipynb --no-output
```

**Benefits:**
- Content-based hashing (SHA256) prevents filename guessing by AI agents
- Same content always maps to same file (automatic deduplication)
- Absolute paths in output headers for reliable file access
- Keeps main output clean and focused

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
nb create analysis.ipynb
nb cell add analysis.ipynb --source "import pandas as pd"
nb cell add analysis.ipynb --source "# Analysis" --type markdown
nb execute analysis.ipynb
```

**Debug and fix cells:**
```bash
# Find problematic cells
nb search notebook.ipynb --with-errors

# Inspect specific cell (outputs included by default)
nb read notebook.ipynb --cell-index 5

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
# Read notebook (AI-Optimized Markdown format, outputs included by default)
nb read notebook.ipynb

# Analyze only code cells
nb read notebook.ipynb --only-code

# Control externalization for large outputs
nb read notebook.ipynb --limit 8000 --output-dir ./outputs

# Find cells with errors
nb search notebook.ipynb --with-errors

# Add analysis cell and execute
nb cell add experiment.ipynb --source "df.describe()"
nb execute experiment.ipynb --cell-index -1

# Parse the AI-Optimized Markdown output
# - Look for lines starting with @@ for sentinels
# - Parse JSON metadata after sentinel markers
# - Cell content follows headers (code in fenced blocks, markdown as raw text)
# - Large outputs externalized with absolute paths in @@output headers
```

## Examples

See `examples/` directory for sample notebooks demonstrating various cell types and outputs.

## License

[BSD-3-Clause](LICENSE)
