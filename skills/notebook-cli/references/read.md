# Reading and Searching Notebooks

Use `nb read` for inspection, summarization, review, and extracting source or outputs.

## Default Read

```bash
nb read notebook.ipynb
nb read notebook.ipynb --no-output
```

The default output is AI-Optimized Markdown with line-oriented sentinels:

- `@@notebook {json}` for notebook metadata
- `@@cell {json}` for each cell, including `index`, `id`, `cell_type`, and `execution_count`
- `@@output {json}` for outputs, including `output_type`, MIME data, stream name, or externalized file path

Use this format for normal agent work. It is easier to review than raw notebook JSON while remaining deterministic enough to parse.

## Cell Selection

```bash
nb read notebook.ipynb --cell-index 0
nb read notebook.ipynb -i -1
nb read notebook.ipynb --cell "cell-id"
nb read notebook.ipynb -c "cell-id"
```

Indexes are zero-based and support negative values. IDs are more stable when cells may move.

## Filters

```bash
nb read notebook.ipynb --only-code --no-output
nb read notebook.ipynb --only-markdown
```

Use filters when the task only needs code or prose. `--only-markdown` does not include outputs.

## Outputs

```bash
nb read notebook.ipynb --no-output
nb read notebook.ipynb --limit 8000
nb read notebook.ipynb --output-dir ./notebook-outputs
```

Outputs are included by default. Prefer `--no-output` for structure/source review. Increase `--limit` or set `--output-dir` when large outputs need inspection.

## JSON

```bash
nb read notebook.ipynb --json
nb read notebook.ipynb --cell-index 2 --json
```

Use JSON only when an exact nbformat structure is required. Do not use JSON only to make routine parsing easier.

## Search

```bash
nb search notebook.ipynb "pattern"
nb search notebook.ipynb "pattern" --scope output
nb search notebook.ipynb "pattern" --scope all
nb search notebook.ipynb "pattern" --ignore-case
nb search notebook.ipynb "pattern" --cell-type code
nb search notebook.ipynb --with-errors
nb search notebook.ipynb "pattern" --list-only
```

Search source first when locating relevant code. Use `--scope output` or `--with-errors` when debugging execution results.
