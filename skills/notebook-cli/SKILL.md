---
name: notebook-cli
description: Use the `nb` CLI for all Jupyter notebook (`.ipynb`) operations, including reading, inspecting, creating, editing cells, deleting cells, clearing outputs, searching, executing, and working with connected JupyterLab sessions. Required when an agent needs deterministic notebook manipulation without directly reading or writing raw notebook JSON.
---

# Notebook CLI

Use `nb` for every `.ipynb` operation. Do not read, write, patch, or edit notebook JSON directly when `nb` can perform the task.

## Core Rules

- Inspect before editing: run `nb read <notebook> --no-output` unless outputs are relevant.
- Prefer the default AI-Optimized Markdown output from `nb read`; use `--json` only when nbformat JSON is specifically needed.
- Prefer cell IDs (`--cell` / `-c`) for durable edits after inspecting a notebook. Use indexes (`--cell-index` / `-i`) for quick positional work; negative indexes are supported.
- Use `--no-output` when summarizing structure or source. Include outputs only when diagnosing results, failures, plots, or displayed values.
- Use stdin (`--source -`) for multi-line or quoted content to avoid shell escaping mistakes.
- If the user specifies `uv` or the workspace is clearly a `uv` project, treat `uv` as the notebook execution environment for the whole task. Use `--uv` on `nb` commands that support it, such as `nb create` and `nb execute`.
- When creating a notebook with several sections, prefer one multi-cell `nb cell add <notebook> --source -` operation with `@@markdown` and `@@code` sentinels instead of many one-cell add commands.
- When notebook code depends on third-party packages, add a dependency-install cell at the top of the notebook before imports if package availability is uncertain.
- If a `uv` environment is present, the dependency cell should use `!uv pip install ...`.
- If a `uv` environment is not present, the dependency cell should use `%pip install ...`.
- Run `nb <command> --help` or `nb cell <subcommand> --help` when command syntax is uncertain.
- In connected mode, let `nb` use the saved connection from `nb connect`. Do not write secret tokens into commands, prompts, logs, or examples; if auto-detection is unavailable, ask the user to establish the connection manually.
- Before running non-notebook Python commands that should match the active notebook environment, use `nb status --python` and run commands through the returned prefix.

## Trust Boundary

`nb` is an external executable that can read notebook content and, in connected mode, use saved Jupyter connection state. Before first use in a workspace, verify the command you will run:

```bash
command -v nb
nb --version
```

Use the repository-built or documented `nb-cli` binary. If `nb` is missing, resolves to an unexpected path, or reports an unexpected version/name, stop and ask the user to install or select the trusted `nb-cli` binary. Do not pass notebook contents, outputs, server URLs, or connection state to an unverified executable.

## Common Workflows

### Create a Notebook

```bash
nb create analysis.ipynb
nb create analysis.ipynb --kernel python3
nb create notes.ipynb --markdown
```

Read [references/create.md](references/create.md) when creating new notebooks, choosing kernels, or overwriting an existing notebook.

### Inspect a Notebook

```bash
nb read analysis.ipynb --no-output
nb read analysis.ipynb --cell-index 3
nb search analysis.ipynb "fit_model"
```

Read [references/read.md](references/read.md) for filters, output handling, and parsing the AI markdown format.

### Edit Cells

```bash
nb cell update analysis.ipynb --cell "cell-id" --source -
nb cell add analysis.ipynb --type markdown --source "# Results"
nb cell delete analysis.ipynb --cell-index -1
```

Read [references/edit-cells.md](references/edit-cells.md) before making multi-cell edits, preserving metadata, inserting around cell IDs, or deleting ranges.

### Execute and Debug

```bash
nb execute analysis.ipynb --cell-index 4
nb execute analysis.ipynb --start 0 --end 5 --allow-errors
nb read analysis.ipynb --cell-index 4
```

Read [references/execute.md](references/execute.md) for kernel selection, timeouts, environment flags, and failure-oriented workflows.

### Work with JupyterLab

```bash
nb connect
nb status
nb cell update analysis.ipynb --cell "cell-id" --source -
nb disconnect
```

Read [references/connect-mode.md](references/connect-mode.md) when a notebook is open in JupyterLab or changes must sync through a running server.

### Manage Outputs

```bash
nb read analysis.ipynb --limit 8000 --output-dir ./notebook-outputs
nb output clear analysis.ipynb
nb output clean
```

Read [references/output-format.md](references/output-format.md) for sentinel parsing, externalized output files, clearing outputs, and commit hygiene.

### Author Quality Notebooks

Read [references/best-practices.md](references/best-practices.md) for research-based guidelines on narrative structure, code organization, reproducibility, and naming conventions when creating or reviewing notebooks.

## Permission Note

If the agent cannot run `nb` or connected-mode commands because of sandbox policy, ask the user to allow the needed command. For recurring use, the project or user rules should allow the `nb` command prefix.

## Validation Prompts

Use [references/validation-prompts.md](references/validation-prompts.md) when checking whether this skill still guides agents toward the intended `nb` workflows.
