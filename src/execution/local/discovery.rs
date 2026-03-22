use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;

/// Find a kernel specification
///
/// Priority:
/// 1. Explicit --kernel flag
/// 2. Notebook metadata kernel
/// 3. Default "python3" kernel
pub fn find_kernel(
    explicit_kernel: Option<&str>,
    notebook_kernel: Option<&str>,
) -> Result<(String, PathBuf)> {
    // Determine which kernel to use
    let kernel_name = if let Some(kernel) = explicit_kernel {
        kernel.to_string()
    } else if let Some(kernel) = notebook_kernel {
        kernel.to_string()
    } else {
        "python3".to_string()
    };

    // Try to find the kernel spec
    match find_kernelspec(&kernel_name) {
        Some(spec_path) => Ok((kernel_name.clone(), spec_path)),
        None => {
            // Kernel not found - provide helpful error message
            let available = list_available_kernels();
            let available_str = if available.is_empty() {
                "No kernels found.".to_string()
            } else {
                format!("Available kernels:\n  {}", available.join("\n  "))
            };

            Err(anyhow!(
                "Kernel '{}' not found.\n\n{}\n\nTo install a kernel, see: https://jupyter.readthedocs.io/en/latest/install-kernel.html",
                kernel_name,
                available_str
            ))
        }
    }
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
fn list_available_kernels() -> Vec<String> {
    let mut names = Vec::new();

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
        .args(&["-c", "import jupyter_core.paths; import json; print(json.dumps(jupyter_core.paths.jupyter_path()))"])
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
        match find_kernel(None, None) {
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
        let result = find_kernel(Some("python3"), Some("julia"));
        if let Ok((name, _)) = result {
            assert_eq!(name, "python3");
        }
    }

    #[test]
    fn test_list_kernels() {
        let kernels = list_available_kernels();
        // Should find at least some kernels (if Jupyter is installed)
        println!("Found {} kernels: {:?}", kernels.len(), kernels);
    }
}
