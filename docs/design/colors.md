# Colors

The palette is semantic first. Product code should consume `primary`, `secondary`, `muted`, `destructive`, `success`, `warning`, `info`, `inactive`, `experimental`, `card`, `background`, and related foreground tokens instead of hard-coded utility colors.

- Brand `#2744A5` comes from the canonical X-series SVG and is the stable identity color for logos and brand marks.
- Primary uses brand blue in light mode and an accessible lighter blue in dark mode for important actions, selected states, links, and focused emphasis. Interactive shades may differ from the logo color to preserve contrast.
- Selection fill uses a light primary tint (`bg-primary/10` or `primary-light`), not a near-white muted gray and not a theme-colored left border.
- Neutral surfaces stay light and quiet: `background`, `card`, `secondary`, and `muted` should carry most of the interface.
- Feedback colors are explicit and not interchangeable: `success` for healthy states, `warning` for caution, `destructive` for failure or irreversible actions.
- `info` communicates processing and neutral operational information, `inactive` communicates stopped or offline states, and `experimental` is reserved for beta or AI-assisted capabilities.
- Dark mode uses the parallel `dark-*` token set and should preserve the same hierarchy rather than inventing a new palette.

The enterprise spec defines the palette as **semantic swatches + alpha**, not a 50–900 ramp. Hex lives in [`tokens.md`](tokens.md); pages use token names and `/10` `/15` `/20`.

| Token | Role |
| --- | --- |
| Primary `#2744A5` | Buttons, selected fill, brand emphasis |
| Text / Primary `#22222A` | Titles, body |
| Text / Secondary `#71717A` | Descriptions, meta |
| Background / Card `#FFFFFF` | Page and card surfaces |
| Muted `#F4F4F5` | Content well, sidebar |
| Border `#E4E4E7` | Borders, inputs, dividers |
| Success `#21C45D` | Healthy / enabled |
| Destructive `#EF4343` | Error / danger / delete |

Alpha (light mode):

| Color | 10% | 15% | 20% |
| --- | --- | --- | --- |
| Primary | Selected fill, menu active (`bg-primary/10`) | — | Focus ring |
| Success | Badge fill | Hover | Focus ring |
| Destructive | Badge fill | — | Focus ring |

DevTools-measured hover hex on a live page (for example secondary border `#D9D9D9`, switch-on `#254AC5`) is **not** a new token. Follow the semantic names above.

Usage rules:

- Do not introduce page-level hex colors, `bg-violet-*`, `bg-gray-*`, or `text-red-*` in product screens.
- Avoid using the primary color as decoration. It should mark the current action, selection, or meaningful emphasis.
- Do not recolor the X-series SVG through page-level CSS. Use the canonical asset and its original brand fill.
- Error panels use low-emphasis tinted containers with destructive text, not fully saturated red blocks.
- Sidebar surfaces should stay slightly separated from content surfaces through neutral tonal change, not heavy shadow.
