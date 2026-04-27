# Creating Notebooks

Use `nb create` to create new `.ipynb` files instead of writing notebook JSON.

## Basic Creation

```bash
nb create notebook.ipynb
nb create notebook
```

If the path does not end with `.ipynb`, `nb` appends the extension. New notebooks start with one empty code cell by default.

## Kernel Selection

```bash
nb create notebook.ipynb --kernel python3
nb create notebook.ipynb -k python3
```

The default kernel is `python3`. `nb` validates kernel availability and writes kernelspec metadata.

## Markdown Initial Cell

```bash
nb create notes.ipynb --markdown
```

Use this when the notebook should start as prose rather than code.

## Environment-Aware Kernel Discovery

```bash
nb create notebook.ipynb --uv
nb create notebook.ipynb --pixi
```

Use these flags when kernels should be discovered through the project's `uv` or `pixi` environment.

## Overwrite and JSON Result

```bash
nb create notebook.ipynb --force
nb create notebook.ipynb --json
```

Use `--force` only when replacing an existing notebook is intentional.
