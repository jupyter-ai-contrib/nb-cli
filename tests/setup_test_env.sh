#!/bin/bash
# Setup test environment for execution tests.
#
# Usage:
#   ./tests/setup_test_env.sh                    # default: jupyter-server-documents venv (.test-venv)
#   ./tests/setup_test_env.sh jsd                # same as above, explicit
#   ./tests/setup_test_env.sh jupyter-collaboration  # separate venv (.test-venv-collab)
#
# jupyter-collaboration and jupyter-server-documents are competing collaborative-editing
# extensions and must never be installed into the same venv, so each backend gets its
# own venv directory.

set -e

BACKEND="${1:-jsd}"

# Package pins verified against current nb-cli code (see AGENTS.md for how/why).
JUPYTER_SERVER_PIN="jupyter_server==2.20.0"
JSD_PIN="jupyter-server-documents==0.2.5"
COLLAB_PIN="jupyter-collaboration==4.4.1"

case "$BACKEND" in
    jsd|jupyter-server-documents)
        VENV_DIR=".test-venv"
        PACKAGES=("$JUPYTER_SERVER_PIN" "$JSD_PIN")
        ;;
    jupyter-collaboration|collab)
        VENV_DIR=".test-venv-collab"
        PACKAGES=("$JUPYTER_SERVER_PIN" "$COLLAB_PIN")
        ;;
    *)
        echo "❌ Unknown backend '$BACKEND' (expected 'jsd' or 'jupyter-collaboration')"
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

# Install pinned collaboration-backend packages for connect-mode tests
echo "📦 Installing ${PACKAGES[*]}..."
uv pip install --python "$VENV_PATH" "${PACKAGES[@]}"

echo ""
echo "✅ Test environment ready ($BACKEND, $VENV_DIR)!"
echo ""
echo "To run execution tests:"
echo "  cargo test --test integration_execution"
echo ""
echo "To run connect-mode tests against this backend (must be single-threaded):"
echo "  NB_TEST_BACKEND=$BACKEND cargo test --test integration_connect_mode -- --test-threads=1"
echo ""
echo "To run all tests:"
echo "  cargo test"
echo ""
