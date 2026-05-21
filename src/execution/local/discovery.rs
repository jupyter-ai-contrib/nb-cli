use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

use crate::commands::env_manager::EnvConfig;

/// Find a kernel specification
///
/// Priority:
/// 1. Explicit --kernel flag
/// 2. Notebook metadata kernel
/// 3. Default "python3" kernel
///
/// If env_config is provided, searches environment-specific kernels first,
/// then falls back to global kernel discovery.
///
/// The `command_context` parameter is used to provide helpful error messages
/// specific to the calling command (e.g., "create" or "execute").
pub fn find_kernel(
    explicit_kernel: Option<&str>,
    notebook_kernel: Option<&str>,
    env_config: Option<&EnvConfig>,
    command_context: Option<&str>,
) -> Result<(String, PathBuf)> {
    // Determine which kernel to use
    let kernel_name = if let Some(kernel) = explicit_kernel {
        kernel.to_string()
    } else if let Some(kernel) = notebook_kernel {
        kernel.to_string()
    } else {
        "python3".to_string()
    };

    // Try env-specific discovery if env_config is provided
    if let Some(env_config) = env_config {
        let manager_name = env_config.manager.as_str();
        match find_kernelspec_in_env(&kernel_name, env_config) {
            Ok(Some(spec_path)) => return Ok((kernel_name.clone(), spec_path)),
            Ok(None) => {
                // Kernel not found in env - fail explicitly with helpful message
                let available = list_kernels_in_env(env_config).unwrap_or_default();
                let available_str = if available.is_empty() {
                    format!("No kernels were found in the {} environment.", manager_name)
                } else {
                    format!(
                        "Available kernels in the {} environment:\n- {}",
                        manager_name,
                        available.join("\n- ")
                    )
                };

                let cmd_name = command_context.unwrap_or("run");
                return Err(anyhow!(
                    "Could not discover kernel '{}' via `{} run jupyter kernelspec list --json`.\n\n\
                     {}\n\n\
                     This usually means Jupyter/kernels are not installed in the {} environment \
                     for this project.\n\n\
                     Next steps:\n\
                     - install Jupyter/ipykernel in the {} environment, or\n\
                     - rerun without --{} if you want to use globally available kernels:\n\
                     \x20 nb {} --kernel {}",
                    kernel_name,
                    manager_name,
                    available_str,
                    manager_name,
                    manager_name,
                    manager_name,
                    cmd_name,
                    kernel_name,
                ));
            }
            Err(e) => {
                // Discovery command itself failed - surface the error
                let cmd_name = command_context.unwrap_or("run");
                return Err(anyhow!(
                    "Failed to discover kernels via `{} run jupyter kernelspec list --json`:\n\
                     {}\n\n\
                     This usually means Jupyter is not installed in the {} environment \
                     for this project.\n\n\
                     Next steps:\n\
                     - install Jupyter/ipykernel in the {} environment, or\n\
                     - rerun without --{} if you want to use globally available kernels:\n\
                     \x20 nb {} --kernel {}",
                    manager_name,
                    e,
                    manager_name,
                    manager_name,
                    manager_name,
                    cmd_name,
                    kernel_name,
                ));
            }
        }
    }

    // No env manager - use global kernel discovery
    match find_kernelspec(&kernel_name) {
        Some(spec_path) => Ok((kernel_name.clone(), spec_path)),
        None => {
            // Kernel not found - provide helpful error message
            let available = list_available_kernels(None);
            let available_str = if available.is_empty() {
                "No kernels found.".to_string()
            } else {
                format!("Available kernels:\n- {}", available.join("\n- "))
            };

            let env_suggestions = match command_context {
                Some("create") => {
                    "\n\nFor kernels installed in virtual environments:\n\
                     - use `nb create --uv` for uv\n\
                     - use `nb create --pixi` for pixi"
                }
                Some("execute") => {
                    "\n\nFor kernels installed in virtual environments:\n\
                     - use `nb execute --uv` for uv\n\
                     - use `nb execute --pixi` for pixi"
                }
                _ => "",
            };

            Err(anyhow!(
                "Kernel '{}' not found.\n\n{}{}",
                kernel_name,
                available_str,
                env_suggestions,
            ))
        }
    }
}

/// Find a kernel specification in an environment-managed context
///
/// Uses `jupyter kernelspec list --json` executed via the environment manager
/// to discover kernels installed in that environment.
///
/// Returns:
/// - `Ok(Some(path))` if the kernel was found
/// - `Ok(None)` if the command succeeded but the kernel wasn't listed
/// - `Err(...)` if the discovery command itself failed
fn find_kernelspec_in_env(name: &str, env_config: &EnvConfig) -> Result<Option<PathBuf>> {
    let manager_name = env_config.manager.as_str();
    let mut cmd = env_config.build_jupyter_command(&["kernelspec", "list", "--json"]);

    let output = cmd
        .output()
        .map_err(|e| anyhow!("`{} run jupyter` is not available: {}", manager_name, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "`{} run jupyter kernelspec list --json` exited with {}{}",
            manager_name,
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stderr.trim())
            },
        ));
    }

    // Parse JSON output
    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
        anyhow!(
            "Failed to parse output from `{} run jupyter kernelspec list --json`: {}",
            manager_name,
            e
        )
    })?;

    // Extract kernelspecs
    if let Some(kernelspecs) = json.get("kernelspecs").and_then(|v| v.as_object()) {
        if let Some(spec) = kernelspecs.get(name) {
            if let Some(resource_dir) = spec.get("resource_dir").and_then(|v| v.as_str()) {
                let path = PathBuf::from(resource_dir);
                if path.exists() && path.is_dir() {
                    return Ok(Some(path));
                }
            }
        }
    }

    Ok(None)
}

