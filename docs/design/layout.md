# Layout

The layout system is built around a shared shell and consistent left-edge alignment. Page framing is structural, not decorative.

## Shell (visual spec v1.0)

Fixed **sidebar + top bar + fluid work area**. Design canvas 1920px; content width is not capped to a board size.

| Part | Size |
| --- | --- |
| Sidebar expanded | 256px (`--sidebar-width: 16rem`) |
| Sidebar collapsed | 48px |
| Top bar | 56px (`h-14`) |
| Menu row | 32px tall, 8px pad, 12px icon gap, 6px radius, 14/20 type |
| Page padding | 32px sides; 32–40px top; ≥32px bottom |
| Module gap | 24px |
| Card gap | 16px |
| Control gaps | 8 / 12 / 16 |

Spacing scale (4px base): **4, 8, 12, 16, 24, 32, 40, 48**. Prefer this ladder; do not invent 10px / 18px.

12-column grid; gutters 16px or 24px.

**Top bar does not carry business actions.** Left: sidebar trigger. Right: theme, language, org/project, notifications, user. Page primary actions stay in `PageHeader`.

Sidebar entries: `admin/src/lib/nav.ts` (or product equivalent). Do not invent a second menu.

## Page types

Pick one. Same tokens; different structure.

| Type | Structure | Use |
| --- | --- | --- |
| 数据看板型 | KPI cards → multi-column modules | Dashboard, analytics |
| 单栏表格型 | Filter/search → full-width table | Lists, batch ops |
| 列表+详情型 | List + detail pane | Object browse |
| 双栏目录型 | Directory + large content | Catalogs |
| 工作台型 | History/nav + working pane | Chat, continuous tasks |
| 混合信息型 | KPI → main module → auxiliaries | Health, billing |

A content page is: title + page actions → optional filter row → optional KPI row → main → optional auxiliary. Do not put filters in the top bar or next to the title.

- Pages use a common composition equivalent to `PageShell`, `PageHeader`, and `PageContainer`.
- The page title and the primary content block must share the same left boundary.
- Page headers belong inside the same content container as the page body, not floated into the global `SiteHeader` / banner.
- `PageHeader` is a single compact row: page title on the left; optional entity create / primary page actions on the right (one horizontal row, no wrap). Do not put search, filters, Auto Refresh, or Export in the header — those belong in the toolbar under the title. Do not put a manual Refresh button in the header; live views use an Auto Refresh (3s) toggle in the toolbar instead. Do not render a page-level or card-level subtitle under the title; instructional copy belongs in empty states, dialogs, or docs.
- Optional filters sit in a toolbar row directly under `PageHeader` (same pattern as logs, billing, and analytics): search / filters on the left, Auto Refresh / Export / Query on the right of that same row. Not inside a titled filter card, and not beside the page title.
- Cards are the default containment primitive for tables, forms, dashboards, detail panes, and empty states.

Spacing strategy:

- Base unit is 4px, but most operational spacing lands on 8px, 16px, 24px, and 32px.
- Content grouping should feel compact but never cramped; dense pages should compress vertical whitespace before compromising text readability.
- Fixed-height cards are discouraged unless the design explicitly reserves overflow handling. When a fixed-height card does manage spacing through explicit `CardHeader` and `CardContent` padding, remove the base `Card` vertical `gap` and top/bottom padding (`gap-0 py-0`) so nested metric blocks do not get squeezed against the card edges.
- When a dashboard or analytics row does reserve height, sibling cards in that same grid row must share a consistent height; do not let one card become visibly taller through content volume or opportunistic `row-span` usage.
- Desktop height tiers are explicit for admin analytics surfaces: dashboard monitoring cards use a 420px row, dashboard insight charts and analytics or breakdown cards use a 336px row, and nested metric tiles inside those cards use an 84px minimum height.
- Analytics and breakdown cards should reserve a shared 96px to 104px header block before content begins so scroll regions and nested tiles line up across sibling cards.
- Dashboard monitoring cards inside the reserved 420px row must participate in the full row height with `h-full`; compact summary cards may keep content top-aligned internally, but the card shell itself must not collapse shorter than adjacent monitoring cards.
- Nested metric tiles should reserve two lines for labels before the value block; do not let short labels collapse the tile and make adjacent cards look misaligned.
- When a tall operational card is followed by a row of smaller summary cards, do not rely on the default content gap alone; reserve at least 32px between the main card's bottom edge and the next card row so the summary section does not read as glued to the card above.
- For nested metric grids inside analytics or dashboard cards, size rows by content rather than fractional fill unless equal-height tiles are explicitly required; summary tiles should not be stretched just to consume leftover card height.

Containment rules:

- High-density cards should use `overflow-hidden` and `min-h-0` to prevent clipping and broken nested scrolling.
- When tables need horizontal scrolling, the scroll must stay inside the table container and must not widen the whole page or card.
- Two-column and panel layouts should collapse vertically when width is constrained rather than squeezing text into overflow.
- Titles should not be visually indented more than their first major table or list container.
- If content exceeds the reserved height of a card row, the overflow should be absorbed by the card's internal scroll region rather than stretching one card beyond its peers.

### Layout stability (no jitter)

Selection, expand/collapse, and validation must not shove the rest of the form or dialog up or down. Layout shift is a defect, especially in create wizards (API Keys, Routes).

Rules:

- Reserve space for secondary details that appear after a choice (for example route protocol/strategy under a route Select). Prefer a fixed `min-h-*` slot that always mounts; fill it with real content after selection, or with a short placeholder when empty. Do not mount/unmount the block only when a value is chosen.
- Strategy-dependent panels (smart routing prefs, CostAware toggles, advanced fields) live inside one always-present reserved region. Switching strategy swaps content inside that region; it must not insert a new block that grows the page from zero height.
- Multi-example / code switchers in Dialogs & Cards must use **fixed-height Tabs** with internal scrolling (`h-[132px]` or `min-h-*`) rather than uncoordinated expand/collapse accordions. Switching tabs must only swap text inside the fixed container and never resize the dialog or cause viewport jitter.
- The default tab must always prioritize the most modern/recommended workflow (e.g. URL path routing).
- Summary sidebars that mirror selected meta should reserve the same secondary lines (protocol / strategy) so the sticky summary card does not resize when the main form updates.
- Avoid conditional blocks that change overall dialog height on first interaction after open when those details are part of the primary path. Prefer placeholder height over `null` until the user chooses.
- Scroll containers (`overflow-y-auto` dialog bodies) must not jump scroll position when content height changes within the reserved slots.
- Validation errors may appear below actions, but should not push critical controls out of view without a reserved error strip when errors are common on that step.

Canonical references: `ApiKeyUsageExamples` (fixed-height tab switching for curl commands), `ApiKeyCreateWizard` route meta slot (`SelectedRouteMeta` with `min-h`), `RouteCreateWizard` strategy extras `min-h` region.
