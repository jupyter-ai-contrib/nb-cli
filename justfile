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

# Install Python test dependencies into tests/.test-venv (run once)
setup:
    ./tests/setup_test_env.sh

# Run the full test suite including connect-mode integration tests.
# Starts a Jupyter server before nextest so the server is ready before
# any test process spawns (avoids the port-contention race).
# Run `just setup` first if tests/.test-venv doesn't exist.
test:
    #!/usr/bin/env bash
    set -euo pipefail
    VENV="$(pwd)/tests/.test-venv"
    if [ ! -f "$VENV/bin/jupyter" ]; then
        echo "❌  Test venv not found. Run: just setup"
        exit 1
    fi
    SERVER_ROOT=$(mktemp -d)
    PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('',0)); p=s.getsockname()[1]; s.close(); print(p)")
    PATH="$VENV/bin:$PATH" VIRTUAL_ENV="$VENV" \
        jupyter server \
            --no-browser \
            --ServerApp.token=nbtest123 \
            --ServerApp.root_dir="$SERVER_ROOT" \
            --port="$PORT" \
            --ServerApp.open_browser=False \
            > /dev/null 2>&1 &
    SERVER_PID=$!
    trap "kill $SERVER_PID 2>/dev/null; rm -rf '$SERVER_ROOT' 2>/dev/null || true" EXIT
    echo "⏳  Waiting for Jupyter server on port $PORT..."
    until curl -sf "http://127.0.0.1:$PORT/api?token=nbtest123" > /dev/null 2>&1; do sleep 0.2; done
    echo "✅  Server ready."
    NB_TEST_SERVER_URL="http://127.0.0.1:$PORT" \
    NB_TEST_SERVER_TOKEN="nbtest123" \
    NB_TEST_SERVER_ROOT="$SERVER_ROOT" \
        cargo nextest run

# Setup then run the full test suite in one shot
test-all: setup test

# Run tests with CI profile (no retries, JUnit XML output)
test-ci:
    cargo nextest run --profile ci

# Format code
fmt:
    cargo fmt

# Check formatting and run clippy with warnings-as-errors
lint:
    cargo fmt -- --check
    cargo clippy -- -D warnings

# Clean build artifacts
clean:
    cargo clean

# Show help
help:
    @just --list