/// Find a kernel specification by name
fn find_kernelspec(name: &str) -> Option<PathBuf> {
    for dir in get_kernel_dirs() {
        let kernel_path = dir.join(name);
        if kernel_path.exists() && kernel_path.is_dir() {
            return Some(kernel_path);
        }
    }
    None
}

/// List all available kernel names
fn list_available_kernels(env_config: Option<&EnvConfig>) -> Vec<String> {
    let mut names = Vec::new();

    // Try env-specific listing first if env_config is provided
    if let Some(env_config) = env_config {
        if let Ok(env_kernels) = list_kernels_in_env(env_config) {
            names.extend(env_kernels);
        }
    }

    // Add global kernels
    for dir in get_kernel_dirs() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !names.contains(&name.to_string()) {
                            names.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    names.sort();
    names
}

/// List kernels available in an environment
fn list_kernels_in_env(env_config: &EnvConfig) -> Result<Vec<String>> {
    let mut cmd = env_config.build_jupyter_command(&["kernelspec", "list", "--json"]);

    let output = cmd.output()?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&json_str)?;

    let mut names = Vec::new();
    if let Some(kernelspecs) = json.get("kernelspecs").and_then(|v| v.as_object()) {
        for (name, _) in kernelspecs {
            names.push(name.clone());
        }
    }

    Ok(names)
}

