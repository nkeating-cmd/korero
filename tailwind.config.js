/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Core
        text: "var(--color-text)",
        "text-muted": "var(--color-text-muted)",
        "text-subtle": "var(--color-text-subtle)",
        background: "var(--color-background)",
        "background-elev": "var(--color-background-elev)",

        // Glass surfaces
        "glass-surface": "var(--color-glass-surface)",
        "glass-surface-hover": "var(--color-glass-surface-hover)",
        "glass-border": "var(--color-glass-border)",
        "glass-hero-tint": "var(--color-glass-hero-tint)",

        // Brand
        "logo-primary": "var(--color-logo-primary)",
        "logo-stroke": "var(--color-logo-stroke)",
        "text-stroke": "var(--color-text-stroke)",
        "accent-yellow": "var(--color-accent-yellow)",
        "accent-purple": "var(--color-accent-purple)",

        // Status pills
        "pill-warning": "var(--color-pill-warning)",
        "pill-urgent": "var(--color-pill-urgent)",
        "pill-positive": "var(--color-pill-positive)",
        "pill-info": "var(--color-pill-info)",

        // Backwards-compat (Handy upstream class names)
        "mid-gray": "var(--color-mid-gray)",
      },
      borderRadius: {
        glass: "16px",
        "glass-hero": "20px",
        "glass-nested": "12px",
      },
      backdropBlur: {
        glass: "20px",
        "glass-hero": "24px",
      },
    },
  },
  plugins: [],
};
