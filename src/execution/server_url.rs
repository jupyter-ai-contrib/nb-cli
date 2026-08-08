//! Shared Jupyter server URL helpers.
//!
//! Every component that talks to a Jupyter server or kernel gateway
//! (`JupyterClient`, `YDocClient`, `RemoteExecutor`, `RemoteKernelExecutor`,
//! externalized-output fetches) builds its endpoint URLs here so the server's
//! `base_url` path prefix (e.g. `/jupyter`) is always preserved — rebuilding
//! from host/port or replacing the path drops it (commit afd0a47).
//!
//! Callers keep the server URL as a plain `String` (as it appears in
//! `ExecutionMode`, config and commands) and pass it to the builders at each
//! use; all builders share a single core ([`append_parts`]).

use anyhow::{bail, Context, Result};
use url::Url;

/// Validate a Jupyter server URL and return its canonical form with any
/// trailing slash stripped.
///
/// Only `http`/`https` are accepted — WebSocket URLs are derived by swapping
/// the scheme, so the server URL must have an HTTP scheme, and a URL without
/// any scheme (e.g. `host:8888`) is a hard error instead of being silently
/// passed through. Call this at construction boundaries (client/executor
/// constructors) to fail fast on invalid URLs; the endpoint builders below
/// validate again on first use.
pub fn normalize(server_url: &str) -> Result<String> {
    let url = Url::parse(server_url.trim_end_matches('/')).context("Invalid server URL")?;
    match url.scheme() {
        "http" | "https" => {}
        s => bail!(
            "Invalid server URL '{}': scheme must be http or https, got '{}'",
            server_url,
            s
        ),
    }
    if url.host_str().is_none() {
        bail!("Invalid server URL '{}': missing host", server_url);
    }
    Ok(url.to_string())
}

/// Core builder shared by all endpoint helpers: parse `server_url`, then
/// append `parts` as path segments, preserving any `base_url` prefix.
fn append_parts<'a>(server_url: &str, parts: impl Iterator<Item = &'a str>) -> Result<Url> {
    let mut url = Url::parse(server_url).context("Invalid server URL")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Server URL cannot be a base"))?;
        for part in parts {
            segments.push(part);
        }
    }
    Ok(url)
}

/// Build an endpoint URL by appending route `segments` to the server URL's
/// existing path, keeping any `base_url` prefix. Returns a `Url` ready to
/// pass to reqwest or to append query parameters to.
pub fn endpoint(server_url: &str, segments: &[&str]) -> Result<Url> {
    append_parts(server_url, segments.iter().copied())
}

/// Build an endpoint URL for a resource addressed by a server path: append
/// the `api` route segments, then append `path`'s components. A leading `/`
/// on `path` is trimmed; a `?query` suffix is preserved as a query string.
///
/// `path` is a path-like string of raw, unescaped components (a notebook
/// path, a server-relative URL) — each is percent-encoded as one segment, so
/// a filename with spaces or `#` stays valid. Pass raw names, not
/// already-escaped ones (those would be double-escaped).
///
/// A trailing `/` is preserved: an empty `path` yields a trailing slash,
/// which the collaboration-session route needs for its Tornado regex
/// `session/(.*)`.
///
/// With `api = &[]` this also resolves server-relative URLs (externalized
/// output URLs read from Y.js `metadata.url`).
pub fn endpoint_with_path(server_url: &str, api: &[&str], path: &str) -> Result<Url> {
    let (path_part, query_part) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    // Split into raw components so `path_segments_mut` percent-encodes each
    // one; trim the leading `/` so it doesn't become an empty first segment
    // (which would serialize as `//`).
    let mut url = append_parts(
        server_url,
        api.iter()
            .copied()
            .chain(path_part.trim_start_matches('/').split('/')),
    )?;
    url.set_query(query_part);
    Ok(url)
}