/// Get Jupyter kernel directories in priority order.
///
/// Priority:
/// 1. Virtual environment: `$VIRTUAL_ENV/share/jupyter/kernels`
/// 2. User data directory (platform-specific):
///    - Windows: `%APPDATA%\jupyter\kernels` (Roaming)
///    - macOS:   `~/Library/Application Support/jupyter/kernels`
///    - Linux:   `~/.local/share/jupyter/kernels`
/// 3. Non-Windows home-directory paths:
///    - Linux:         `~/.local/share/jupyter/kernels`
///    - macOS (legacy): `~/Library/Jupyter/kernels`
/// 4. Windows-only explicit paths:
///    - `%APPDATA%\Jupyter\kernels` (title-case fallback for Jupyter-conventional casing)
///    - `%PROGRAMDATA%\Jupyter\kernels` (system-wide, equivalent to Unix /usr/local/share)
/// 5. Unix system paths: `/usr/local/share/jupyter/kernels`, `/usr/share/jupyter/kernels`
/// 6. Python `jupyter_core.paths.jupyter_path()` — covers Anaconda, custom prefixes, etc.
///    Tries `python3` then `python` on Unix; `python` then `python3` on Windows.
fn get_kernel_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Virtual environment kernels (cross-platform: works on all OSes)
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        dirs.push(
            PathBuf::from(venv)
                .join("share")
                .join("jupyter")
                .join("kernels"),
        );
    }

    // 2. User data directory via the `dirs` crate (platform-aware):
    //    - Windows: %APPDATA% (Roaming) — where Jupyter installs user kernels
    //    - macOS/Linux: same result as data_local_dir()
    //    Note: data_local_dir() returns %LOCALAPPDATA% on Windows, which is wrong for Jupyter.
    if let Some(data_dir) = dirs::data_dir() {
        dirs.push(data_dir.join("jupyter").join("kernels"));
    }

    // 3. Non-Windows: explicit XDG and macOS legacy paths
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home_dir) = dirs::home_dir() {
            // Linux XDG user data path
            dirs.push(
                home_dir
                    .join(".local")
                    .join("share")
                    .join("jupyter")
                    .join("kernels"),
            );
            // macOS legacy path (Jupyter 4.x style, distinct from "Application Support")
            dirs.push(home_dir.join("Library").join("Jupyter").join("kernels"));
        }
    }

    // 4. Windows-only: explicit %APPDATA% and %PROGRAMDATA% paths
    //    %APPDATA% with title-case "Jupyter" is the conventional casing used by Jupyter on Windows.
    //    %PROGRAMDATA% is the system-wide equivalent of /usr/local/share on Unix.
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = PathBuf::from(&appdata).join("Jupyter").join("kernels");
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            dirs.push(PathBuf::from(programdata).join("Jupyter").join("kernels"));
        }
    }

    // 5. Unix system-wide paths (not applicable on Windows)
    #[cfg(not(target_os = "windows"))]
    {
        dirs.push(PathBuf::from("/usr/local/share/jupyter/kernels"));
        dirs.push(PathBuf::from("/usr/share/jupyter/kernels"));
    }

    // 6. Python jupyter_core paths — catches Anaconda, custom prefixes, and any install
    //    that doesn't follow the standard directory layout above.
    //    On Windows the Python executable is "python", not "python3"; try the
    //    platform-preferred name first and fall back to the other.
    let python_candidates: &[&str] = if cfg!(target_os = "windows") {
        &["python", "python3"]
    } else {
        &["python3", "python"]
    };

    'python_discovery: for python_cmd in python_candidates {
        if let Ok(output) = std::process::Command::new(python_cmd)
            .args([
                "-c",
                "import jupyter_core.paths; import json; \
                 print(json.dumps(jupyter_core.paths.jupyter_path()))",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(path_str) = String::from_utf8(output.stdout) {
                    // .trim() guards against a trailing newline confusing serde_json
                    if let Ok(paths) = serde_json::from_str::<Vec<String>>(path_str.trim()) {
                        for path in paths {
                            let kernel_path = PathBuf::from(path).join("kernels");
                            if !dirs.contains(&kernel_path) && kernel_path.exists() {
                                dirs.push(kernel_path);
                            }
                        }
                        // Don't try the second candidate if the first succeeded
                        break 'python_discovery;
                    }
                }
            }
        }
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_default_kernel() {
        // Should find python3 kernel (or fail gracefully)
        match find_kernel(None, None, None, None) {
            Ok((name, _path)) => {
                assert_eq!(name, "python3");
            }
            Err(e) => {
                // It's okay if python3 isn't installed in test environment
                println!("Note: python3 kernel not found: {}", e);
            }
        }
    }

    #[test]
    fn test_explicit_kernel_priority() {
        // Explicit kernel should take priority
        let result = find_kernel(Some("python3"), Some("julia"), None, None);
        if let Ok((name, _)) = result {
            assert_eq!(name, "python3");
        }
    }

    #[test]
    fn test_list_kernels() {
        let kernels = list_available_kernels(None);
        // Should find at least some kernels (if Jupyter is installed)
        println!("Found {} kernels: {:?}", kernels.len(), kernels);
    }

    #[test]
    fn test_get_kernel_dirs_is_nonempty() {
        // Should always return at least some candidate directories even without Jupyter
        let dirs = get_kernel_dirs();
        assert!(
            !dirs.is_empty(),
            "get_kernel_dirs() should never return an empty list"
        );
    }

    #[test]
    fn test_get_kernel_dirs_virtual_env() {
        // When VIRTUAL_ENV is set, its kernels path should appear first.
        // Use a platform-neutral PathBuf comparison so path separators don't matter.
        let venv = std::env::temp_dir().join("test-venv");
        std::env::set_var("VIRTUAL_ENV", &venv);
        let dirs = get_kernel_dirs();
        std::env::remove_var("VIRTUAL_ENV");

        let expected = venv.join("share").join("jupyter").join("kernels");
        let first = dirs.first().expect("dirs should be non-empty");
        assert_eq!(
            first, &expected,
            "VIRTUAL_ENV kernels should be the first entry"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_get_kernel_dirs_contains_unix_system_paths() {
        let dirs = get_kernel_dirs();
        let paths: Vec<_> = dirs
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        assert!(
            paths.iter().any(|p| p.contains("/usr/local/share/jupyter")),
            "Expected /usr/local/share/jupyter/kernels in dirs; got: {:?}",
            paths
        );
        assert!(
            paths.iter().any(|p| p.contains("/usr/share/jupyter")),
            "Expected /usr/share/jupyter/kernels in dirs; got: {:?}",
            paths
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn test_get_kernel_dirs_no_windows_paths_on_unix() {
        let dirs = get_kernel_dirs();
        let paths: Vec<_> = dirs
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        assert!(
            !paths
                .iter()
                .any(|p| p.contains("AppData") || p.contains("ProgramData")),
            "Should not contain Windows AppData/ProgramData paths on Unix; got: {:?}",
            paths
        );
    }

    /// On Windows: verify that %APPDATA% and %PROGRAMDATA% kernel paths are included
    /// and that Unix /usr paths are absent.
    #[test]
    #[cfg(target_os = "windows")]
    fn test_get_kernel_dirs_windows_paths() {
        let dirs = get_kernel_dirs();
        let paths: Vec<_> = dirs
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        // Should not contain hardcoded Unix system paths
        assert!(
            !paths.iter().any(|p| p.starts_with("/usr")),
            "Should not contain /usr paths on Windows; got: {:?}",
            paths
        );

        // %APPDATA% or %PROGRAMDATA% should appear (they are always set on Windows)
        assert!(
            paths
                .iter()
                .any(|p| p.contains("AppData") || p.contains("ProgramData")),
            "Should contain AppData or ProgramData Jupyter path on Windows; got: {:?}",
            paths
        );
    }
}
