#!/bin/bash
set -e

VERSION="v0.0.1"
REPO="jupyter-ai-contrib/nb-cli"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Determine platform
case "$OS" in
    darwin)
        case "$ARCH" in
            arm64) BINARY="nb-macos-arm64" ;;
            x86_64) BINARY="nb-macos-amd64" ;;
            *)
                echo "❌ Unsupported macOS architecture: $ARCH"
                echo "Please install from source: cargo install nb-cli"
                exit 1
                ;;
        esac
        ;;
    linux)
        case "$ARCH" in
            x86_64) BINARY="nb-linux-amd64" ;;
            aarch64|arm64) BINARY="nb-linux-arm64" ;;
            *)
                echo "❌ Unsupported Linux architecture: $ARCH"
                echo "Please install from source: cargo install nb-cli"
                exit 1
                ;;
        esac
        ;;
    *)
        echo "❌ Unsupported OS: $OS"
        echo "Please install from source: cargo install nb-cli"
        exit 1
        ;;
esac

echo "📦 Installing nb-cli $VERSION for $OS ($ARCH)..."
echo ""

# Download URL
URL="https://github.com/$REPO/releases/download/$VERSION/$BINARY"

# Installation directory (can be overridden with INSTALL_DIR env var)
INSTALL_DIR="${INSTALL_DIR:-$HOME/.nb-cli/bin}"

# Create install directory if it doesn't exist
mkdir -p "$INSTALL_DIR"

# Download the binary
echo "⬇️  Downloading from $URL..."
if ! curl -L "$URL" -o "$INSTALL_DIR/nb" 2>/dev/null; then
    echo "❌ Failed to download. The binary for your platform may not be available yet."
    echo ""
    echo "Available installation methods:"
    echo "  1. Install from source: cargo install nb-cli"
    echo "  2. Check available binaries: https://github.com/$REPO/releases/tag/$VERSION"
    exit 1
fi

# Make it executable
chmod +x "$INSTALL_DIR/nb"

echo ""
echo "✅ nb-cli installed successfully to $INSTALL_DIR/nb"
echo ""

# Check if install dir is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo "⚠️  $INSTALL_DIR is not in your PATH"
    echo ""
    echo "To add nb to your PATH, run:"
    echo ""
    case "$SHELL" in
        */zsh)
            echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc"
            echo "  source ~/.zshrc"
            ;;
        */bash)
            echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
            echo "  source ~/.bashrc"
            ;;
        *)
            echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.profile"
            echo "  source ~/.profile"
            ;;
    esac
    echo ""
    echo "Or for this session only:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
fi

echo "Verify installation:"
if [[ ":$PATH:" == *":$INSTALL_DIR:"* ]]; then
    echo "  nb --version"
else
    echo "  $INSTALL_DIR/nb --version"
    echo ""
    echo "After adding to PATH:"
    echo "  nb --version"
fi
