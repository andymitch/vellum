// The open-notes list behind the editor tab strip (#169).
//
// Kept pure and separate from `session.svelte.ts` (which owns the reactive
// state and localStorage) so the rules — which tab a click reuses, what happens
// to a tab whose note is renamed or deleted elsewhere — are testable without a
// DOM. Every function returns a new list, or the *same* list when nothing
// changed: App's autosave effect pins the active tab as a side effect, and an
// identity change with no real change would re-fire it.

export type Mode = "source" | "preview";

export interface Tab {
  path: string;
  /** Source or preview, remembered per tab. */
  mode: Mode;
  /** Scroll position as a 0..1 ratio, so it maps across the two views. */
  scroll: number;
  /**
   * A *preview* tab is the one a single click in the sidebar reuses, shown in
   * italics. There is at most one; double-clicking, or editing the note, pins
   * it so the next click opens a tab of its own instead of replacing it.
   * (VS Code's model — it keeps browsing the tree from piling up tabs.)
   */
  preview: boolean;
}

/** `active` is -1 exactly when `tabs` is empty. */
export interface TabList {
  tabs: Tab[];
  active: number;
}

/**
 * An empty list. A function rather than a shared constant: this value is
 * assigned into reactive state, and one instance shared between the store and
 * every caller is an object anyone could mutate through — and one that reads
 * as a *different* identity from its own reactive proxy, which quietly breaks
 * the "returns the same list when nothing changed" contract above.
 */
export const noTabs = (): TabList => ({ tabs: [], active: -1 });

export const activeTab = (l: TabList): Tab | null => l.tabs[l.active] ?? null;

export const tabIndex = (l: TabList, path: string): number =>
  l.tabs.findIndex((t) => t.path === path);

const newTab = (path: string, mode: Mode, preview: boolean): Tab => ({
  path,
  mode,
  scroll: 0,
  preview,
});

export interface OpenOpts {
  /** Open pinned — a double-click in the tree, or a note we just created. */
  pin?: boolean;
  /** Force a tab of its own (Cmd-click / middle-click) rather than reusing one. */
  newTab?: boolean;
  /** View the new tab opens in. Callers pass the current one, so opening a note
   *  while reading keeps you in preview. */
  mode?: Mode;
}

/**
 * Open `path`, reusing a tab per the preview-tab rules above. Already-open
 * notes are focused rather than opened twice, so the list never holds a path
 * more than once.
 */
export function openTab(l: TabList, path: string, opts: OpenOpts = {}): TabList {
  const mode = opts.mode ?? "source";
  const already = tabIndex(l, path);
  if (already !== -1) {
    // Already open: focus it. A double-click also pins it — that is how you
    // promote the tab you're browsing in.
    const focused = activate(l, already);
    return opts.pin ? pin(focused, already) : focused;
  }
  const fresh = newTab(path, mode, !opts.pin);
  if (opts.newTab) {
    // Next to the tab it was opened from, like a browser — and pinned, however
    // it was asked for: wanting a tab of its own is the same intent pinning
    // serves, and a second preview tab would break the one-at-a-time rule the
    // reuse below depends on.
    const at = l.active + 1;
    const tabs = [...l.tabs.slice(0, at), { ...fresh, preview: false }, ...l.tabs.slice(at)];
    return { tabs, active: at };
  }
  // Reuse the preview tab if there is one — the active one for preference, so
  // an unpinned tab is replaced where the user is looking.
  const reuse = activeTab(l)?.preview ? l.active : l.tabs.findIndex((t) => t.preview);
  if (reuse !== -1) {
    const tabs = [...l.tabs];
    tabs[reuse] = fresh;
    return { tabs, active: reuse };
  }
  const at = l.active + 1;
  return { tabs: [...l.tabs.slice(0, at), fresh, ...l.tabs.slice(at)], active: at };
}

export function activate(l: TabList, i: number): TabList {
  if (i === l.active || i < 0 || i >= l.tabs.length) return l;
  return { tabs: l.tabs, active: i };
}

export function pin(l: TabList, i: number): TabList {
  const t = l.tabs[i];
  if (!t || !t.preview) return l;
  const tabs = [...l.tabs];
  tabs[i] = { ...t, preview: false };
  return { tabs, active: l.active };
}

