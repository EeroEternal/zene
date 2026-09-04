# Surfaces

Elevation, depth, and shape language for Admin UI.

## Elevation & Depth

Depth is intentionally restrained. The admin UI should rely more on border, tone, and sectioning than on dramatic shadows.

- Standard cards and panels use a clear border and flat surface.
- Dialogs may use stronger elevation than inline cards, but only enough to separate modal context from the page.
- Selected rows, active list items, and the current sidebar route use a light primary fill (`bg-primary/10`) plus medium weight. Do not use raised surfaces or theme-colored left-edge bars.
- Layering inside dashboards should come from tonal separation and spacing rhythm.

Depth rules:

- Do not stack multiple strong shadows within one view.
- Prefer border plus surface contrast before adding shadow.
- Floating hints, masks, or overlays must reserve interior space and must not cover the first readable line of card content.

## Shapes

The shape language is modest and engineered. Corners are soft enough to feel modern, but not so rounded that operational tooling starts to feel playful.

- Default radius is 8px.
- Small inner treatments may reduce to 4px or 6px when needed by nested elements.
- Pills and badges can use full rounding.
- Sharp corners and fully rounded controls should not be mixed randomly within the same screen.

Shape rules:

- Primary interactive controls, cards, dialogs, and inputs should all resolve through the same radius scale.
- If a component needs emphasis, do not invent a new corner language; use color, spacing, or layout instead.
