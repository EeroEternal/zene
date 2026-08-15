---
name: console-capability
description: Reuse a Cloud Console capability by name via import { … } from "@/cap/<id>" (LLM, repos, GitHub, composer, pickers). Load when building a Console page or choosing which module to import.
---

# Console capability

`import { … } from "@/cap/<id>"`. Mix ids. Do not copy `NewAgent.tsx`.

List or print imports:

```
./cloud/scripts/use-capability.sh
./cloud/scripts/use-capability.sh llm composer project-picker
```

Catalog: `cloud/apps/web/lib/capabilities.ts`. Barrels: `cloud/apps/web/lib/cap/<id>.ts`.

Composer already includes attach + model. New dropdown that is not project/branch/model: `@/cap/picker`.

Missing ability: `./cloud/scripts/new-feature.sh <kebab-name>` (writes the `@/cap/<id>` barrel too).

No native `<select>`. No `window.alert` / `confirm` / `prompt`. No `api("/api/v1/...")` in pages.
