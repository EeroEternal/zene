import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./app/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}", "./lib/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#FFFFFF",
        secondary: "#F4F5F6",
        tertiary: "#FAFBFC",
        active: "#E8EAED",
        ink: "#202124",
        "ink-hover": "#3C4043",
        muted: "#5F6368",
        placeholder: "#9AA0A6",
        line: "#E8EAED",
        "line-strong": "#DADCE0",
        ok: "#4A7C59",
        "ok-soft": "#E8F5E9",
        warn: "#E8B339",
        "warn-soft": "#FFF8E8",
        "warn-ink": "#8A6D1B",
        danger: "#C06C5D",
        "danger-soft": "#FDECEA",
        "danger-line": "#F0C9C4",
      },
      fontFamily: {
        sans: ["Inter", "ui-sans-serif", "-apple-system", "BlinkMacSystemFont", "Segoe UI", "sans-serif"],
        mono: ["Menlo", "Monaco", "Consolas", "Liberation Mono", "monospace"],
      },
      boxShadow: {
        card: "0 1px 2px rgba(32, 33, 36, .06)",
        menu: "0 8px 28px rgba(32, 33, 36, .12)",
      },
    },
  },
  plugins: [],
};

export default config;
