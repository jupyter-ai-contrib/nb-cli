# Editing Notebook Cells

Use `nb cell add`, `nb cell update`, and `nb cell delete` for all notebook mutations.

## Update One Cell

Inspect first, then update by stable ID when possible:

```bash
nb read notebook.ipynb --no-output
nb cell update notebook.ipynb --cell "cell-id" --source -
```

Use indexes for quick positional edits:

```bash
nb cell update notebook.ipynb --cell-index 0 --source "x = 1"
nb cell update notebook.ipynb -i -1 --append "\nprint(x)"
nb cell update notebook.ipynb -i 0 --type markdown
```

At least one of `--source`, `--append`, or `--type` is required. Changing source resets execution count for code cells.

## Add One Cell

```bash
nb cell add notebook.ipynb --source "print('hello')"
nb cell add notebook.ipynb --type markdown --source "# Analysis"
nb cell add notebook.ipynb --source "import pandas as pd" --insert-at 0
nb cell add notebook.ipynb --source "df.head()" --after "cell-id"
nb cell add notebook.ipynb --source "setup()" --before "cell-id"
```

Default cell type is code. Use `--id` only when a caller requires a specific cell ID.

## Add Multiple Cells

Add cells in batches of roughly 3–5, grouped by logical section (e.g., setup, data loading, analysis). Execute and verify each batch before adding the next. See [best-practices.md](best-practices.md#add-cells-in-batches-by-logical-section) for the full workflow.

Start the source with a sentinel line. Multi-cell mode activates only when the first non-empty line is a sentinel, so sentinel-like text inside normal content is preserved.

```bash
nb cell add notebook.ipynb --source -
```

Then send:

```text
@@markdown
# Setup
@@code
import pandas as pd
@@code
df = pd.read_csv("data.csv")
```

Supported sentinels:

- `@@code`
- `@@markdown`
- `@@raw`
- `@@cell {"cell_type":"code","metadata":{"tags":["setup"]}}`

When sentinels are present, `--type` is ignored and `--id` cannot be used. Leading and trailing blank lines are stripped from each new cell.

## Delete Cells

```bash
nb cell delete notebook.ipynb --cell-index 0
nb cell delete notebook.ipynb -i -1
nb cell delete notebook.ipynb --cell "cell-id"
nb cell delete notebook.ipynb --range 0:3
nb cell delete notebook.ipynb -i 0 -i 2 -i 5
```

Ranges use an exclusive end: `0:3` deletes cells 0, 1, and 2.

## Safe Edit Pattern

1. Run `nb read <file> --no-output`.
2. Identify the target cell IDs or indexes.
3. Apply the smallest `nb cell ...` command.
4. Run `nb read <file> --no-output` again to verify placement and source.
5. Execute only the affected cell or range when behavior changed.
