# Executing Notebooks

Use `nb execute` to run notebooks or selected cells. Inspect results with `nb read` after execution unless the text output already contains the needed details.

## Execute All or Part

```bash
nb execute notebook.ipynb
nb execute notebook.ipynb --cell-index 0
nb execute notebook.ipynb -i -1
nb execute notebook.ipynb --cell "cell-id"
nb execute notebook.ipynb --start 0 --end 5
```

`--start` and `--end` are inclusive. Cell selectors conflict with ranges.

## Kernels and Timeouts

```bash
nb execute notebook.ipynb --kernel python3
nb execute notebook.ipynb --timeout 60
nb execute notebook.ipynb --allow-errors
```

If no kernel is specified, `nb` uses notebook metadata when available. Use `--allow-errors` when collecting failure outputs is more important than stopping at the first error.

## Environment-Aware Local Execution

```bash
nb execute notebook.ipynb --uv
nb execute notebook.ipynb --pixi
```

Use these flags when the project uses `uv` or `pixi` and kernels should be discovered through that environment.

Before running separate Python commands that should match the connected notebook environment:

```bash
nb status --python
$(nb status --python) python script.py
$(nb status --python) python -c "import numpy; print(numpy.__version__)"
```

The prefix may be `uv run`, `pixi run`, or empty for direct/system Python.

## Remote Execution

```bash
nb connect
nb execute notebook.ipynb
nb execute notebook.ipynb --cell "cell-id" --restart-kernel
```

Prefer `nb connect` plus saved connection state for remote work. Do not put Jupyter authentication tokens in commands, prompts, logs, or examples; if a manual server connection is required, ask the user to establish it. `--restart-kernel` is remote-mode only.

## Kernel Gateway

```bash
nb execute notebook.ipynb --gateway http://gateway:8888 --gateway-token "$KG_TOKEN"
nb execute notebook.ipynb --gateway http://gateway:8888 --gateway-token "$KG_TOKEN" --kernel-id <id>
nb execute notebook.ipynb --gateway http://gateway:8888 --gateway-token "$KG_TOKEN" --gateway-auth-scheme Bearer
```

Use `--gateway` for environments that expose only a Jupyter Kernel Gateway — kernels over REST and WebSocket, with no full notebook server. The notebook is read and written locally; only execution runs on the gateway. `--gateway-token` is required when `--gateway` is set. If no `--kernel-id` is given, `nb` reuses an existing kernel on the gateway when listing is permitted, otherwise it starts a new one. Default auth scheme is `token`; override with `--gateway-auth-scheme` only when the gateway requires another scheme.

Do not put gateway tokens into commands, prompts, logs, or examples. Source them from environment variables or saved configuration; if the token is not available through those channels, ask the user to run the command themselves.

## Debugging Failed Cells

1. Run `nb search notebook.ipynb --with-errors`.
2. Read the failing cell with outputs: `nb read notebook.ipynb --cell-index N`.
3. Patch the smallest relevant cell with `nb cell update`.
4. Execute the patched cell or a minimal range.
5. Read the affected cell again to confirm outputs.
