#!/bin/bash
# Setup test environment for nb-cli tests.
#
# Usage:
#   ./tests/setup_test_env.sh                    # default: local execution venv (.test-venv, ipykernel only)
#   ./tests/setup_test_env.sh local              # same as above, explicit
#   ./tests/setup_test_env.sh jsd                # jupyter-server-documents connect-mode venv (.test-venv-jsd)
#   ./tests/setup_test_env.sh jupyter-collaboration  # jupyter-collaboration connect-mode venv (.test-venv-collab)
#   ./tests/setup_test_env.sh none               # plain jupyter_server, no collab extension (.test-venv-plain)
#
# .test-venv (local) is shared by integration_local_mode, integration_execution, and
# integration_env_kernels — it only needs ipykernel, not any Jupyter Server extension.
#
# jupyter-collaboration and jupyter-server-documents are competing collaborative-editing
# extensions and must never be installed into the same venv, so each connect-mode backend
# gets its own venv directory.

set -e

BACKEND="${1:-local}"

# Package pins verified against current nb-cli code (see AGENTS.md for how/why).
JUPYTER_SERVER_PIN="jupyter_server==2.20.0"
JSD_PIN="jupyter-server-documents==0.2.5"
COLLAB_PIN="jupyter-collaboration==4.4.1"

case "$BACKEND" in
    local)
        VENV_DIR=".test-venv"
        PACKAGES=()
        ;;
    jsd|jupyter-server-documents)
        VENV_DIR=".test-venv-jsd"
        PACKAGES=("$JUPYTER_SERVER_PIN" "$JSD_PIN")
        ;;
    jsd-3)
        VENV_DIR=".test-venv-jsd-3"
        PACKAGES=("$JUPYTER_SERVER_PIN" "jupyter-server-documents==0.3.0")
        ;;
    jupyter-collaboration|collab)
        VENV_DIR=".test-venv-collab"
        PACKAGES=("$JUPYTER_SERVER_PIN" "$COLLAB_PIN")
        ;;
    none|plain)
        VENV_DIR=".test-venv-plain"
        PACKAGES=("$JUPYTER_SERVER_PIN")
        ;;
    *)
        echo "❌ Unknown backend '$BACKEND' (expected 'local', 'jsd', 'jsd-3', 'jupyter-collaboration', or 'none')"
        exit 1
        ;;
esac

echo "🔧 Setting up test environment for backend: $BACKEND..."

# Check if uv is installed
if ! command -v uv &> /dev/null; then
    echo "❌ uv is not installed"
    echo "📦 Install uv with: curl -LsSf https://astral.sh/uv/install.sh | sh"
    exit 1
fi

# Check if python3 is installed
if ! command -v python3 &> /dev/null; then
    echo "❌ python3 is not installed"
    echo "📦 Please install Python 3.8 or later"
    exit 1
fi

# Get the directory of this script
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
VENV_PATH="$SCRIPT_DIR/$VENV_DIR"

# Check if venv exists AND is valid
if [ ! -d "$VENV_PATH" ] || [ ! -f "$VENV_PATH/bin/python" ]; then
    echo "📦 Creating test virtual environment at $VENV_DIR..."
    rm -rf "$VENV_PATH"  # Clean up if partially exists
    uv venv "$VENV_PATH"
else
    echo "✅ Test venv already exists at $VENV_DIR"
fi

# Install ipykernel for Python kernel
echo "📦 Installing ipykernel..."
uv pip install --python "$VENV_PATH" ipykernel

# Install pinned collaboration-backend packages for connect-mode tests (skipped for local)
if [ "${#PACKAGES[@]}" -gt 0 ]; then
    echo "📦 Installing ${PACKAGES[*]}..."
    uv pip install --python "$VENV_PATH" "${PACKAGES[@]}"
fi

echo ""
echo "✅ Test environment ready ($BACKEND, $VENV_DIR)!"
echo ""
if [ "$BACKEND" = "local" ]; then
    echo "To run local/execution tests:"
    echo "  cargo test --test integration_local_mode"
    echo "  cargo test --test integration_execution"
    echo ""
    echo "To run all tests (connect-mode tests require their own venv, see above):"
    echo "  cargo test"
else
    echo "To run connect-mode tests against this backend (must be single-threaded):"
    echo "  NB_TEST_BACKEND=$BACKEND cargo test --test integration_connect_mode -- --test-threads=1"
fi
echo ""
