# UI Entry & Visual Consistency

This document defines front-end UI visual consistency rules.

## Core Rules

1. **Strict Design Alignment**: Any visible page (including reports and landings) loads skill `admin-ui-change` and `docs/design.md`. Never add `index.html`.
2. **Semantic Colors**: Use HSL CSS variables (`--primary`, `--muted`, `--destructive`, `--background`, `--foreground`). Never use arbitrary ad-hoc Tailwind colors.
3. **Quiet Selection**: Highlight selected items via background tint (`bg-primary/10` or `bg-sidebar-accent`) and font weight. Never use bright glowing borders or accent indicator bars.
4. **Layout Stability**: Reserve minimum height for dynamic components to prevent layout jitter.
