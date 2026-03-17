#!/usr/bin/env bash
# Source this file to add the development nb binary to your PATH
# Usage: source dev-env.sh
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$SCRIPT_DIR/target/release:$PATH"
echo "✓ Development environment activated"
echo "  nb binary: $SCRIPT_DIR/target/release/nb"
echo ""
echo "Run 'which nb' to verify you're using the dev version"
