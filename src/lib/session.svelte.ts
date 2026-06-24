// Persisted UI session: the last vault + note the user had open, and whether the
// sidebar/drawer was open. Restored on launch so the app reopens where you left
// off. Mirrors theme.svelte.ts (localStorage + $state + getters/setters).

const KEY = "notes-session";

type Mode = "source" | "preview";
let saved: { vault?: string | null; path?: string | null; mode?: Mode } = {};
try {
  saved = JSON.parse(localStorage.getItem(KEY) || "{}");
} catch {
  /* ignore malformed */
}

let vault = $state<string | null>(saved.vault ?? null);
let path = $state<string | null>(saved.path ?? null);
let mode = $state<Mode>(saved.mode ?? "source");

function persist() {
  localStorage.setItem(KEY, JSON.stringify({ vault, path, mode }));
}

export const session = {
  get vault() {
    return vault;
  },
  set vault(v: string | null) {
    vault = v;
    persist();
  },
  get path() {
    return path;
  },
  set path(v: string | null) {
    path = v;
    persist();
  },
  get mode() {
    return mode;
  },
  set mode(v: Mode) {
    mode = v;
    persist();
  },
};
