# Agent Guidelines

## Working with Notebooks (.ipynb files)

When the user asks to read, edit, execute, or work with .ipynb files, use the notebook-cli skill, which provides the `nb` command-line tool. Do not use the built-in Read/Write tools for `.ipynb` files.

## Connect-mode integration tests: backend selection

`tests/integration_connect_mode.rs` exercises connect-mode against whatever
collaboration backend is installed in the active test venv. `jupyter-collaboration`
and `jupyter-server-documents` (JSD) are competing collaborative-editing server
extensions and **must never be installed into the same venv** — each has its own:

- `tests/.test-venv` — JSD + local-mode tests (default). Pinned:
  `jupyter_server==2.20.0`, `jupyter-server-documents==0.2.5`.
- `tests/.test-venv-collab` — jupyter-collaboration. Pinned:
  `jupyter_server==2.20.0`, `jupyter-collaboration==4.4.1`.

Set up a venv with `./tests/setup_test_env.sh [jsd|jupyter-collaboration]`
(defaults to `jsd`). Select which backend a test run targets with
`NB_TEST_BACKEND=<jsd|jupyter-collaboration>` (read by `test_helpers::test_backend()`);
this also picks the matching venv directory automatically. Run with:

```
NB_TEST_BACKEND=jupyter-collaboration cargo test --test integration_connect_mode -- --test-threads=1
```

The spawned shared Jupyter server runs in its own process group (`current_dir`
set to its tempdir root, so backend-specific artifacts like jupyter-collaboration's
`.jupyter_ystore.db` land there instead of the crate root) and is killed as a
group on teardown, so an interrupted run doesn't leak orphaned kernels. Each
notebook-executing test also explicitly deletes its Jupyter session/kernel via
`DELETE /api/sessions/{id}` when its `NotebookSession` guard drops (production
code intentionally never deletes sessions, so tests must do this themselves).

**Known state (2026-07-05):** against `jupyter-collaboration`, the 4
execute/restart tests (what PR #99 / issue #92 fixed — FileID fallback, `sessionId`
on the Y.js room WS handshake, v1 kernel-WS subprotocol, client-side output
writing) pass 10/10 runs with zero flakiness. `test_clear_outputs_in_connect_mode`
and `test_clear_outputs_specific_cell_in_connect_mode` fail consistently
(10/10) against jupyter-collaboration — but this is **not** a jupyter-collaboration-specific
or PR #99-scoped bug: the same two tests also fail against JSD on this codebase,
so it predates and is unrelated to PR #99 (which only touches
`src/execution/remote/{mod,ydoc}.rs`, the execute path). Root cause looks like
`output clear`'s Y.js-doc edits not round-tripping back to the persisted
notebook for either backend. Not yet tracked as an issue upstream; out of scope
for PR #99 verification, same category as the already-known JSD bugs #87/#90/#97.
