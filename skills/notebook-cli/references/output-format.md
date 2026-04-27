# Output Format and Output Files

`nb read` defaults to AI-Optimized Markdown. This is the normal format for agent work.

## Sentinels

```text
@@notebook {"format":"ai-notebook","metadata":{...}}
@@cell {"index":0,"id":"cell-id","cell_type":"code","execution_count":1}
@@output {"output_type":"stream","name":"stdout"}
```

Treat sentinel lines as structured records. Parse the JSON object after the sentinel name when cell IDs, indexes, cell types, MIME types, execution counts, or externalized paths matter.

## Cell Content

- Code cells are fenced with a language hint.
- Markdown cells are raw markdown.
- Outputs are fenced or externalized depending on size and MIME type.

## Externalized Outputs

Large outputs are written to files and referenced in `@@output` JSON metadata.

```bash
nb read notebook.ipynb --limit 8000
nb read notebook.ipynb --output-dir ./notebook-outputs
```

Externalized filenames are content-hashed with SHA256 and paths are absolute. Do not guess filenames; read the `path` field from the output sentinel.

## Clearing Outputs

```bash
nb output clear notebook.ipynb
nb output clear notebook.ipynb --cell-index 0
nb output clear notebook.ipynb --cell "cell-id"
nb output clear notebook.ipynb --keep-execution-count
```

Clear outputs before committing notebooks when outputs are not part of the requested change.

## Cleaning Externalized Output Directories

```bash
nb output clean
```

Use this to remove externalized output files from the temp directory after large-output inspection.

## Commit Hygiene

- Use `nb read --no-output` for source-only review.
- Use `nb output clear` when output churn is unrelated.
- Keep generated externalized output directories out of commits unless they are explicitly requested artifacts.
