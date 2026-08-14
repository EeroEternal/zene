import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}", "./lib/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#FFFFFF",
        "canvas-bg": "#F6F5F4",
        nav: "#EBE9E7",
        secondary: "#EBE9E7",
        tertiary: "#F0EFED",
        active: "#EAF2FF",
        ink: "#2E3436",
        "ink-hover": "#241F1C",
        muted: "#687174",
        placeholder: "#8A8F91",
        line: "#DDDCD9",
        "line-strong": "#C8C6C2",
        primary: "#3584E4",
        "primary-hover": "#1C71D8",
        tint: "#EAF2FF",
        ok: "#287A46",
        "ok-soft": "#EAF4EC",
        warn: "#E5A50A",
        "warn-soft": "#FFF5D6",
        "warn-ink": "#8A6D1B",
        danger: "#C01C28",
        "danger-soft": "#FCEBEC",
        "danger-line": "#E8B4B8",
        info: "#EAF2FF",
        chip: "#E4E2E0",
      },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"],
        mono: [
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "Monaco",
          "Consolas",
          "JetBrains Mono",
          "monospace",
        ],
      },
      borderRadius: {
        sm: "4px",
        DEFAULT: "6px",
        md: "6px",
        lg: "8px",
        xl: "8px",
      },
      boxShadow: {
        card: "0 1px 2px rgba(46, 52, 54, 0.08)",
        menu: "0 1px 2px rgba(46, 52, 54, 0.08), 0 4px 12px rgba(46, 52, 54, 0.10)",
      },
      transitionDuration: {
        DEFAULT: "150ms",
      },
    },
  },
  plugins: [],
};

export default config;
