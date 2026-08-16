#!/usr/bin/env bash
# Print import lines for one or more Console capabilities.
# Usage: ./cloud/scripts/use-capability.sh
#        ./cloud/scripts/use-capability.sh llm composer project-picker
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
exec node --experimental-strip-types "$ROOT/cloud/apps/web/lib/cap/cli.ts" "$@"
