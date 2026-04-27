# Connected JupyterLab Mode

Use connected mode when notebook edits or execution should sync with a running JupyterLab server.

## Connect

```bash
nb connect
nb connect --uv
nb connect --pixi
```

Auto-detection is preferred. Use `--uv` or `--pixi` when Jupyter is running inside that project environment.
Do not put Jupyter authentication tokens in agent-authored commands, prompts, logs, or examples. If auto-detection cannot find the server, ask the user to run the manual `nb connect` command themselves.

Connection info is saved in `.jupyter/cli.json` in the current directory. Later `nb` commands use that saved connection automatically.

## Check and Use the Connection

```bash
nb status
nb status --python
nb read notebook.ipynb --no-output
nb cell update notebook.ipynb --cell "cell-id" --source -
nb execute notebook.ipynb --cell "cell-id"
```

In remote mode, cell edits use Y.js real-time notebook updates so changes appear in open JupyterLab tabs.

## Disconnect

```bash
nb disconnect
```

Disconnect when switching projects, switching Jupyter servers, or returning to direct file-based local edits.

## Practical Guidance

- Run `nb status` before assuming connected mode is active.
- Use notebook paths relative to the project/server root when possible.
- If multiple Jupyter servers are running, prefer interactive `nb connect` selection over hand-copying tokens.
- Never include real or placeholder authentication tokens in commands you generate; use saved connection state instead.
- If commands fail because Codex sandbox policy blocks connected-mode process or network access, request user approval for the `nb` command.
