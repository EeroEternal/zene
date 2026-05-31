#!/usr/bin/env bash
set -euo pipefail

# =====================================================================
# Zene One-Line Installation Script
# Supports: Linux (x86_64) and macOS (Intel / Apple Silicon)
# =====================================================================

echo "=== Zene Installation Script ==="

# 1. Detect OS and Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"
TARGET=""

case "$OS" in
  Linux)
    if [ "$ARCH" = "x86_64" ]; then
      TARGET="x86_64-unknown-linux-gnu"
    fi
    ;;
  Darwin)
    if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
      TARGET="aarch64-apple-darwin"
    elif [ "$ARCH" = "x86_64" ]; then
      TARGET="x86_64-apple-darwin"
    fi
    ;;
esac

INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"
BINARY_PATH="$INSTALL_DIR/zene"

# 2. Try downloading pre-built binary
if [ -n "$TARGET" ]; then
  echo "Detected platform: $OS ($ARCH)"
  echo "Fetching latest release tag from GitHub..."
  
  # Fetch latest release tag
  LATEST_TAG=$(curl -sfI https://github.com/ParaTensor/zene/releases/latest | grep -i "location:" | grep -oE "tag/v[0-9.]+" | cut -d/ -f2 || echo "")
  
  # Fallback to a default version if API fails
  if [ -z "$LATEST_TAG" ]; then
    LATEST_TAG="v0.1.0"
  fi
  
  DOWNLOAD_URL="https://github.com/ParaTensor/zene/releases/download/${LATEST_TAG}/zene-${TARGET}"
  
  echo "Downloading pre-built binary from: $DOWNLOAD_URL"
  if curl -sfL "$DOWNLOAD_URL" -o "$BINARY_PATH"; then
    chmod +x "$BINARY_PATH"
    echo "✓ Zene binary successfully installed to $BINARY_PATH"
    
    # Check macOS Quarantine attribute
    if [ "$OS" = "Darwin" ]; then
      if command -v xattr &> /dev/null; then
        xattr -d com.apple.quarantine "$BINARY_PATH" 2>/dev/null || true
      fi
    fi
    
    echo "=== Installation Completed ==="
    echo "Make sure $INSTALL_DIR is in your shell PATH."
    exit 0
  else
    echo "Could not download pre-built binary. Falling back to source compilation..."
  fi
else
  echo "Unsupported pre-built platform: $OS ($ARCH). Falling back to source compilation..."
fi

# 3. Source compilation fallback
echo "=== Building from Source ==="

# Check Rust/Cargo
if ! command -v cargo &> /dev/null; then
  echo "Rust/Cargo not found. Installing via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi

echo "Installing Zene from GitHub..."
if cargo install --git https://github.com/ParaTensor/zene --locked; then
  # Cargo installs to ~/.cargo/bin
  echo "✓ Zene installed to ~/.cargo/bin/zene"
  echo "=== Installation Completed ==="
  echo "Make sure ~/.cargo/bin is in your shell PATH."
else
  echo "Error: Installation failed."
  exit 1
fi
