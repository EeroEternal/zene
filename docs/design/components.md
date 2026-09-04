# Components

Components follow semantic roles before custom styling. Reuse baseline UI primitives and preserve their intended variants.

- Buttons: use semantic variants for primary, secondary, ghost, link, and destructive actions. Do not restyle a button into a new primary system with ad hoc classes.
- Dialogs and alert dialogs: keep the `header -> title -> description -> content -> footer` structure and align primary and secondary actions consistently.
- Dialog viewport bounds & collapsible content (global): all dialogs, sheets, and alert overlays must strictly bound their vertical and horizontal footprint within the screen (`max-h-[85vh]` or `max-h-[90vh]` with `overflow-y-auto`). Large blocks of examples, technical tokens, long JSON/cURL blocks, or secondary options inside dialogs must use collapsible accordions (`CollapsibleUsageCodeBlock`) or tabs, never unconstrained vertical stacking that pushes confirmation buttons or dialog headers off-screen.
- Overlay dismiss (global): clicking the dimmed area outside a popup (Dialog / Sheet / AlertDialog / Popover) must close it. Escape must also close it. Do not block `onOpenChange(false)` for outside clicks or Escape unless a rare, explicitly documented exception is required (for example an irreversible one-shot secret reveal that still unfinished). Prefer the shared `Dialog` / `AlertDialog` / `Sheet` primitives — they already dismiss on overlay click; never reimplement a modal that ignores outside clicks.
- No native browser popups: never use `window.confirm`, `window.alert`, or `window.prompt` (or the bare `confirm` / `alert` / `prompt` globals) in Admin UI. Destructive actions (delete from ellipsis menus, confirm buttons, bulk remove, clear, revoke, etc.) must use shared `AlertDialog` / `ConfirmAlertDialog`. Short success or error feedback uses toast; blocking confirmations use our Dialog/AlertDialog overlays so styling, i18n, and overlay-dismiss rules stay consistent.
- All entity creation opens as a Dialog over the list — never a full-page form or page replacement. Simple creates (projects, organizations, users, providers, models, **API Keys**) use a compact create Dialog. API Key create is a single page: name + project + route, with call boundaries in a fold; do not use a stepper. Route create remains a multi-step Dialog wizard (`sm:max-w-5xl`, stepper + main panel + summary sidebar). Cancel / overlay click / Escape close the dialog. Entity **detail / edit** always stays a compact Dialog.
- API Key create and Route create are separate flows. Creating an API Key only binds an **existing** route (name + project + route select). Do not embed “create dedicated route”, strategy/provider pickers, or smart-routing setup inside the API Key wizard. Create routes on the Routes page; create keys on the API Keys page.
- Toasts: transient feedback should use the shared top-center toast pattern instead of a blocking modal. Success feedback uses the solid `success` green surface with `success-foreground` text, keeps copy short, and disappears without interrupting the current workflow.
- Inputs, selects, and switches: favor the shared component set instead of raw HTML controls. Native `select` should not appear in product pages.
- **One primary button per view.** Height 40px, radius 6px, pad 12×8. Hover darkens primary (`#1F3A89`); active `#182F70`; disabled 50% opacity; keyboard focus is a 2px primary ring.
- Secondary button: white fill, 1px border, hover `muted`. Icon button: 28×28, 16px icon, transparent until hover.
- Form `Input`: 40px, radius 4px, 1px border. Focus = primary border + 2px ring. Error = destructive border. Disabled = muted fill.
- Toolbar `Select` / date trigger: 32px, radius 8px. Panel max-height 240px (select) / 384px (date), scroll inside. Show 3–5 options when possible.
- Top-bar search (if present): 32px, radius 8px, muted fill until focus. Dropdown 320×360 max.
- `Switch`: track 36×20, thumb 16, radius 10px, 150ms ease. On = primary track, not a new blue.
- `Tabs`: sibling content switch under the page title (2–8 items). Active = primary text + 2px primary underline. **Not** hierarchy navigation. No icon+label mix. Single selection.
- Select / Combobox option labels show **only the entity identity** (usually the display name). Do not pack protocol, strategy, bound-key counts, status, or other secondary fields into the option string with `·` / `/`. After the user picks a value, secondary facts may appear in a separate summary row, labeled fields, or the detail dialog — never inside the dropdown options themselves. Canonical good examples: API Key detail route select and `ApiKeyRouteSummary` (`label: item.name`).
- Badges: reserve for status and compact metadata; they must support longer text when localization expands labels.
- Tables: selected state belongs to `bg-primary/10` and medium-weight text. Do not use a theme-colored left border as a selection indicator.
- Cards: use them as containers for major sections, but avoid duplicating the page title as a second identical card title.

### Detail presentation modes

Every detail surface must choose one mode. Do not invent a fourth presentation for ordinary entity work.

