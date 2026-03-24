use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment manager types for running Jupyter commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvManager {
    /// Run jupyter directly (default)
    Direct,
    /// Run jupyter via `uv run`
    Uv,
    /// Run jupyter via `pixi run`
    Pixi,
}

/// Configuration for environment-aware command execution
#[derive(Debug, Clone)]
pub struct EnvConfig {
    pub manager: EnvManager,
    pub project_root: Option<PathBuf>,
}

impl EnvConfig {
    /// Create environment configuration from CLI flags
    pub fn from_flags(uv: bool, pixi: bool) -> Result<Self> {
        let manager = match (uv, pixi) {
            (true, false) => EnvManager::Uv,
            (false, true) => EnvManager::Pixi,
            (false, false) => EnvManager::Direct,
            (true, true) => {
                // This should be prevented by clap's conflicts_with, but just in case
                bail!("Cannot use both --uv and --pixi flags");
            }
        };

        let project_root = match manager {
            EnvManager::Direct => None,
            EnvManager::Uv => Some(find_uv_project_root()?),
            EnvManager::Pixi => Some(find_pixi_project_root()?),
        };

        Ok(EnvConfig {
            manager,
            project_root,
        })
    }

    /// Build a Command for running jupyter with the appropriate environment manager
    pub fn build_jupyter_command(&self, args: &[&str]) -> Command {
        match self.manager {
            EnvManager::Direct => {
                let mut cmd = Command::new("jupyter");
                cmd.args(args);
                cmd
            }
            EnvManager::Uv => {
                let mut cmd = Command::new("uv");
                cmd.arg("run");
                cmd.arg("jupyter");
                cmd.args(args);
                if let Some(root) = &self.project_root {
                    cmd.current_dir(root);
                }
                cmd
            }
            EnvManager::Pixi => {
                let mut cmd = Command::new("pixi");
                cmd.arg("run");
                cmd.arg("jupyter");
                cmd.args(args);
                if let Some(root) = &self.project_root {
                    cmd.current_dir(root);
                }
                cmd
            }
        }
    }
}

/// Find the root directory of a uv project
///
/// Searches upward from the current directory for a directory containing
/// `pyproject.toml`, `uv.toml`, or `uv.lock`
fn find_uv_project_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    find_project_root(&current_dir, &["pyproject.toml", "uv.toml", "uv.lock"]).with_context(|| {
        format!(
            "No uv project found.\n\
            \n\
            The --uv flag requires a uv project (pyproject.toml, uv.toml, or uv.lock) in the\n\
            current directory or any parent directory.\n\
            \n\
            Current directory: {}\n\
            \n\
            To use uv:\n\
              1. Initialize a uv project: uv init\n\
              2. Or navigate to a directory with a uv project\n\
              3. Or omit the --uv flag to use jupyter directly",
            current_dir.display()
        )
    })
}

/// Find the root directory of a pixi project
///
/// Searches upward from the current directory for a directory containing
/// `pyproject.toml`, `pixi.toml`, or `pixi.lock`
fn find_pixi_project_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    find_project_root(&current_dir, &["pyproject.toml", "pixi.toml", "pixi.lock"]).with_context(|| {
        format!(
            "No pixi project found.\n\
            \n\
            The --pixi flag requires a pixi project (pyproject.toml, pixi.toml, or pixi.lock) in the\n\
            current directory or any parent directory.\n\
            \n\
            Current directory: {}\n\
            \n\
            To use pixi:\n\
              1. Initialize a pixi project: pixi init\n\
              2. Or navigate to a directory with a pixi project\n\
              3. Or omit the --pixi flag to use jupyter directly",
            current_dir.display()
        )
    })
}

/// Find the root directory containing one of the marker files
///
/// Searches upward from the given starting directory until one of the marker files is found
fn find_project_root(start_dir: &Path, marker_files: &[&str]) -> Result<PathBuf> {
    let mut path = start_dir;

    loop {
        // Check if any marker file exists in this directory
        for marker in marker_files {
            let marker_path = path.join(marker);
            if marker_path.exists() {
                return Ok(path.to_path_buf());
            }
        }

        // Move up to parent directory
        match path.parent() {
            Some(parent) => path = parent,
            None => bail!("Project root not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_env_config_from_flags() {
        // Test default (no flags)
        let config = EnvConfig::from_flags(false, false).unwrap();
        assert_eq!(config.manager, EnvManager::Direct);
        assert!(config.project_root.is_none());
    }

    #[test]
    fn test_build_command_direct() {
        let config = EnvConfig {
            manager: EnvManager::Direct,
            project_root: None,
        };

        let cmd = config.build_jupyter_command(&["server", "list", "--json"]);
        let program = cmd.get_program().to_string_lossy();
        assert_eq!(program, "jupyter");
    }

    #[test]
    fn test_build_command_uv() {
        let config = EnvConfig {
            manager: EnvManager::Uv,
            project_root: Some(PathBuf::from("/tmp/project")),
        };

        let cmd = config.build_jupyter_command(&["server", "list", "--json"]);
        let program = cmd.get_program().to_string_lossy();
        assert_eq!(program, "uv");
    }

    #[test]
    fn test_build_command_pixi() {
        let config = EnvConfig {
            manager: EnvManager::Pixi,
            project_root: Some(PathBuf::from("/tmp/project")),
        };

        let cmd = config.build_jupyter_command(&["server", "list", "--json"]);
        let program = cmd.get_program().to_string_lossy();
        assert_eq!(program, "pixi");
    }

    #[test]
    fn test_find_project_root_direct_match() {
        let temp_dir = TempDir::new().unwrap();
        let marker_path = temp_dir.path().join("pyproject.toml");
        fs::write(&marker_path, "").unwrap();

        let result = find_project_root(temp_dir.path(), &["pyproject.toml"]);

        assert!(result.is_ok());
        // Canonicalize both paths to handle symlinks (e.g., /var vs /private/var on macOS)
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            temp_dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_project_root_parent_match() {
        let temp_dir = TempDir::new().unwrap();
        let marker_path = temp_dir.path().join("uv.lock");
        fs::write(&marker_path, "").unwrap();

        // Create subdirectory and search from there
        let sub_dir = temp_dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();

        let result = find_project_root(&sub_dir, &["uv.lock"]);

        assert!(result.is_ok());
        // Canonicalize both paths to handle symlinks (e.g., /var vs /private/var on macOS)
        assert_eq!(
            result.unwrap().canonicalize().unwrap(),
            temp_dir.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn test_find_project_root_no_match() {
        let temp_dir = TempDir::new().unwrap();

        let result = find_project_root(temp_dir.path(), &["nonexistent.toml"]);

        assert!(result.is_err());
    }
}
