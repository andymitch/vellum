// Theme state: a palette (color set) × a mode (light / dark / follow-system).
// Persisted to localStorage; applied by toggling `.dark` and a `data-theme`
// attribute on <html>. A matching inline script in index.html applies the saved
// choice before first paint to avoid a flash — keep the two in sync.

import { invoke } from "@tauri-apps/api/core";

export type Mode = "system" | "light" | "dark";
export type Palette = { id: string; name: string };
export type Font = { id: string; name: string; stack: string };

export const PALETTES: Palette[] = [
  { id: "things", name: "Things" },
  { id: "nord", name: "Nord" },
  { id: "rose-pine", name: "Rosé Pine" },
];

// Body/UI typeface. `stack` is assigned to the --font-sans CSS var on <html>.
// The "Vellum" wordmark keeps Fraunces regardless (it uses --font-vellum).
export const FONTS: Font[] = [
  {
    id: "basic",
    name: "Basic",
    stack:
      '-apple-system, BlinkMacSystemFont, "Inter", "Segoe UI", Roboto, Helvetica, Arial, sans-serif',
  },
  { id: "serif", name: "Serif", stack: '"Fraunces", Georgia, serif' },
  {
    id: "mono",
    name: "Mono",
    stack: '"Space Mono", "JetBrains Mono", ui-monospace, monospace',
  },
];

const KEY = "notes-theme";
const mql = window.matchMedia("(prefers-color-scheme: dark)");

let saved: { palette?: string; mode?: Mode; font?: string } = {};
try {
  saved = JSON.parse(localStorage.getItem(KEY) || "{}");
} catch {
  /* ignore malformed */
}

let palette = $state<string>(saved.palette ?? "things");
let mode = $state<Mode>(saved.mode ?? "system");
let font = $state<string>(saved.font ?? "basic");

function resolvedDark(): boolean {
  return mode === "dark" || (mode === "system" && mql.matches);
}

export function applyTheme() {
  const el = document.documentElement;
  // "things" is the default :root palette — no attribute needed.
  if (palette === "things") delete el.dataset.theme;
  else el.dataset.theme = palette;
  const dark = resolvedDark();
  el.classList.toggle("dark", dark);
  // Tell native (Android) to match system-bar icon contrast to the web theme.
  // Fire-and-forget; no-op on desktop and harmless if the backend isn't ready.
  invoke("set_dark_mode", { dark }).catch(() => {});
  // Override the body typeface (Tailwind's --font-sans). "basic" clears it.
  const f = FONTS.find((x) => x.id === font);
  if (f && f.id !== "basic") el.style.setProperty("--font-sans", f.stack);
  else el.style.removeProperty("--font-sans");
}

function persist() {
  localStorage.setItem(KEY, JSON.stringify({ palette, mode, font }));
}

// Follow the OS when in "system" mode.
mql.addEventListener("change", () => {
  if (mode === "system") applyTheme();
});

export const theme = {
  get palette() {
    return palette;
  },
  set palette(v: string) {
    palette = v;
    persist();
    applyTheme();
  },
  get mode() {
    return mode;
  },
  set mode(v: Mode) {
    mode = v;
    persist();
    applyTheme();
  },
  get font() {
    return font;
  },
  set font(v: string) {
    font = v;
    persist();
    applyTheme();
  },
};