| Mode | Use when | Desktop | Narrow |
| --- | --- | --- | --- |
| Master–detail workspace | Operators need to scan many rows and keep list context (logs, providers, orgs/projects browser) | Side-by-side list + detail; both bordered cards with independent scroll | Full-width Sheet/Dialog for detail; restore list state on close |
| Compact detail Dialog | Single-entity inspect/edit (API Keys, Routes, users) | `max-w-3xl` Dialog | Same Dialog, body scrolls, footer stays reachable |
| Independent detail page | Deep, shareable objects that need a durable URL | Dedicated route | Single-column page |

Default for ordinary entities is the compact Dialog. Use the master–detail workspace only when continuous multi-row browsing is the primary job.

Canonical plan: `docs/dev/admin-ux-plan.md`.

### Master–detail workspace (canonical)

Desktop (`>= 1280px` content width):

1. Page title, filters, and workspace share one left edge inside the same page container.
2. Workspace is a two-column grid: list `minmax(640px, 1fr)`, detail `440px`, gap `24px`.
3. Do not overlay the detail with absolute positioning over the list.
4. Both panes use `rounded-lg border border-border bg-card`; no strong shadow for ordinary split panes.
5. Both pane shells stretch to the **same workspace height** (`h-full` / `xl:h-full` on the grid and each pane). Do not let content height leave one card shorter than the other.
6. Each pane owns its own vertical scroll; the page shell does not grow when detail content expands. Prefer shared `TwoPanelLayout` with `workspace`.

Narrow (`< 1280px`):

1. Do not squeeze list and detail into one compressed row.
2. Open detail as a full-width Sheet or Dialog.
3. Closing detail restores the previous list scroll position and filters.

Detail pane structure:

1. Header: min-height `72px`, padding `16px 24px`; title, status, and close share one baseline.
2. Body: padding `24px`; order is overview → grouped fields → events/advanced.
3. Optional footer: padding `16px 24px` with a top border; at most one primary action.
4. Group spacing `24px`, field spacing `16px`. Prefer section titles and `border-t` over nested cards for every group.

### Entity list pattern (canonical)

Most admin entity lists (models / providers, projects, organizations, API Keys, users) share one table list pattern. New list pages must follow it instead of inventing a second row language.

Canonical reference implementation: `admin/src/components/instances/ProviderList.tsx`.

Structure:

1. Outer card: `rounded-xl border bg-card p-4 sm:p-6`
2. Toolbar (`EntityListToolbar`): 100% full-width container with dual-wing semantic alignment:
   - **Left wing (search & filters)**: search input (`pl-9` + search icon, responsive width `w-64 sm:w-72 lg:w-80`) + business filter selects (project, org, status, protocol).
   - **Right wing (sort & count & actions)**: sort selector (with direction `Sort: Name (A→Z)` / `Sort: Price (low→high)`) + right-aligned result count text (`总共 N 个` / `Total N`).
   - **Narrow split-pane mode (`layout="two-row"`)**: used in master–detail left panels (e.g. `ProviderList`); Row 1 is a 100% full-width search input matching the table width below, and Row 2 hosts left filters and right sort/count aligned to edges.
