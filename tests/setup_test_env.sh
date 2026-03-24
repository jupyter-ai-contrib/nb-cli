#!/bin/bash
# Setup test environment for execution tests

set -e

echo "🔧 Setting up test environment..."

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
VENV_PATH="$SCRIPT_DIR/.test-venv"

# Check if venv exists AND is valid
if [ ! -d "$VENV_PATH" ] || [ ! -f "$VENV_PATH/bin/python" ]; then
    echo "📦 Creating test virtual environment..."
    rm -rf "$VENV_PATH"  # Clean up if partially exists
    uv venv "$VENV_PATH"
else
    echo "✅ Test venv already exists"
fi

# Install ipykernel for Python kernel
echo "📦 Installing ipykernel..."
uv pip install --python "$VENV_PATH" ipykernel

echo ""
echo "✅ Test environment ready!"
echo ""
echo "To run execution tests:"
echo "  cargo test --test integration_execution"
echo ""
echo "To run all tests:"
echo "  cargo test"
echo ""
