//! Backend-agnostic notebook loading and mutation.
//!
//! Every cell-mutating command (`add`, `update`, `delete`, `clear`) needs to
//! run against one of three targets depending on how the CLI was invoked:
//! a local `.ipynb` file, a remote Jupyter Server's Contents API, or a
//! remote Jupyter Server's Y.js realtime collaboration session. This module
//! resolves which target applies once ([`resolve_backend`]) and provides a
//! single entry point ([`run_mutation`]) that handles the realtime-with-
//! fallback dance, so commands only need to implement [`CellMutator`] once
//! for the shared nbformat `Notebook` representation and once (only where
//! Y.js semantics genuinely differ) for the realtime path.

use anyhow::Context;
use nbformat::v4::Notebook;

use crate::commands::common::{self, normalize_notebook_path};
use crate::execution::types::ExecutionMode;
use crate::notebook::{local, remote};

/// Where a notebook-mutating command should read/write, resolved once per
/// invocation instead of every command re-deriving it.
pub enum NotebookBackend {
    /// Local file, or a remote kernel gateway (which has no server-side
    /// notebook document at all — the file is still local).
    File { file_path: String },
    /// A Jupyter Server connection. `cached_ydoc_available` is the routing
    /// hint from `common::resolve_ydoc_available`: `Some(false)` skips
    /// straight to the Contents API, anything else tries the realtime path
    /// first and falls back on a definitive backend-absent signal.
    Remote {
        file_path: String,
        server_url: String,
        token: String,
        server_path: String,
        cached_ydoc_available: Option<bool>,
    },
}

/// Resolve which backend a mutating command should use, given its `--server`
/// / `--token` args (or saved connection config).
pub fn resolve_backend(
    file: &str,
    server_arg: Option<String>,
    token_arg: Option<String>,
) -> anyhow::Result<NotebookBackend> {
    let mode = common::resolve_execution_mode(server_arg.clone(), token_arg.clone())?;
    let file_path = normalize_notebook_path(file);

    match mode {
        ExecutionMode::Local | ExecutionMode::RemoteKernel { .. } => {
            Ok(NotebookBackend::File { file_path })
        }
        ExecutionMode::Remote { server_url, token } => {
            let server_root = remote::resolve_server_root();
            let server_path = remote::notebook_path_for_server(&file_path, server_root.as_deref());
            let cached_ydoc_available = common::resolve_ydoc_available(&server_arg, &token_arg);
            Ok(NotebookBackend::Remote {
                file_path,
                server_url,
                token,
                server_path,
                cached_ydoc_available,
            })
        }
    }
}

/// One notebook mutation, expressed against whichever representation a
/// backend uses.
///
/// `mutate_notebook` runs for both the `File` backend and the `Remote`
/// backend when going through the Contents API — both operate on the same
/// in-memory nbformat `Notebook`, so this is implemented exactly once per
/// command. `mutate_realtime` runs only when a Y.js collaborative session
/// is available; it is a separate method (not a generic "apply this mutation
/// to any representation") because Y.js CRDT operations are structurally
/// different from `Vec<Cell>` splicing and cannot be unified without a lossy
/// abstraction.
#[async_trait::async_trait]
pub trait CellMutator {
    type Output;

    fn mutate_notebook(
        &self,
        notebook: &mut Notebook,
        file_path: &str,
    ) -> anyhow::Result<Self::Output>;

    async fn mutate_realtime(
        &self,
        server_url: &str,
        token: &str,
        server_path: &str,
        file_path: &str,
    ) -> anyhow::Result<Self::Output>;
}

/// Run one [`CellMutator`] against whichever backend [`resolve_backend`]
/// picked, returning the (possibly-normalized) file path alongside the
/// mutator's result. This is the single place the realtime-try/fallback-to-
/// Contents-API dance lives; it replaces what was previously ~20 lines
/// duplicated verbatim in every cell-mutating command.
pub async fn run_mutation<M: CellMutator>(
    backend: NotebookBackend,
    mutator: &M,
) -> anyhow::Result<(String, M::Output)> {
    match backend {
        NotebookBackend::File { file_path } => {
            let mut notebook =
                local::read_notebook(&file_path).context("Failed to read notebook")?;
            let result = mutator.mutate_notebook(&mut notebook, &file_path)?;
            local::write_notebook_atomic(&file_path, &notebook)
                .context("Failed to write notebook")?;
            Ok((file_path, result))
        }
        NotebookBackend::Remote {
            file_path,
            server_url,
            token,
            server_path,
            cached_ydoc_available,
        } => {
            if cached_ydoc_available == Some(false) {
                let result = run_via_contents_api(&file_path, &server_url, &token, mutator).await?;
                return Ok((file_path, result));
            }

            match mutator
                .mutate_realtime(&server_url, &token, &server_path, &file_path)
                .await
            {
                Err(e) if crate::execution::server::ydoc::is_yjs_unavailable(&e) => {
                    common::warn_stale_collab_cache(cached_ydoc_available);
                    let result =
                        run_via_contents_api(&file_path, &server_url, &token, mutator).await?;
                    Ok((file_path, result))
                }
                result => result.map(|r| (file_path, r)),
            }
        }
    }
}

async fn run_via_contents_api<M: CellMutator>(
    file_path: &str,
    server_url: &str,
    token: &str,
    mutator: &M,
) -> anyhow::Result<M::Output> {
    let mode = ExecutionMode::Remote {
        server_url: server_url.to_string(),
        token: token.to_string(),
    };
    remote::with_contents_api(file_path, &mode, |notebook, fp| {
        mutator.mutate_notebook(notebook, fp)
    })
    .await
}

/// Load a notebook for reading, regardless of backend. Used by read/search/
/// execute wherever they need "just get me the Notebook".
pub async fn load_notebook(file_path: &str, mode: &ExecutionMode) -> anyhow::Result<Notebook> {
    match mode {
        ExecutionMode::Remote { server_url, token } => {
            let server_root = remote::resolve_server_root();
            let server_path = remote::notebook_path_for_server(file_path, server_root.as_deref());
            remote::read_notebook_remote(server_url, token, &server_path).await
        }
        ExecutionMode::Local | ExecutionMode::RemoteKernel { .. } => {
            local::read_notebook(file_path)
        }
    }
}
