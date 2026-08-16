#!/usr/bin/env bash
# Scaffold a Cloud Console feature slice (API router + typed client + hook).
# Usage: ./cloud/scripts/new-feature.sh <name>
# Name: lowercase snake or kebab, e.g. widgets or team-tokens.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RAW="${1:-}"
if [[ -z "$RAW" || "$RAW" == "-h" || "$RAW" == "--help" ]]; then
  echo "usage: $0 <feature-name>" >&2
  echo "example: $0 widgets" >&2
  echo "see docs/agents/console-feature.md" >&2
  exit 1
fi
if [[ ! "$RAW" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo "name must be lowercase kebab-case starting with a letter" >&2
  exit 1
fi

NAME="${RAW//-/_}"
KEBAB="${RAW//_/-}"
PASCAL="$(python3 - << PY
name = "${NAME}"
print("".join(p.title() for p in name.split("_")))
PY
)"
CAMEL="$(python3 - << PY
name = "${NAME}"
parts = name.split("_")
print(parts[0] + "".join(p.title() for p in parts[1:]))
PY
)"

FEATURE_RS="$ROOT/cloud/apps/api/src/features/${NAME}.rs"
CLIENT_TS="$ROOT/cloud/apps/web/lib/cloud/${NAME}.ts"
HOOK_TS="$ROOT/cloud/apps/web/lib/hooks/use${PASCAL}.ts"
MOD_RS="$ROOT/cloud/apps/api/src/features/mod.rs"
ROUTES_RS="$ROOT/cloud/apps/api/src/routes.rs"
CLOUD_INDEX="$ROOT/cloud/apps/web/lib/cloud/index.ts"
HOOKS_INDEX="$ROOT/cloud/apps/web/lib/hooks/index.ts"
TYPES_TS="$ROOT/cloud/apps/web/lib/types.ts"
CAP_TS="$ROOT/cloud/apps/web/lib/cap/${KEBAB}.ts"
CAPS_TS="$ROOT/cloud/apps/web/lib/capabilities.ts"

for f in "$FEATURE_RS" "$CLIENT_TS" "$HOOK_TS" "$CAP_TS"; do
  if [[ -e "$f" ]]; then
    echo "already exists: $f" >&2
    exit 1
  fi
done
if grep -q "pub mod ${NAME};" "$MOD_RS"; then
  echo "module already registered: ${NAME}" >&2
  exit 1
fi

cat > "$FEATURE_RS" << EOF
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/${KEBAB}", get(list_${NAME}).post(create_${NAME}))
}

async fn list_${NAME}(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<Value>>, AppError> {
    let _org = state.db.primary_org(user.id).await?;
    Ok(Json(Vec::new()))
}

async fn create_${NAME}(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _org = state.db.primary_org(user.id).await?;
    let Value::Object(mut body) = req else {
        return Err(AppError::bad_request("json object required"));
    };
    body.entry("id")
        .or_insert_with(|| json!(Uuid::new_v4().to_string()));
    Ok(Json(Value::Object(body)))
}
EOF

cat > "$CLIENT_TS" << EOF
import type { ${PASCAL} } from "@/lib/types";
import { getJson, postJson } from "./http";

export const ${CAMEL}Api = {
  list: () => getJson<${PASCAL}[]>("/api/v1/${KEBAB}"),
  create: (body: Omit<${PASCAL}, "id">) => postJson<${PASCAL}>("/api/v1/${KEBAB}", body),
};
EOF

cat > "$HOOK_TS" << EOF
"use client";

import { useCallback } from "react";
import { ${CAMEL}Api } from "@/lib/cloud";
import { useCloudGet } from "./useCloudGet";

export function use${PASCAL}() {
  const loader = useCallback(() => ${CAMEL}Api.list(), []);
  return useCloudGet(loader);
}
EOF

cat > "$CAP_TS" << EOF
"use client";

export { ${CAMEL}Api } from "../cloud/${NAME}";
export { use${PASCAL} } from "../hooks/use${PASCAL}";
EOF

python3 - << PY
from pathlib import Path

mod = Path("$MOD_RS")
text = mod.read_text()
line = "pub mod ${NAME};\n"
if line not in text:
    mods = [ln for ln in text.splitlines(True) if ln.startswith("pub mod ")]
    rest = [ln for ln in text.splitlines(True) if not ln.startswith("pub mod ")]
    mods.append(line)
    mods.sort()
    mod.write_text("".join(mods + rest))

routes = Path("$ROUTES_RS")
rt = routes.read_text()
needle = ".merge(crate::features::github::router())"
insert = needle + "\n        .merge(crate::features::${NAME}::router())"
if "features::${NAME}::router()" not in rt:
    if needle not in rt:
        raise SystemExit("routes.rs: github merge not found; add .merge(crate::features::${NAME}::router()) manually")
    routes.write_text(rt.replace(needle, insert, 1))

cloud = Path("$CLOUD_INDEX")
ct = cloud.read_text()
export = 'export { ${CAMEL}Api } from "./${NAME}";\n'
if export not in ct:
    cloud.write_text(ct.rstrip() + "\n" + export)

hooks = Path("$HOOKS_INDEX")
ht = hooks.read_text()
hexport = 'export { use${PASCAL} } from "./use${PASCAL}";\n'
if hexport not in ht:
    hooks.write_text(ht.rstrip() + "\n" + hexport)

types = Path("$TYPES_TS")
tt = types.read_text()
block = """
/** Scaffolded by cloud/scripts/new-feature.sh — replace with domain types. */
export interface ${PASCAL} {
  id: string;
}
"""
if "export interface ${PASCAL} " not in tt:
    types.write_text(tt.rstrip() + block)

caps = Path("$CAPS_TS")
ctext = caps.read_text()
entry = '''  "${KEBAB}": {
    use: "TODO",
    symbols: ["${CAMEL}Api", "use${PASCAL}"],
    files: [
      "cloud/apps/api/src/features/${NAME}.rs",
      "cloud/apps/web/lib/cloud/${NAME}.ts",
      "cloud/apps/web/lib/hooks/use${PASCAL}.ts",
    ],
  },
'''
# unquoted key when kebab has no hyphen
if "-" not in "${KEBAB}":
    entry = entry.replace('"${KEBAB}":', "${KEBAB}:", 1)
if "${KEBAB}" not in ctext.split("as const")[0]:
    marker = "} as const;"
    if marker not in ctext:
        raise SystemExit("capabilities.ts: missing } as const;")
    caps.write_text(ctext.replace(marker, entry + marker, 1))
PY

echo "created:"
echo "  $FEATURE_RS"
echo "  $CLIENT_TS"
echo "  $HOOK_TS"
echo "  $CAP_TS"
echo "wired features/mod.rs, routes.rs, lib/cloud/index.ts, lib/hooks/index.ts, lib/types.ts, lib/capabilities.ts"
echo "reuse: ./cloud/scripts/use-capability.sh ${KEBAB}"
echo "next: domain struct + Db + real handlers (docs/agents/console-feature.md)"
echo "then: cd cloud/apps/web && npx tsc --noEmit && npm test"
echo "      cargo test -p zene-cloud-api --locked"
