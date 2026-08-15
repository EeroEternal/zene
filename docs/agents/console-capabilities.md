# Console capabilities

Capabilities are **named importable modules**, not a single mega-component. A Cursor skill (`.cursor/skills/console-capability/SKILL.md`) is only the index: look up an id, import those symbols.

Source of truth: [`cloud/apps/web/lib/capabilities.ts`](../../cloud/apps/web/lib/capabilities.ts).

To reuse one ability when rebuilding a page, pick an id (`llm`, `composer`, `project-picker`, …) and paste its `import` lines. Do not clone `NewAgent.tsx`. To add an ability that is missing, ship a [feature slice](console-feature.md) then append a row to the catalog.
