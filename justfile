# justfile for nb-cli development

# Install skill to local agent directories for development
install-skill:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Installing notebook-cli skill to project agent directories..."

    # Install to project .claude directory
    mkdir -p .claude/skills/notebook-cli
    cp -v skills/notebook-cli/SKILL.md .claude/skills/notebook-cli/
    echo "✓ Installed to .claude/skills/notebook-cli"

    # Install to project .agents directory
    mkdir -p .agents/skills/notebook-cli
    cp -v skills/notebook-cli/SKILL.md .agents/skills/notebook-cli/
    echo "✓ Installed to .agents/skills/notebook-cli"

    echo ""
    echo "Skill installation complete!"
    echo "The skill is now available in this project's agent directories."

# Build the release binary
build:
    cargo build --release

# Run tests
test:
    just test-all

# Run non-connect tests, then connect-mode against every backend.
test-all:
    cargo test --bins
    cargo test --test integration_connect_config
    cargo test --test integration_env_kernels
    cargo test --test integration_execution
    cargo test --test integration_local_mode
    NB_TEST_BACKEND=jsd cargo test --test integration_connect_mode -- --test-threads=1
    NB_TEST_BACKEND=jupyter-collaboration cargo test --test integration_connect_mode -- --test-threads=1
    NB_TEST_BACKEND=none cargo test --test integration_connect_mode -- --test-threads=1

# Run one connect-mode backend. Usage: just test-connect jsd
test-connect backend="jsd":
    NB_TEST_BACKEND={{backend}} cargo test --test integration_connect_mode -- --test-threads=1

# Format code
fmt:
    cargo fmt

# Run clippy linter
lint:
    cargo clippy

# Clean build artifacts
clean:
    cargo clean

# Show help
help:
    @just --list
