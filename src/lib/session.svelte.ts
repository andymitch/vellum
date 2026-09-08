// Persisted UI session: the vault the user had open, the notes open in the
// editor's tabs (#169), and per-tab view mode + scroll. Restored on launch so
// the app reopens where you left off. Mirrors theme.svelte.ts (localStorage +
// $state + getters/setters).
//
// The tab-list rules live in `$lib/tabs` — pure and tested there. This module
// is the reactive, persisted holder for one of those lists, plus the vault it
// belongs to.

import {
  activate,
  activeTab,
  closeTab,
  moveTab,
  noTabs,
  normalize,
  openTab,
  pin,
  pruneTabs,
  removePaths,
  renamePaths,
  updateActive,
  type Mode,
  type OpenOpts,
  type TabList,
} from "./tabs";

const KEY = "vellum-session";

type Saved = {
  vault?: string | null;
  tabs?: unknown;
  active?: unknown;
  mode?: Mode;
  // Pre-#169 single-note session, migrated below.
  path?: string | null;
  scroll?: number;
};

let saved: Saved = {};
try {
  saved = JSON.parse(localStorage.getItem(KEY) || "{}");
} catch {
  /* ignore malformed */
}

// A session saved before tabs existed holds one note. Carry it over as a single
// pinned tab, so upgrading reopens exactly the note (and scroll) it would have.
function restore(s: Saved): TabList {
  if (s.tabs !== undefined) return normalize(s);
  if (!s.path) return noTabs();
  return normalize({
    tabs: [{ path: s.path, mode: s.mode, scroll: s.scroll, preview: false }],
    active: 0,
  });
}

let vault = $state<string | null>(saved.vault ?? null);
let list = $state<TabList>(restore(saved));
// The view mode a tab opens in when there is no tab to inherit from — i.e. the
// last one the user chose. Persisted so launch reopens in the mode they left.
let lastMode = $state<Mode>(saved.mode === "preview" ? "preview" : "source");

function persist() {
  localStorage.setItem(
    KEY,
    JSON.stringify({ vault, tabs: list.tabs, active: list.active, mode: lastMode }),
  );
}

// Every mutator funnels through here: `tabs.ts` returns the same list when a
// call changes nothing, so an unchanged write costs neither a persist nor a
// reactive update.
function set(next: TabList) {
  if (next === list) return;
  list = next;
  persist();
}

export const session = {
  get vault() {
    return vault;
  },
  /** Switching vaults closes the tabs — they name notes in the old one. */
  set vault(v: string | null) {
    if (v !== vault) {
      vault = v;
      list = noTabs();
      persist();
    }
  },

  get tabs() {
    return list.tabs;
  },
  get active() {
    return list.active;
  },
  /** The active tab's note, or null when nothing is open. */
  get path() {
    return activeTab(list)?.path ?? null;
  },

  get mode(): Mode {
    return activeTab(list)?.mode ?? lastMode;
  },
  set mode(m: Mode) {
    lastMode = m;
    // Persist unconditionally: updateActive is a no-op with no tabs open (and
    // when the tab is already in this mode), but lastMode still moved.
    list = updateActive(list, { mode: m });
    persist();
  },

  get scroll() {
    return activeTab(list)?.scroll ?? 0;
  },
  set scroll(r: number) {
    set(updateActive(list, { scroll: r }));
  },

  /** Open a note in the tabs, following the preview-tab rules in `tabs.ts`. */
  open(path: string, opts: OpenOpts = {}) {
    set(openTab(list, path, { mode: session.mode, ...opts }));
  },
  activate(i: number) {
    set(activate(list, i));
  },
  close(i: number) {
    set(closeTab(list, i));
  },
  /** Reorder: the tab at `from` is dragged to index `to`. */
  move(from: number, to: number) {
    set(moveTab(list, from, to));
  },
  /** Promote the preview tab so the next sidebar click doesn't replace it. */
  pin(i: number) {
    set(pin(list, i));
  },
  pinActive() {
    set(pin(list, list.active));
  },
  /** A note (or a folder of them) was renamed — follow it in the tabs. */
  renamed(from: string, to: string, isDir: boolean) {
    set(renamePaths(list, from, to, isDir));
  },
  /** A note (or a folder of them) was deleted — drop its tabs. */
  removed(path: string, isDir: boolean) {
    set(removePaths(list, path, isDir));
  },
  /** Drop tabs for notes that no longer exist (launch restore). */
  prune(exists: (path: string) => boolean) {
    set(pruneTabs(list, exists));
  },
  closeAll() {
    set(noTabs());
  },
};

export type { Mode };