/** Close one tab, focusing its right-hand neighbour (or its left, at the end). */
export function closeTab(l: TabList, i: number): TabList {
  if (i < 0 || i >= l.tabs.length) return l;
  const tabs = l.tabs.filter((_, n) => n !== i);
  if (!tabs.length) return noTabs();
  // Closing a tab left of the active one shifts it down; closing the active one
  // keeps the index (the neighbour on the right slides in) until it was last.
  const active = i < l.active ? l.active - 1 : Math.min(l.active, tabs.length - 1);
  return { tabs, active };
}

/** Patch the active tab — its view mode, or its scroll position. */
export function updateActive(l: TabList, patch: Partial<Omit<Tab, "path">>): TabList {
  const t = activeTab(l);
  if (!t) return l;
  const next = { ...t, ...patch };
  if (next.mode === t.mode && next.scroll === t.scroll && next.preview === t.preview) return l;
  const tabs = [...l.tabs];
  tabs[l.active] = next;
  return { tabs, active: l.active };
}

const under = (path: string, dir: string) => path.startsWith(dir + "/");

/**
 * Follow a rename (of a note, or of a folder holding open notes) so a tab that
 * isn't the active one doesn't keep pointing at a path that no longer exists.
 * If the destination is already open, the tabs collapse into one.
 */
export function renamePaths(l: TabList, from: string, to: string, isDir: boolean): TabList {
  const moved = (p: string) =>
    isDir ? (p === from || under(p, from) ? to + p.slice(from.length) : p) : p === from ? to : p;
  if (!l.tabs.some((t) => moved(t.path) !== t.path)) return l;
  const activePath = activeTab(l)?.path;
  const tabs: Tab[] = [];
  for (const t of l.tabs) {
    const path = moved(t.path);
    // A rename onto an already-open path leaves one note in two tabs; keep the
    // first and drop the duplicate.
    if (tabs.some((k) => k.path === path)) continue;
    tabs.push(path === t.path ? t : { ...t, path });
  }
  const want = activePath === undefined ? -1 : tabs.findIndex((t) => t.path === moved(activePath));
  return { tabs, active: want === -1 ? Math.min(l.active, tabs.length - 1) : want };
}

/** Drop tabs whose note was deleted (or that sat inside a deleted folder). */
export function removePaths(l: TabList, path: string, isDir: boolean): TabList {
  const gone = (p: string) => (isDir ? p === path || under(p, path) : p === path);
  return pruneTabs(l, (p) => !gone(p));
}

/**
 * Keep only tabs whose notes exist — used on launch, where a note in the
 * restored list may have been deleted on another device since.
 */
export function pruneTabs(l: TabList, exists: (path: string) => boolean): TabList {
  if (l.tabs.every((t) => exists(t.path))) return l;
  const activePath = activeTab(l)?.path;
  const tabs = l.tabs.filter((t) => exists(t.path));
  if (!tabs.length) return noTabs();
  const want = tabs.findIndex((t) => t.path === activePath);
  return { tabs, active: want === -1 ? Math.min(l.active, tabs.length - 1) : want };
}

/**
 * Validate a persisted list. Anything malformed is dropped rather than trusted:
 * this is user-editable localStorage, and a bad `active` index or a duplicated
 * path would break invariants the rest of the module relies on.
 */
export function normalize(raw: unknown): TabList {
  if (!raw || typeof raw !== "object") return noTabs();
  const { tabs, active } = raw as { tabs?: unknown; active?: unknown };
  if (!Array.isArray(tabs)) return noTabs();
  const seen = new Set<string>();
  const clean: Tab[] = [];
  for (const t of tabs) {
    if (!t || typeof t !== "object") continue;
    const { path, mode, scroll, preview } = t as Record<string, unknown>;
    if (typeof path !== "string" || !path || seen.has(path)) continue;
    seen.add(path);
    clean.push({
      path,
      mode: mode === "preview" ? "preview" : "source",
      scroll: typeof scroll === "number" && scroll >= 0 && scroll <= 1 ? scroll : 0,
      preview: preview === true,
    });
  }
  if (!clean.length) return noTabs();
  const i = typeof active === "number" ? Math.trunc(active) : 0;
  return { tabs: clean, active: i >= 0 && i < clean.length ? i : 0 };
}
