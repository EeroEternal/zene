---
version: alpha
name: Admin UI
description: Unified UI design specification for the admin console, aligned with the admin visual guidelines and implemented semantic tokens.
colors:
  brand: "#2744A5"
  background: "#FFFFFF"
  foreground: "#22222A"
  card: "#FFFFFF"
  card-foreground: "#22222A"
  popover: "#FFFFFF"
  popover-foreground: "#22222A"
  primary: "#2744A5"
  primary-foreground: "#FFFFFF"
  primary-hover: "#1F3A89"
  primary-active: "#182F70"
  primary-light: "#E8F0FF"
  secondary: "#F4F4F5"
  secondary-foreground: "#454554"
  muted: "#F4F4F5"
  muted-foreground: "#71717A"
  accent: "#F4F4F5"
  accent-foreground: "#22222A"
  destructive: "#EF4343"
  destructive-foreground: "#FFFFFF"
  success: "#21C45D"
  success-foreground: "#E9FBF0"
  warning: "#FFA71A"
  warning-foreground: "#FFF5E5"
  info: "#3B82F6"
  inactive: "#64748B"
  experimental: "#7C3AED"
  border: "#E4E4E7"
  input: "#E4E4E7"
  sidebar-background: "#F4F4F5"
  sidebar-foreground: "#71717A"
  sidebar-primary: "#2744A5"
  sidebar-primary-foreground: "#FFFFFF"
  sidebar-accent: "#EAEAEC"
  sidebar-accent-foreground: "#22222A"
  sidebar-border: "#E4E4E7"
  sidebar-ring: "#2744A5"
  dark-background: "#09090B"
  dark-foreground: "#F2F2F2"
  dark-card: "#0E0E11"
  dark-card-foreground: "#F2F2F2"
  dark-secondary: "#242428"
  dark-secondary-foreground: "#CCCCCC"
  dark-muted: "#242428"
  dark-muted-foreground: "#878792"
  dark-primary: "#8AA4FF"
  dark-primary-foreground: "#0F172A"
  dark-accent: "#242428"
  dark-accent-foreground: "#F2F2F2"
  dark-destructive: "#DF3A3A"
  dark-destructive-foreground: "#FFFFFF"
  dark-border: "#2C2C30"
  dark-input: "#2C2C30"
  dark-sidebar-background: "#0E0E11"
  dark-sidebar-foreground: "#878792"
  dark-sidebar-accent: "#1D1D20"
  dark-sidebar-accent-foreground: "#F2F2F2"
  dark-sidebar-border: "#2C2C30"
typography:
  page-title:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 20px
    fontWeight: 600
    lineHeight: 28px
  section-title:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 16px
    fontWeight: 600
    lineHeight: 24px
  table-header:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 14px
    fontWeight: 500
    lineHeight: 20px
  body-sm:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 14px
    fontWeight: 400
    lineHeight: 20px
  label-md:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 14px
    fontWeight: 500
    lineHeight: 20px
  label-sm:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 12px
    fontWeight: 500
    lineHeight: 20px
  secondary:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 12px
    fontWeight: 400
    lineHeight: 16px
  metric:
    fontFamily: ui-sans-serif, system-ui, sans-serif
    fontSize: 20px
    fontWeight: 600
    lineHeight: 28px
  mono-sm:
    fontFamily: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.5
rounded:
  none: 0px
  sm: 4px
  md: 6px
  lg: 8px
  full: 9999px
spacing:
  xs: 4px
  sm: 8px
  inner: 12px
  md: 16px
  lg: 24px
  xl: 32px
  2xl: 40px
  3xl: 48px
  page-padding: 32px
  content-gap: 24px
  card-gap: 16px
  table-min-width: 760px
shell:
  sidebar-expanded: 256px
  sidebar-collapsed: 48px
  topbar: 56px
  menu-row: 32px
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
    rounded: "{rounded.md}"
    padding: 12px 8px
    height: 40px
    typography: "{typography.label-md}"
  button-secondary:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.md}"
    padding: 12px 8px
    height: 40px
    typography: "{typography.label-md}"
  button-destructive:
    backgroundColor: "{colors.destructive}"
    textColor: "{colors.destructive-foreground}"
    rounded: "{rounded.md}"
    padding: 12px 8px
    height: 40px
    typography: "{typography.label-md}"
  input-default:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.sm}"
    padding: 12px
    height: 40px
    typography: "{typography.body-sm}"
  select-default:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.lg}"
    height: 32px
    typography: "{typography.body-sm}"
  switch-default:
    track: 36px 20px
    thumb: 16px
    rounded: 10px
  card-default:
    backgroundColor: "{colors.card}"
    textColor: "{colors.card-foreground}"
    rounded: "{rounded.lg}"
    padding: "{spacing.md}"
    typography: "{typography.body-sm}"
  dialog-default:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.lg}"
    padding: "{spacing.lg}"
  badge-success:
    backgroundColor: "{colors.success-foreground}"
    textColor: "{colors.success}"
    rounded: "{rounded.full}"
    padding: 8px
    typography: "{typography.label-sm}"
  badge-warning:
    backgroundColor: "{colors.warning-foreground}"
    textColor: "{colors.warning}"
    rounded: "{rounded.full}"
    padding: 8px
    typography: "{typography.label-sm}"
  badge-destructive:
    backgroundColor: "{colors.destructive-foreground}"
    textColor: "{colors.destructive}"
    rounded: "{rounded.full}"
    padding: 8px
    typography: "{typography.label-sm}"
---

# Design tokens

Canonical semantic token table for Admin UI (enterprise visual spec v1.0). Product code should consume these names via CSS/Tailwind, not hard-coded hex in pages.

Narrative: [colors.md](colors.md) · [typography.md](typography.md) · [layout.md](layout.md) · [components.md](components.md).

Alpha: primary/success/destructive at 10% fill, 15% status hover, 20% focus ring. Quiet selection is `bg-primary/10`.