/// Build a WebSocket endpoint URL: like [`endpoint`], but with
/// `http`→`ws` / `https`→`wss` swapped in place. Path, port and `base_url`
/// prefix are preserved; an `http` substring inside a path segment is never
/// rewritten.
pub fn ws_endpoint(server_url: &str, segments: &[&str]) -> Result<Url> {
    let mut url = endpoint(server_url, segments)?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        s => bail!("Cannot build WebSocket URL from scheme '{}'", s),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("Failed to set WebSocket scheme"))?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_plain_root_url() {
        assert_eq!(
            normalize("http://127.0.0.1:8888").unwrap(),
            "http://127.0.0.1:8888/"
        );
    }

    #[test]
    fn normalize_keeps_base_url_path_prefix() {
        assert_eq!(
            normalize("https://host/jupyter").unwrap(),
            "https://host/jupyter"
        );
    }

    #[test]
    fn normalize_strips_trailing_slash_from_path() {
        assert_eq!(
            normalize("http://host:8888/foo/bar/").unwrap(),
            "http://host:8888/foo/bar"
        );
    }

    #[test]
    fn normalize_rejects_non_http_scheme() {
        let err = normalize("ftp://host").unwrap_err();
        assert!(err.to_string().contains("http or https"), "got: {err}");
    }

    #[test]
    fn normalize_rejects_scheme_less_url() {
        // "host:8888" parses as scheme "host" + opaque "8888"; the old
        // gateway code silently passed it through, reqwest then rejected it
        // later with a worse error.
        let err = normalize("host:8888").unwrap_err();
        assert!(err.to_string().contains("http or https"), "got: {err}");
    }

    #[test]
    fn normalize_rejects_garbage() {
        assert!(normalize("not a url").is_err());
        assert!(normalize("http://").is_err());
    }

    #[test]
    fn endpoint_appends_segments_onto_base_url_prefix() {
        let url = endpoint(
            "https://my-jupyter.example.com/jupyter",
            &["api", "kernels", "kid1"],
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "https://my-jupyter.example.com/jupyter/api/kernels/kid1"
        );
    }

    #[test]
    fn endpoint_on_root_url() {
        let url = endpoint("http://host:8888", &["api", "sessions"]).unwrap();
        assert_eq!(url.to_string(), "http://host:8888/api/sessions");
    }

    #[test]
    fn endpoint_with_path_appends_path_components() {
        let url = endpoint_with_path(
            "http://host:8888/jupyter",
            &["api", "contents"],
            "dir/nb.ipynb",
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "http://host:8888/jupyter/api/contents/dir/nb.ipynb"
        );
    }

    #[test]
    fn endpoint_with_path_escapes_components_as_raw_path_parts() {
        // `path` is a path-like string, not a URL: a filename with spaces or
        // `#` must be escaped as a single valid segment.
        let url = endpoint_with_path(
            "http://host:8888",
            &["api", "contents"],
            "/my dir/nb#1.ipynb",
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "http://host:8888/api/contents/my%20dir/nb%231.ipynb"
        );
    }

    #[test]
    fn endpoint_with_path_empty_path_yields_trailing_slash() {
        // The collaboration-session route is matched by the Tornado regex
        // `session/(.*)`; the connect-time probe uses an empty path, so the
        // empty path must produce the trailing slash itself.
        let url = endpoint_with_path(
            "http://host:8888/jupyter",
            &["api", "collaboration", "session"],
            "",
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "http://host:8888/jupyter/api/collaboration/session/"
        );
    }

    #[test]
    fn endpoint_with_path_resolves_relative_url() {
        // server-relative URLs (externalized outputs) with no api prefix
        let url = endpoint_with_path(
            "http://host:8888/jupyter",
            &[],
            "/api/contents/file-1/outputs/0",
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "http://host:8888/jupyter/api/contents/file-1/outputs/0"
        );
    }

    #[test]
    fn endpoint_with_path_keeps_query_string() {
        let url = endpoint_with_path(
            "http://host:8888/jupyter",
            &[],
            "/api/contents/file-1?format=json",
        )
        .unwrap();
        assert_eq!(
            url.to_string(),
            "http://host:8888/jupyter/api/contents/file-1?format=json"
        );
    }

    #[test]
    fn ws_endpoint_swaps_scheme_keeping_port_and_prefix() {
        let ws = ws_endpoint(
            "http://host:8888/foo/bar",
            &["api", "kernels", "kid1", "channels"],
        )
        .unwrap();
        assert_eq!(
            ws.to_string(),
            "ws://host:8888/foo/bar/api/kernels/kid1/channels"
        );
    }

    #[test]
    fn ws_endpoint_swaps_https_to_wss() {
        let ws = ws_endpoint(
            "https://my-jupyter.example.com/jupyter",
            &["api", "collaboration", "room", "json:notebook:file-1"],
        )
        .unwrap();
        assert_eq!(
            ws.to_string(),
            "wss://my-jupyter.example.com/jupyter/api/collaboration/room/json:notebook:file-1"
        );
    }

    #[test]
    fn ws_endpoint_does_not_rewrite_http_inside_path() {
        // Only the scheme is swapped; "http" inside a path segment must stay.
        let ws = ws_endpoint(
            "https://gw.example.com/httpapi",
            &["api", "kernels", "abc", "channels"],
        )
        .unwrap();
        assert_eq!(
            ws.to_string(),
            "wss://gw.example.com/httpapi/api/kernels/abc/channels"
        );
    }
}
