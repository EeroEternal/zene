import type { Config } from "tailwindcss";

const config: Config = {
  darkMode: ["class"],
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}", "./lib/**/*.{ts,tsx}"],
  theme: {
    container: {
      center: true,
      padding: "2rem",
      screens: {
        "2xl": "1400px",
      },
    },
    extend: {
      colors: {
        // console-kit semantic HSL tokens
        brand: "hsl(var(--brand))",
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
          hover: "hsl(var(--primary-hover))",
          active: "hsl(var(--primary-active))",
          light: "hsl(var(--primary-light))",
          dark: "hsl(var(--primary-dark))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        success: {
          DEFAULT: "hsl(var(--success))",
          foreground: "hsl(var(--success-foreground))",
        },
        warning: {
          DEFAULT: "hsl(var(--warning))",
          foreground: "hsl(var(--warning-foreground))",
        },
        info: "hsl(var(--info))",
        inactive: "hsl(var(--inactive))",
        experimental: "hsl(var(--experimental))",
        code: {
          DEFAULT: "hsl(var(--code-background))",
          border: "hsl(var(--code-border))",
          foreground: "hsl(var(--code-foreground))",
          muted: "hsl(var(--code-muted))",
          blue: "hsl(var(--code-blue))",
          cyan: "hsl(var(--code-cyan))",
          green: "hsl(var(--code-green))",
          yellow: "hsl(var(--code-yellow))",
          orange: "hsl(var(--code-orange))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        sidebar: {
          DEFAULT: "hsl(var(--sidebar-background))",
          foreground: "hsl(var(--sidebar-foreground))",
          primary: "hsl(var(--sidebar-primary))",
          "primary-foreground": "hsl(var(--sidebar-primary-foreground))",
          accent: "hsl(var(--sidebar-accent))",
          "accent-foreground": "hsl(var(--sidebar-accent-foreground))",
          border: "hsl(var(--sidebar-border))",
          ring: "hsl(var(--sidebar-ring))",
        },

        // Backward compatibility mappings for existing workbench classes
        canvas: "hsl(var(--card))",
        "canvas-bg": "hsl(var(--background))",
        nav: "hsl(var(--sidebar-background))",
        tertiary: "hsl(var(--muted))",
        active: "hsl(var(--primary-light))",
        ink: "hsl(var(--foreground))",
        "ink-hover": "hsl(var(--foreground))",
        placeholder: "hsl(var(--muted-foreground))",
        line: "hsl(var(--border))",
        "line-strong": "hsl(var(--border))",
        tint: "hsl(var(--primary-light))",
        ok: "hsl(var(--success))",
        "ok-soft": "hsl(var(--success-foreground))",
        warn: "hsl(var(--warning))",
        "warn-soft": "hsl(var(--warning-foreground))",
        "warn-ink": "hsl(var(--warning))",
        danger: "hsl(var(--destructive))",
        "danger-soft": "hsl(var(--destructive) / 0.1)",
        "danger-line": "hsl(var(--destructive) / 0.25)",
        chip: "hsl(var(--muted))",
      },
      spacing: {
        "sidebar-expanded": "256px",
        "sidebar-collapsed": "48px",
        topbar: "56px",
      },
      fontSize: {
        metric: ["20px", { lineHeight: "28px", fontWeight: "600" }],
        "page-title": ["20px", { lineHeight: "28px", fontWeight: "600" }],
        "section-title": ["16px", { lineHeight: "24px", fontWeight: "600" }],
        "body-md": ["14px", { lineHeight: "20px", fontWeight: "400" }],
        "label-sm": ["12px", { lineHeight: "20px", fontWeight: "500" }],
        "meta-sm": ["12px", { lineHeight: "16px", fontWeight: "400" }],
      },
      fontFamily: {
        sans: [
          "Inter",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI",
          "Roboto",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "Monaco",
          "Consolas",
          "monospace",
        ],
      },
      borderRadius: {
        none: "0px",
        sm: "calc(var(--radius) - 4px)",
        DEFAULT: "calc(var(--radius) - 2px)",
        md: "calc(var(--radius) - 2px)",
        lg: "var(--radius)",
        xl: "calc(var(--radius) + 2px)",
        menu: "6px",
        card: "8px",
        badge: "4px",
      },
      boxShadow: {
        card: "0 1px 2px rgba(0, 0, 0, 0.05)",
        menu: "0 1px 3px rgba(0, 0, 0, 0.08), 0 4px 12px rgba(0, 0, 0, 0.06)",
      },
      transitionDuration: {
        DEFAULT: "150ms",
      },
    },
  },
  plugins: [],
};

export default config;
