// Persisted UI session: the last vault + note the user had open, and whether the
// sidebar/drawer was open. Restored on launch so the app reopens where you left
// off. Mirrors theme.svelte.ts (localStorage + $state + getters/setters).

const KEY = "notes-session";

type Mode = "source" | "preview";
let saved: {
  vault?: string | null;
  path?: string | null;
  mode?: Mode;
  scroll?: number;
} = {};
try {
  saved = JSON.parse(localStorage.getItem(KEY) || "{}");
} catch {
  /* ignore malformed */
}

let vault = $state<string | null>(saved.vault ?? null);
let path = $state<string | null>(saved.path ?? null);
let mode = $state<Mode>(saved.mode ?? "source");
// Scroll position of the open note, as a 0..1 ratio (so it maps across the
// source/preview views, which have different heights). Restored on launch.
let scroll = $state<number>(saved.scroll ?? 0);

function persist() {
  localStorage.setItem(KEY, JSON.stringify({ vault, path, mode, scroll }));
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
  get scroll() {
    return scroll;
  },
  set scroll(v: number) {
    scroll = v;
    persist();
  },
};
