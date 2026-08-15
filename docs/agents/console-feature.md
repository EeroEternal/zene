# Console feature slice

To add a Console capability that works in the UI and API, ship **one vertical slice**. Do not add a page that calls `fetch("/api/v1/...")`, and do not add a handler only in `routes.rs`. Copy `llm` or `repositories` (both already sliced) or run `cloud/scripts/new-feature.sh <name>`. After it exists, add a row to [`cloud/apps/web/lib/capabilities.ts`](../../cloud/apps/web/lib/capabilities.ts) so later work can import it by id. Reuse catalog: [`console-capabilities.md`](console-capabilities.md).

Worked examples: `cloud/apps/api/src/features/llm.rs` + `cloud/apps/web/lib/cloud/llm.ts` + `cloud/apps/web/lib/hooks/useLlmSettings.ts`; same pattern for `repositories` and `github`.

## Layers (required, in this order)

1. **Domain** — request/response structs in `cloud/crates/domain/src/lib.rs` (`camelCase` serde). Mirror them in `cloud/apps/web/lib/types.ts`.
2. **DB** — queries on `zene_cloud_db::Db` (`cloud/crates/db/src/`). Add a SQL migration under `cloud/migrations/` when the table is new.
3. **API feature** — `cloud/apps/api/src/features/<name>.rs` with `pub fn router() -> Router<AppState>`. Paths and handlers live in that file. Register `pub mod <name>;` in `features/mod.rs` and `.merge(crate::features::<name>::router())` in `routes.rs`.
4. **Typed client** — `cloud/apps/web/lib/cloud/<name>.ts` using `getJson` / `postJson` / `putJson` / `patchJson` / `deleteJson` from `lib/cloud/http.ts`. Export from `lib/cloud/index.ts`. Pages must not import `@/lib/api` except for `loadToken` / `setToken` / status helpers.
5. **Hook** — `cloud/apps/web/lib/hooks/use<Name>.ts` (or `useCloudGet(() => fooApi.list())`). Export from `lib/hooks/index.ts`.
6. **UI** — screens in `cloud/apps/web/components/`. Inputs: `Composer` + `useComposerText`. Pickers: `components/ui` (`SearchablePicker` / `FieldSelect` / `Menu`) and `components/pickers`. Feedback: `useToast`, `ConfirmDialog`, `PromptDialog`. No native `<select>`, no `window.alert` / `confirm` / `prompt`.

`runs` stay in `routes.rs` until they are split the same way; new run sub-resources still go through `runsApi` on the client.

## Scaffold

From the repo root:

```
./cloud/scripts/new-feature.sh widgets
```

This writes a compiling `GET/POST /api/v1/widgets` feature, typed client, and hook, and wires `mod.rs` / `routes.rs` / TS barrels. GET returns `[]`; POST echoes the JSON body with an `id`. Replace `serde_json::Value` and the generated TS interface with domain types, then persist in `Db`.

## Checklist for agents

- Path string appears in **both** `features/<name>.rs` and `lib/cloud/<name>.ts` (pin it in `lib/cloud/cloud.test.ts`).
- Auth: user routes take `AuthUser`; worker routes take `WorkerAuth`.
- Org-scoped rows go through `state.db.primary_org(user.id)`.
- After wiring, `cd cloud/apps/web && npx tsc --noEmit && npm test` and `cargo test -p zene-cloud-api --locked`.
