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
  { id: "dracula", name: "Dracula" },
  { id: "gruvbox", name: "Gruvbox" },
  { id: "catppuccin", name: "Catppuccin" },
  { id: "solarized", name: "Solarized" },
  { id: "github", name: "GitHub" },
];

// Android 12+ exposes a Material You (Monet) palette derived from the wallpaper.
// The "Dynamic" theme below is only meaningful there, so it's appended to
// PALETTES only on Android and the colors are applied at runtime (see below).
export const isAndroid = /Android/.test(navigator.userAgent);
if (isAndroid) PALETTES.push({ id: "dynamic", name: "Dynamic (Material You)" });

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
  { id: "inter", name: "Inter", stack: '"Inter", system-ui, sans-serif' },
  { id: "lora", name: "Lora", stack: '"Lora", Georgia, "Times New Roman", serif' },
];

const KEY = "vellum-theme";
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
// Reactive mirror of the OS preference so `dark` recomputes when it flips.
let systemDark = $state(mql.matches);

function resolvedDark(): boolean {
  return mode === "dark" || (mode === "system" && systemDark);
}

// --- Material You ("Dynamic" palette) -------------------------------------
// The native get_material_you command returns the device's Monet tonal palette
// as { a1_500: "#rrggbb", n1_900: ..., ... } (or null pre-Android-12). We fetch
// once, then map the tones onto our theme vars as inline styles on <html> (which
// outrank the [data-theme] CSS blocks). Mid-tone accents stand in for the syntax
// colors since Monet only exposes three accent ramps.
type Monet = Record<string, string>;
const MONET_KEY = "vellum-monet";
let monetPromise: Promise<Monet | null> | null = null;
function ensureMonet(): Promise<Monet | null> {
  if (!monetPromise) {
    monetPromise = invoke<string | null>("get_material_you")
      .then((s) => {
        const m = s ? (JSON.parse(s) as Monet) : null;
        // Cache the resolved light+dark var maps so index.html's pre-paint
        // script can apply Dynamic colors before first paint (no flash). Device
        // tones rarely change; this refresh keeps the cache current.
        if (m) {
          try {
            localStorage.setItem(
              MONET_KEY,
              JSON.stringify({ light: monetMap(m, false), dark: monetMap(m, true) }),
            );
          } catch {
            /* ignore quota/serialization */
          }
        }
        return m;
      })
      .catch(() => null);
  }
  return monetPromise;
}

// Every var the dynamic theme sets — also the list we clear when leaving it.
const DYN_VARS = [
  "--background", "--foreground", "--card", "--card-foreground", "--popover",
  "--popover-foreground", "--primary", "--primary-foreground", "--secondary",
  "--secondary-foreground", "--muted", "--muted-foreground", "--accent",
  "--accent-foreground", "--destructive", "--destructive-foreground", "--border",
  "--input", "--ring", "--editor-selection", "--editor-cursor", "--editor-code-bg",
  "--code-keyword", "--code-string", "--code-number", "--code-comment",
  "--code-function", "--code-type", "--code-variable", "--md-h2", "--md-h3",
  "--md-h4", "--md-h5", "--md-strong", "--md-em", "--md-quote",
];

function clearMonetVars(el: HTMLElement) {
  for (const v of DYN_VARS) el.style.removeProperty(v);
}

// Map Monet tones onto our theme vars (light or dark). Pure — also used to
// build the cached maps that index.html applies pre-paint.
function monetMap(m: Monet, dark: boolean): Record<string, string> {
  return dark
    ? {
        "--background": m.n1_900, "--foreground": m.n1_50,
        "--card": m.n1_800, "--card-foreground": m.n1_50,
        "--popover": m.n1_800, "--popover-foreground": m.n1_50,
        "--primary": m.a1_200, "--primary-foreground": m.n1_900,
        "--secondary": m.n1_800, "--secondary-foreground": m.n1_50,
        "--muted": m.n1_800, "--muted-foreground": m.n2_300,
        "--accent": m.a1_200, "--accent-foreground": m.n1_900,
        "--destructive": "#ff6b6b", "--destructive-foreground": m.n1_900,
        "--border": m.n2_700, "--input": m.n2_700, "--ring": m.a1_200,
        "--editor-selection": m.a1_200 + "38", "--editor-cursor": m.a1_200,
        "--editor-code-bg": m.n1_800,
        "--code-keyword": m.a1_200, "--code-string": m.a3_500,
        "--code-number": m.a2_500, "--code-comment": m.n2_300,
        "--code-function": m.a1_300, "--code-type": m.a3_500,
        "--code-variable": m.a2_500,
        "--md-h2": m.a1_200, "--md-h3": m.a1_300, "--md-h4": m.a2_500,
        "--md-h5": "#ff6b6b", "--md-strong": m.a3_500, "--md-em": m.a3_500,
        "--md-quote": m.a2_500,
      }
    : {
        "--background": m.n1_10, "--foreground": m.n1_900,
        "--card": m.n1_50, "--card-foreground": m.n1_900,
        "--popover": m.n1_50, "--popover-foreground": m.n1_900,
        "--primary": m.a1_600, "--primary-foreground": m.n1_10,
        "--secondary": m.n2_100, "--secondary-foreground": m.n1_900,
        "--muted": m.n2_100, "--muted-foreground": m.n2_700,
        "--accent": m.a1_600, "--accent-foreground": m.n1_10,
        "--destructive": "#c0392b", "--destructive-foreground": "#ffffff",
        "--border": m.n2_300, "--input": m.n2_300, "--ring": m.a1_600,
        "--editor-selection": m.a1_600 + "2e", "--editor-cursor": m.a1_600,
        "--editor-code-bg": m.n1_100,
        "--code-keyword": m.a1_600, "--code-string": m.a3_500,
        "--code-number": m.a2_500, "--code-comment": m.n2_700,
        "--code-function": m.a1_500, "--code-type": m.a3_500,
        "--code-variable": m.a2_500,
        "--md-h2": m.a1_600, "--md-h3": m.a1_500, "--md-h4": m.a2_500,
        "--md-h5": "#c0392b", "--md-strong": m.a3_500, "--md-em": m.a3_500,
        "--md-quote": m.a2_500,
      };
}

function applyMonetVars(el: HTMLElement, m: Monet | null, dark: boolean) {
  if (!m) {
    clearMonetVars(el);
    return;
  }
  const map = monetMap(m, dark);
  for (const [k, v] of Object.entries(map)) if (v) el.style.setProperty(k, v);
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
  // Dynamic (Material You): apply device tones as inline overrides; otherwise
  // clear them so the chosen static palette shows through.
  if (palette === "dynamic") {
    ensureMonet().then((m) => {
      if (palette === "dynamic") applyMonetVars(el, m, resolvedDark());
    });
  } else {
    clearMonetVars(el);
  }
  // Override the body typeface (Tailwind's --font-sans). "basic" clears it.
  const f = FONTS.find((x) => x.id === font);
  if (f && f.id !== "basic") el.style.setProperty("--font-sans", f.stack);
  else el.style.removeProperty("--font-sans");
}

function persist() {
  localStorage.setItem(KEY, JSON.stringify({ palette, mode, font }));
}

// Follow the OS when in "system" mode.
mql.addEventListener("change", (e) => {
  systemDark = e.matches;
  if (mode === "system") applyTheme();
});

export const theme = {
  // Resolved light/dark, reactive — for consumers (e.g. the CodeMirror theme)
  // that must reconfigure when the mode or OS preference changes.
  get dark() {
    return resolvedDark();
  },
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
