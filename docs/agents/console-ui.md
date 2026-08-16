# Console UI

When changing `cloud/apps/web/`, follow root [`DESIGN.md`](../../DESIGN.md). Details: [`docs/Designs.md`](../Designs.md). Reference: [`docs/design/changes-and-checks.png`](../design/changes-and-checks.png). Ignore [`docs/design/DESIGN.md`](../design/DESIGN.md) (leftover Cursor export; not the Console spec).

## Dialogs

Do not use `window.alert` / `window.confirm` / `window.prompt` or equivalent blocking dialogs. Confirmations use an in-app styled modal / `<dialog>` per the Console design system in `docs/Designs.md`.

## Dropdowns / pickers

Do not use native HTML `<select>` / `<option>`. All option-picking UI must use in-app popup panels (button trigger + `shadow-menu` picker with `picker-item` rows). Reuse `components/ui` (`SearchablePicker` / `FieldSelect` / `Menu`) and `components/pickers`. New Agent / follow-up input uses `components/composer`. Backdrop click, Esc, or choosing an item closes the panel.

New Console capabilities follow the [feature slice](console-feature.md): domain type → db → `apps/api/src/features/<name>.rs` → `lib/cloud/<name>.ts` → hook → UI. Reuse an existing ability with `import { … } from "@/cap/<id>"` ([catalog](console-capabilities.md), `./cloud/scripts/use-capability.sh`). Do not call `fetch` / `api()` with path strings from pages.

## Toasts

Use `useToast()` from `components/Toast.tsx`. Do not use inline banners or error bars for feedback.

## Icons

Use [Lucide](https://lucide.dev/icons) via `lucide-react`. Re-export from `lib/icons.tsx` with `Icon*` names. Do not hand-write SVGs in components.

Small icons (≤16px): `strokeWidth={2}` + `absoluteStrokeWidth`. If Lucide has no match, pick the closest Lucide icon; only then hand-write SVG (24×24 viewBox, 2px stroke, `strokeLinecap="round"`).

Brand icons (GitHub / GitLab): official artwork in `cloud/apps/web/public/icons/`, inlined in `lib/icons.tsx`. GitHub: [Brand Toolkit Mark](https://brand.github.com/foundations/logo). GitLab: [gitlab-artwork](https://gitlab.com/gitlab-com/gitlab-artwork) logomark (`currentColor` in menus; do not restyle the mark).