3. Sort/filter controls: sort option labels include direction (e.g. `Sort: Price (low→high)`). See [`ui-entry.md`](../ai/agents/ui-entry.md#列表排序与检索).
4. `Table` with `table-fixed`
6. Sticky header: `TableHeader` with `sticky top-0 bg-card`
7. First column header uses normal horizontal padding (`px-4` / `pl-4`); do not reserve space for a selection accent bar
8. Rows: `cursor-pointer hover:bg-muted/50`; selected row uses `bg-primary/10` with medium-weight text so the current item is clearly visible against white/card surfaces
9. No theme-colored left border / vertical accent for selection. Keyboard focus uses a visible `ring`, separate from selected state
10. Status uses `Badge variant="outline"` with enabled `bg-primary/10 text-primary border-primary/20`, disabled `bg-muted text-muted-foreground border-0`
11. Trailing actions cell: `MoreVertical` ghost icon button inside `DropdownMenu modal={false}`; cell must `stopPropagation` so menu clicks do not open the row
12. Destructive delete belongs in the ellipsis menu (`text-destructive`), confirmed by `AlertDialog` / `ConfirmAlertDialog` — do not require opening the detail dialog only to delete, and never use native `confirm()` / `alert()` for that confirmation

List content rules:

- Keep one semantic per column. Do not concatenate unrelated fields with `·` (for example route name and strategy, or QPS and concurrency).
- One line per cell: a table column must not stack a primary value and a secondary subtitle (e.g. route name above protocol / “auto-created”). Put each field in its own column; drop low-value provenance labels from the list when they do not help scanning (auto-created can stay in overview stats or detail, not under the name).
- Numeric operational limits use separate columns with clear headers (e.g. `QPS`, `Concurrency`) and tabular numbers.
- Prefer truncation with `truncate` inside fixed-width columns over stretching sparse content across the full viewport.
- Detail editing opens a compact Dialog following the Entity detail dialog pattern; prefer `⋯` → Edit over bare row click for credential / route style lists.
- Master–detail browsers may open the right-hand detail pane on row click; credential / route style lists still prefer `⋯` → Edit.
- All creates open a Dialog over the list; Route create uses the Dialog wizard shell described under Hard rules — never replace the list page.

### Organization / project naming display

Organization and project are separate entities. UI copy must keep them separate.

- Organization selectors, filters, labels, and table cells show **only** the organization display name.
- Project selectors, filters, labels, and table cells show **only** the project display name.
- Never concatenate the two into one string with `/` or ` / ` (for example `default / default`, `Org/Project`). That pattern is forbidden in select options, form summaries, table cells, badges, and detail sidebars.
- When both values are needed, use separate labeled fields or separate columns (canonical reference: Projects list — project name column + organization column).
- The seeded compatibility entities (`id === 1` and storage name `default`) must render localized display names via `organizations.defaultName` / `projects.defaultName` in product UI; do not surface the raw storage name `default` as the primary label when those helpers exist.

### Entity detail dialog pattern (canonical)

List-row detail / edit for dense entities (API Keys, Routes, and similar) opens as a **compact Dialog**, not a full page and not a vertically stacked long form. Canonical references: `admin/src/components/api-keys/ApiKeyDetailDialog.tsx`, `admin/src/components/routes/RouteDetailDialog.tsx`.

Open trigger:

1. Prefer the trailing `⋯` menu item **Edit** (same as delete / other row actions).
2. Do not open the detail dialog on bare row click unless the list is explicitly a master-detail browser (rare). Default for credential / route style lists is menu-only open.
3. Overlay click and Escape must close the dialog (`onOpenChange`).

Shell:

1. `DialogContent`: `max-h-[90vh] max-w-3xl overflow-y-auto`
2. `DialogHeader`: entity name as `DialogTitle` (`pr-8 break-all`); one short `DialogDescription` for purpose — do not repeat the name as a second in-body card title
3. Body: `space-y-4` between major blocks
4. `DialogFooter`: secondary **Close** (`variant="secondary"`) + primary **Save**; optional utility action (e.g. “Create API Key”) may sit on the footer’s leading side as `outline`, never as a second primary

Body composition (keep it short):

1. **Overview card** — one `rounded-lg border border-border p-4`:
   - Top: compact meta grid (`gap-3`, usually `sm:grid-cols-2` or `sm:grid-cols-3`) with `text-xs` muted labels and dense controls (`h-9`, often `bg-muted/30`)
   - Related primary fields share one horizontal row with matching label height (`min-h-5`) and control height (`h-9`)
   - Section micro-labels use `text-[11px] font-medium uppercase tracking-[0.08em] text-muted-foreground`
   - Optional `border-t` block for secondary overview fields (strategy, bound route summary, etc.)
2. **Sibling operational cards** — `grid gap-4 md:grid-cols-2`:
   - Equal visual weight; each card is `rounded-lg border border-border p-4`
   - One concern per card (e.g. rate limit vs quota; service instances vs rate control)
   - Advanced / rarely edited parameters stay behind a ghost “Advanced” disclosure, not always expanded
3. Inline notices (legacy mapping, validation errors) use thin tinted banners above the overview card — do not invent a third card language for errors

Density rules:

- Labels: `12px` medium muted, label slot min-height `20px`.
- Body / values: `14px`. Controls and dense buttons: `36px` (`h-9`); page-toolbar primary buttons may stay `40px`.
- Ordinary fields use a two-column grid with `16px` gaps; long text, secrets, JSON, and request bodies stay full width.
- Prefer read+edit in the same compact cell over a separate “view mode” then “edit mode”.
- One overview card + at most two sibling operational cards. Do not wrap every field group in its own card.
- Footer owns the only primary Save. Nested cards must not ship a second primary Save when the dialog already has a global Save (`hideSave` / draft pattern as in API Key rate limits).
- Sibling cards need explicit `gap-4` and should not collapse when one side has less content.
- Long IDs / names wrap with `break-all` / `break-words`; mono only for secrets and technical tokens.
- Reserve height for post-selection details so validation and strategy panels do not jitter the dialog.

Do not:

- Turn the detail dialog into a multi-step wizard (wizards belong to create flows).
- Duplicate page-level padding, large hero titles, or marketing-style vertical rhythm inside the dialog.
- Open detail from accidental row clicks when the product pattern is `⋯` → Edit.

High-density component behavior:

- Card headers must allow title, subtitle, badge, and summary metrics to wrap instead of colliding.
- Right-aligned metrics should live in `shrink-0` containers so long labels do not hide them.
- Dashboard and analytics cards prefer min-heights and internal scroll over opportunistic stretching.
- Equal-height dashboard rows use `h-full` plus internal scrolling so the band stays aligned.
- Empty states preserve enough height that surrounding grids do not collapse abruptly.

### Overlay accessibility

Shared `Dialog` / `AlertDialog` / `Sheet` must:

1. Expose `role="dialog"` and `aria-modal="true"`, and associate title/description.
2. Move focus into the overlay on open and restore focus to the trigger on close.
3. Keep Tab / Shift+Tab focus inside the overlay while open.
4. Close on Escape and overlay click unless an explicitly documented exception applies.
5. Give icon-only actions an accessible name and Tooltip.
