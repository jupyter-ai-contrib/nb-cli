# Validation Prompts

Use these prompts to check whether the skill leads agents toward the intended `nb` workflows. Run them against disposable fixture notebooks or copies of examples.

## Source Inspection

```text
Use $notebook-cli to summarize the structure and source cells of examples/sample.ipynb without reading raw notebook JSON and without including outputs.
```

Expected behavior: use `nb read examples/sample.ipynb --no-output`; summarize cells from AI-Optimized Markdown.

## Single-Cell Update

```text
Use $notebook-cli to replace the last code cell in a copy of examples/sample.ipynb with `print("done")`, then verify the notebook source.
```

Expected behavior: inspect first, use `nb cell update ... --cell-index -1 --source ...`, then verify with `nb read --no-output`.

## Multi-Cell Add

```text
Use $notebook-cli to add a markdown heading followed by two code cells to a copy of examples/data_analysis.ipynb.
```

Expected behavior: use `nb cell add --source -` with `@@markdown` and `@@code` sentinels.

## Failure Debugging

```text
Use $notebook-cli to find cells with execution errors in tests/fixtures/with_error.ipynb and report the failing cell index and error text.
```

Expected behavior: use `nb search --with-errors` and `nb read` for the failing cell with outputs.

## Output Hygiene

```text
Use $notebook-cli to clear outputs from a copy of tests/fixtures/with_outputs.ipynb while preserving source cells, then verify the result.
```

Expected behavior: use `nb output clear`, then `nb read` to confirm sources remain and outputs are gone.

## Connected Mode

```text
Use $notebook-cli to check whether the current project is connected to JupyterLab, then describe which command prefix should be used for Python commands in the same environment.
```

Expected behavior: use `nb status` and `nb status --python`; do not infer environment from unrelated files.
