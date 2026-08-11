#!/usr/bin/env bash
# Publish Zene composable crates to crates.io (dependency order).
#
# Usage:
#   ./scripts/publish-crates.sh --verify   # local cargo package (all crates, with path patch)
#   ./scripts/publish-crates.sh --dry-run  # crates.io dry-run (config-only until deps are live)
#   ./scripts/publish-crates.sh            # upload to crates.io (needs API token)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

PATCH_CONFIG="$ROOT/.cargo/publish-local.toml"
PACKAGES=(zene-config zene-llm zene-session zene-context)

mode="${1:-}"

verify_local() {
  echo "Verifying packaging with local path patch..."
  for pkg in "${PACKAGES[@]}"; do
    echo "==> cargo package -p ${pkg} --allow-dirty"
    cargo package -p "$pkg" --allow-dirty --config "patch.crates-io.zene-config.path=\"crates/config\"" \
      --config "patch.crates-io.zene-llm.path=\"crates/llm\"" \
      --config "patch.crates-io.zene-session.path=\"crates/session\""
  done
  echo "All packages verified."
}

publish_crates() {
  local extra=("$@")
  for pkg in "${PACKAGES[@]}"; do
    echo "==> cargo publish -p ${pkg} ${extra[*]:-}"
    cargo publish -p "$pkg" "${extra[@]}" --allow-dirty
  done
}

case "$mode" in
  --verify)
    verify_local
    ;;
  --dry-run)
    echo "Note: dry-run for zene-llm+ requires prior crates already on crates.io."
    echo "Use --verify for full local packaging check."
    publish_crates --dry-run
    ;;
  "")
    publish_crates
    ;;
  *)
    echo "Unknown option: $mode" >&2
    echo "Usage: $0 [--verify|--dry-run]" >&2
    exit 1
    ;;
esac

echo "Done."
