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

    // Try env-specific discovery first if env_config is provided
    if let Some(env_config) = env_config {
        if let Some(spec_path) = find_kernelspec_in_env(&kernel_name, env_config)? {
            return Ok((kernel_name.clone(), spec_path));
        }
        // If not found in env, fall through to global discovery
    }

    // Try to find the kernel spec in global paths
    match find_kernelspec(&kernel_name) {
        Some(spec_path) => Ok((kernel_name.clone(), spec_path)),
        None => {
            // Kernel not found - provide helpful error message
            let available = list_available_kernels(env_config);
            let available_str = if available.is_empty() {
                "No kernels found.".to_string()
            } else {
                format!("Available kernels:\n- {}", available.join("\n- "))
            };

            // Build environment-specific suggestions based on command context
            let env_suggestions = if env_config.is_some() {
                // Already using --uv or --pixi, no additional suggestions needed
                String::new()
            } else {
                match command_context {
                    Some("create") => "For kernels installed in virtual environments:\n\
                         - use `nb create --uv` for uv\n\
                         - use `nb create --pixi` for pixi"
                        .to_string(),
                    Some("execute") => "For kernels installed in virtual environments:\n\
                         - use `nb execute --uv` for uv\n\
                         - use `nb execute --pixi` for pixi"
                        .to_string(),
                    _ => {
                        // No suggestions for other contexts (e.g., tests)
                        String::new()
                    }
                }
            };

            let message = if env_suggestions.is_empty() {
                format!("Kernel '{}' not found.\n\n{}", kernel_name, available_str)
            } else {
                format!(
                    "Kernel '{}' not found.\n\n{}\n\n{}",
                    kernel_name, available_str, env_suggestions
                )
            };
            Err(anyhow!(message))
        }
    }
}

/// Find a kernel specification in an environment-managed context
///
/// Uses `jupyter kernelspec list --json` executed via the environment manager
/// to discover kernels installed in that environment.
fn find_kernelspec_in_env(name: &str, env_config: &EnvConfig) -> Result<Option<PathBuf>> {
    let mut cmd = env_config.build_jupyter_command(&["kernelspec", "list", "--json"]);

    let output = match cmd.output() {
        Ok(output) => output,
        Err(_) => {
            // If jupyter isn't available in the env, fall back to global discovery
            return Ok(None);
        }
    };

    if !output.status.success() {
        // jupyter command failed, fall back to global discovery
        return Ok(None);
    }

    // Parse JSON output
    let json_str = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

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

/// Get Jupyter kernel directories
///
/// Checks in priority order:
/// 1. Virtual environment kernels: $VIRTUAL_ENV/share/jupyter/kernels (if VIRTUAL_ENV is set)
/// 2. User kernels: ~/.local/share/jupyter/kernels (Linux/Mac)
/// 3. System kernels: /usr/local/share/jupyter/kernels, /usr/share/jupyter/kernels
/// 4. Python site-packages kernels
fn get_kernel_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Virtual environment kernels (if VIRTUAL_ENV is set)
    if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
        dirs.push(
            PathBuf::from(venv)
                .join("share")
                .join("jupyter")
                .join("kernels"),
        );
    }

    // User kernels
    if let Some(data_dir) = dirs::data_local_dir() {
        dirs.push(data_dir.join("jupyter").join("kernels"));
    }

    // Home directory (macOS/Linux)
    if let Some(home_dir) = dirs::home_dir() {
        dirs.push(
            home_dir
                .join(".local")
                .join("share")
                .join("jupyter")
                .join("kernels"),
        );
        dirs.push(home_dir.join("Library").join("Jupyter").join("kernels")); // macOS
    }

    // System directories
    dirs.push(PathBuf::from("/usr/local/share/jupyter/kernels"));
    dirs.push(PathBuf::from("/usr/share/jupyter/kernels"));

    // Try to find Python site-packages kernels using jupyter_core paths
    if let Ok(output) = std::process::Command::new("python3")
        .args(["-c", "import jupyter_core.paths; import json; print(json.dumps(jupyter_core.paths.jupyter_path()))"])
        .output()
    {
        if output.status.success() {
            if let Ok(path_str) = String::from_utf8(output.stdout) {
                if let Ok(paths) = serde_json::from_str::<Vec<String>>(&path_str) {
                    for path in paths {
                        let kernel_path = PathBuf::from(path).join("kernels");
                        if kernel_path.exists() {
                            dirs.push(kernel_path);
                        }
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
}
