#!/usr/bin/env bash
set -euo pipefail

# Locate repo root
cd "$(dirname "$0")/.."

# Execute pre-render builder
node site/build.js
