---
name: console-capability
description: Reuse a Cloud Console capability by name (LLM, repos, GitHub, composer, pickers, menus, dialogs) or add a new vertical slice. Load when building a Console page, choosing which module to import, or adding an API that should work in the UI.
---

# Console capability

Do not copy-paste picker/composer/fetch code into a new page. Look up one **capability id** and import those symbols.

Catalog (source of truth): `cloud/apps/web/lib/capabilities.ts`

## Reuse one capability

Open the catalog, pick an id (`llm`, `repositories`, `github`, `session`, `runs`, `composer`, `project-picker`, `branch-picker`, `model-picker`, `attach-menu`, `picker`, `menu`, `dialogs`, `http`), paste its `import` lines, wire props. Do not open `NewAgent.tsx` / `RunView.tsx` to duplicate UI.

Composer already includes attach + model. For a new prompt box, import `composer` only (`useComposerText` + `Composer`). For a new dropdown that is not project/branch/model, import `picker` (`SearchablePicker` / `FieldSelect`).

## Add a capability that does not exist

Follow `docs/agents/console-feature.md`. From repo root:

```
./cloud/scripts/new-feature.sh <kebab-name>
```

Then add a row to `cloud/apps/web/lib/capabilities.ts` so later work can import it by id.

## Constraints

No native `<select>`. No `window.alert` / `confirm` / `prompt`. No `api("/api/v1/...")` in pages — add a method on the matching `*Api` in `lib/cloud/`.
