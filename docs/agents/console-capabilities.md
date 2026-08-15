# Console capabilities

Import one ability: `import { … } from "@/cap/<id>"`. Mix ids in the same file. List ids / print import lines with `./cloud/scripts/use-capability.sh` (optionally pass `llm composer project-picker`).

Catalog: [`cloud/apps/web/lib/capabilities.ts`](../../cloud/apps/web/lib/capabilities.ts). Barrels: `cloud/apps/web/lib/cap/<id>.ts`. Skill: `.cursor/skills/console-capability/SKILL.md`.

Do not clone `NewAgent.tsx`. Missing ability: [feature slice](console-feature.md) then `use-capability.sh <id>`.
