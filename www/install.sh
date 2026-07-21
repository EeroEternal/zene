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
ZENE_PATH="$INSTALL_DIR/zene"
GATEWAY_PATH="$INSTALL_DIR/zene-gateway"

install_binary() {
  local url="$1"
  local dest="$2"
  local label="$3"
  echo "Downloading $label from: $url"
  if curl -sfL "$url" -o "$dest"; then
    chmod +x "$dest"
    echo "✓ $label installed to $dest"
    return 0
  fi
  echo "Could not download $label."
  return 1
}

# 2. Try downloading pre-built binaries
if [ -n "$TARGET" ]; then
  echo "Detected platform: $OS ($ARCH)"
  echo "Fetching latest release tag from GitHub..."
  
  # Fetch latest release tag
  LATEST_TAG=$(curl -sfI https://github.com/ParaTensor/zene/releases/latest | grep -i "location:" | grep -oE "tag/v[0-9.]+" | cut -d/ -f2 || echo "")
  
  # Fallback to a default version if API fails
  if [ -z "$LATEST_TAG" ]; then
    LATEST_TAG="v0.1.0"
  fi
  
  ZENE_URL="https://github.com/ParaTensor/zene/releases/download/${LATEST_TAG}/zene-${TARGET}"
  GATEWAY_URL="https://github.com/ParaTensor/zene/releases/download/${LATEST_TAG}/zene-gateway-${TARGET}"
  
    if install_binary "$ZENE_URL" "$ZENE_PATH" "zene" \
    && install_binary "$GATEWAY_URL" "$GATEWAY_PATH" "zene-gateway"; then
    # Check macOS Quarantine attribute
    if [ "$OS" = "Darwin" ]; then
      if command -v xattr &> /dev/null; then
        xattr -d com.apple.quarantine "$ZENE_PATH" 2>/dev/null || true
        xattr -d com.apple.quarantine "$GATEWAY_PATH" 2>/dev/null || true
      fi
    fi

    LEGACY_DIR="$HOME/.cargo/bin"
    for legacy in "$LEGACY_DIR/zene" "$LEGACY_DIR/zene-gateway"; do
      if [ -f "$legacy" ]; then
        echo "⚠ Removing older Cargo install: $legacy"
        rm -f "$legacy"
      fi
    done
    
    echo "=== Installation Completed ==="
    echo "Installed to $INSTALL_DIR"
    echo "Run: $ZENE_PATH web --yolo --sandbox-off"
    echo "Ensure $INSTALL_DIR is before ~/.cargo/bin in your PATH."
    exit 0
  else
    echo "Could not download pre-built binaries. Falling back to source compilation..."
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
if cargo install --git https://github.com/ParaTensor/zene --locked zene-cli \
  && cargo install --git https://github.com/ParaTensor/zene --locked zene-gateway --bin zene-gateway; then
  # Cargo installs to ~/.cargo/bin
  echo "✓ zene installed to ~/.cargo/bin/zene"
  echo "✓ zene-gateway installed to ~/.cargo/bin/zene-gateway"
  echo "=== Installation Completed ==="
  echo "Make sure ~/.cargo/bin is in your shell PATH."
else
  echo "Error: Installation failed."
  exit 1
fi
