import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}", "./lib/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#FFFFFF",
        "canvas-bg": "#FCFCFD",
        secondary: "#F9F9FB",
        tertiary: "#FCFCFD",
        active: "#E6F4FE",
        ink: "#1C2024",
        "ink-hover": "#242938",
        muted: "#60646C",
        placeholder: "#80858F",
        line: "#E6E8EB",
        "line-strong": "#DFE3E8",
        primary: "#0090FF",
        "primary-hover": "#0588F0",
        tint: "#E6F4FE",
        ok: "#1B7F3A",
        "ok-soft": "#E0FFE0",
        warn: "#8A6D1B",
        "warn-soft": "#FEF8E3",
        "warn-ink": "#8A6D1B",
        danger: "#C23B32",
        "danger-soft": "#FEE8E7",
        "danger-line": "#F0C9C4",
        info: "#E8F0FE",
      },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"],
        mono: ["JetBrains Mono", "Menlo", "Monaco", "Consolas", "Liberation Mono", "monospace"],
      },
      borderRadius: {
        sm: "4px",
        DEFAULT: "8px",
        md: "8px",
        lg: "12px",
        xl: "16px",
      },
      boxShadow: {
        card: "0 1px 2px rgba(0,0,0,.05)",
        menu: "0 8px 24px rgba(0,0,0,.12)",
      },
    },
  },
  plugins: [],
};

export default config;
