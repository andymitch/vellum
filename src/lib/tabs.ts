// The open-notes list behind the editor tab strip (#169).
//
// Kept pure and separate from `session.svelte.ts` (which owns the reactive
// state and localStorage) so the rules — which tab a click reuses, what happens
// to a tab whose note is renamed or deleted elsewhere — are testable without a
// DOM. Every function returns a new list, or the *same* list when nothing
// changed: App's autosave effect pins the active tab as a side effect, and an
// identity change with no real change would re-fire it.
//
// A tab holds one note, or two shown side by side — dropping a tab onto another
// joins them. The strip stays flat either way: a joined tab is one entry, not
// an editor group. Two panes is the limit; there is no nesting.

export type Mode = "source" | "preview";

/** One note in a tab, and how it is being looked at. */
export interface Pane {
  path: string;
  /** Source or preview, remembered per pane. */
  mode: Mode;
  /** Scroll position as a 0..1 ratio, so it maps across the two views. */
  scroll: number;
}

export interface Tab {
  /** One or two notes, left to right. */
  panes: Pane[];
  /** Which pane has the keyboard. Always 0 for a single-note tab. */
  focused: number;
  /** The left pane's share of the width, for a joined tab. */
  ratio: number;
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

/** A 50/50 split, and the value a single-note tab carries. */
export const EVEN = 0.5;
/** Neither pane may be dragged narrower than this share of the width. */
export const MIN_RATIO = 0.15;

/**
 * An empty list. A function rather than a shared constant: this value is
 * assigned into reactive state, and one instance shared between the store and
 * every caller is an object anyone could mutate through — and one that reads
 * as a *different* identity from its own reactive proxy, which quietly breaks
 * the "returns the same list when nothing changed" contract above.
 */
export const noTabs = (): TabList => ({ tabs: [], active: -1 });

export const activeTab = (l: TabList): Tab | null => l.tabs[l.active] ?? null;

/** The note the keyboard is in: the focused pane of the active tab. */
export function activePane(l: TabList): Pane | null {
  const t = activeTab(l);
  return t?.panes[t.focused] ?? null;
}

export const isJoined = (t: Tab): boolean => t.panes.length > 1;

/** Where a note is open, if it is: which tab, and which pane of it. */
export function findPath(l: TabList, path: string): { tab: number; pane: number } | null {
  for (let tab = 0; tab < l.tabs.length; tab++) {
    const pane = l.tabs[tab].panes.findIndex((p) => p.path === path);
    if (pane !== -1) return { tab, pane };
  }
  return null;
}

const newPane = (path: string, mode: Mode): Pane => ({ path, mode, scroll: 0 });

const newTab = (path: string, mode: Mode, preview: boolean): Tab => ({
  panes: [newPane(path, mode)],
  focused: 0,
  ratio: EVEN,
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
  const already = findPath(l, path);
  if (already) {
    // Already open: focus it, in whichever pane holds it. A double-click also
    // pins it — that is how you promote the tab you're browsing in.
    const focused = focusPane(activate(l, already.tab), already.tab, already.pane);
    return opts.pin ? pin(focused, already.tab) : focused;
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
  const current = activeTab(l);
  if (current && isJoined(current)) {
    // A joined tab is a deliberate arrangement of two notes: opening a third
    // replaces the note in the pane you were last in, rather than tearing the
    // split down or quietly becoming a new tab somewhere else.
    const panes = [...current.panes];
    panes[current.focused] = newPane(path, current.panes[current.focused].mode);
    const tabs = [...l.tabs];
    tabs[l.active] = { ...current, panes };
    return { tabs, active: l.active };
  }
  // Reuse the preview tab if there is one — the active one for preference, so
  // an unpinned tab is replaced where the user is looking.
  const reuse = current?.preview ? l.active : l.tabs.findIndex((t) => t.preview && !isJoined(t));
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

/** Move the keyboard to one side of a joined tab (clicking into that pane). */
export function focusPane(l: TabList, i: number, pane: number): TabList {
  const t = l.tabs[i];
  if (!t || pane < 0 || pane >= t.panes.length || t.focused === pane) return l;
  const tabs = [...l.tabs];
  tabs[i] = { ...t, focused: pane };
  return { tabs, active: l.active };
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

/**
 * Move the tab at `from` to index `to`, dragged along the strip. `to` is the
 * index it ends up at in the new list, and the *note* that was active stays
 * active wherever it lands — a drag rearranges the strip, it doesn't navigate.
 */
export function moveTab(l: TabList, from: number, to: number): TabList {
  if (from < 0 || from >= l.tabs.length) return l;
  const at = Math.max(0, Math.min(to, l.tabs.length - 1));
  if (at === from) return l;
  const activeTabRef = activeTab(l);
  const tabs = [...l.tabs];
  const [moved] = tabs.splice(from, 1);
  tabs.splice(at, 0, moved);
  const active = tabs.findIndex((t) => t === activeTabRef);
  return { tabs, active: active === -1 ? l.active : active };
}

/**
 * Drop the tab at `from` onto the tab at `to`: the two notes become one tab
 * showing both, left/right, with the dragged note on the right and the keyboard
 * in it. Only two single-note tabs can join — dropping onto a tab that is
 * already split is a no-op, since two panes is the limit.
 */
export function joinTabs(l: TabList, from: number, to: number): TabList {
  const a = l.tabs[to];
  const b = l.tabs[from];
  if (!a || !b || from === to || isJoined(a) || isJoined(b)) return l;
  const joined: Tab = {
    panes: [a.panes[0], b.panes[0]],
    focused: 1,
    ratio: EVEN,
    // A split is a deliberate arrangement; nothing should replace it on the
    // next click in the tree.
    preview: false,
  };
  const tabs = l.tabs.map((t, i) => (i === to ? joined : t)).filter((_, i) => i !== from);
  return { tabs, active: tabs.indexOf(joined) };
}

/**
 * Split a joined tab back into two, side by side in the strip, keeping the
 * note that had the keyboard active.
 */
export function splitApart(l: TabList, i: number): TabList {
  const t = l.tabs[i];
  if (!t || !isJoined(t)) return l;
  const [left, right] = t.panes;
  const singles: Tab[] = [left, right].map((p) => ({
    panes: [p],
    focused: 0,
    ratio: EVEN,
    preview: false,
  }));
  const tabs = [...l.tabs.slice(0, i), ...singles, ...l.tabs.slice(i + 1)];
  // Splitting the tab you're in keeps you in the note you had the keyboard in.
  // Splitting some *other* tab — its button is reachable on hover — must not
  // navigate: stay where you were, one index further along for the tab the
  // split just inserted.
  const active =
    l.active === i
      ? i + (t.focused === 1 ? 1 : 0)
      : l.active > i
        ? l.active + 1
        : l.active;
  return { tabs, active };
}

/** Drag the divider between a joined tab's panes. */
export function setRatio(l: TabList, i: number, ratio: number): TabList {
  const t = l.tabs[i];
  if (!t || !isJoined(t)) return l;
  const next = Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio));
  if (next === t.ratio) return l;
  const tabs = [...l.tabs];
  tabs[i] = { ...t, ratio: next };
  return { tabs, active: l.active };
}

/**
 * Patch one pane — its view mode, or its scroll. Addressed rather than implied,
 * because the pane that reports a scroll is not always the focused one: a
 * split's other side scrolls under the wheel without taking the keyboard.
 */
export function updatePane(
  l: TabList,
  tab: number,
  pane: number,
  patch: Partial<Omit<Pane, "path">>,
): TabList {
  const t = l.tabs[tab];
  const p = t?.panes[pane];
  if (!t || !p) return l;
  const next = { ...p, ...patch };
  if (next.mode === p.mode && next.scroll === p.scroll) return l;
  const panes = [...t.panes];
  panes[pane] = next;
  const tabs = [...l.tabs];
  tabs[tab] = { ...t, panes };
  return { tabs, active: l.active };
}

/** Patch the focused pane of the active tab. */
export function updateActive(l: TabList, patch: Partial<Omit<Pane, "path">>): TabList {
  const t = activeTab(l);
  if (!t) return l;
  return updatePane(l, l.active, t.focused, patch);
}

const under = (path: string, dir: string) => path.startsWith(dir + "/");

/**
 * Follow a rename (of a note, or of a folder holding open notes) so a pane that
 * isn't the focused one doesn't keep pointing at a path that no longer exists.
 * If the destination is already open, the panes collapse into one.
 */
export function renamePaths(l: TabList, from: string, to: string, isDir: boolean): TabList {
  const moved = (p: string) =>
    isDir ? (p === from || under(p, from) ? to + p.slice(from.length) : p) : p === from ? to : p;
  if (!l.tabs.some((t) => t.panes.some((p) => moved(p.path) !== p.path))) return l;
  const activePath = activePane(l)?.path;
  const seen = new Set<string>();
  const tabs: Tab[] = [];
  for (const t of l.tabs) {
    const panes: Pane[] = [];
    for (const p of t.panes) {
      const path = moved(p.path);
      // A rename onto an already-open path leaves one note in two places; keep
      // the first and drop the duplicate.
      if (seen.has(path)) continue;
      seen.add(path);
      panes.push(path === p.path ? p : { ...p, path });
    }
    if (panes.length) tabs.push(withPanes(t, panes, moved(t.panes[t.focused]?.path ?? "")));
  }
  // The active note is wherever the rename put it, not where it was.
  return {
    tabs,
    active: relocate(tabs, activePath === undefined ? undefined : moved(activePath), l.active),
  };
}

/** Drop panes whose note was deleted (or that sat inside a deleted folder). */
export function removePaths(l: TabList, path: string, isDir: boolean): TabList {
  const gone = (p: string) => (isDir ? p === path || under(p, path) : p === path);
  return pruneTabs(l, (p) => !gone(p));
}

/**
 * Keep only panes whose notes exist — used on launch, where a note in the
 * restored list may have been deleted on another device since. A joined tab
 * that loses one side stays open as a single note.
 */
export function pruneTabs(l: TabList, exists: (path: string) => boolean): TabList {
  if (l.tabs.every((t) => t.panes.every((p) => exists(p.path)))) return l;
  const activePath = activePane(l)?.path;
  const tabs: Tab[] = [];
  for (const t of l.tabs) {
    const panes = t.panes.filter((p) => exists(p.path));
    if (panes.length) tabs.push(withPanes(t, panes));
  }
  if (!tabs.length) return noTabs();
  return { tabs, active: relocate(tabs, activePath, l.active) };
}

/**
 * Rebuild a tab around a new pane list, keeping the keyboard in the note that
 * had it. `focusedPath` is that note's path *in the new list*, which a rename
 * has to supply — the tab's own copy still says where it used to be.
 */
function withPanes(t: Tab, panes: Pane[], focusedPath = t.panes[t.focused]?.path): Tab {
  if (panes.length === t.panes.length && panes.every((p, i) => p === t.panes[i])) return t;
  const focused = panes.findIndex((p) => p.path === focusedPath);
  return { ...t, panes, focused: focused === -1 ? 0 : focused };
}

/** Which tab holds `path` now, falling back to something in range. */
function relocate(tabs: Tab[], path: string | undefined, was: number): number {
  const want = path === undefined ? -1 : tabs.findIndex((t) => t.panes.some((p) => p.path === path));
  return want === -1 ? Math.min(was, tabs.length - 1) : want;
}

/** What a tab shows: the note's name, plus enough folder to tell it apart. */
export interface TabLabel {
  name: string;
  /** Empty unless another open note has the same name. */
  qualifier: string;
}

const stem = (path: string) => path.split("/").pop()!.replace(/\.md$/, "");

/**
 * Label every pane of every tab, qualifying the ones whose names collide with
 * the *shortest* trailing folder path that tells them apart — `journal/Notes`
 * and `projects/Notes` become "Notes · journal" and "Notes · projects", and a
 * deeper clash (`work/q3/Notes`, `home/q3/Notes`) grows to "work/q3" rather
 * than stopping at an ambiguous "q3". A note at the vault root is qualified
 * with "/", since it has no folder to name.
 *
 * Only collisions are qualified: the common case of distinct names stays a
 * bare filename, which is what a tab has room for. Returns one array per tab,
 * one entry per pane — a joined tab shows both names.
 */
export function tabLabels(tabs: Tab[]): TabLabel[][] {
  const labels = tabs.map((t) => t.panes.map((p) => ({ name: stem(p.path), qualifier: "" })));
  const flat = tabs.flatMap((t, i) => t.panes.map((p, j) => ({ path: p.path, i, j })));
  const byName = new Map<string, typeof flat>();
  for (const entry of flat) {
    const name = stem(entry.path);
    byName.set(name, [...(byName.get(name) ?? []), entry]);
  }
  for (const group of byName.values()) {
    if (group.length < 2) continue;
    // Folder segments above the filename, nearest first.
    const dirs = group.map((e) => e.path.split("/").slice(0, -1));
    const deepest = Math.max(...dirs.map((d) => d.length));
    for (let depth = 1; depth <= Math.max(deepest, 1); depth++) {
      const qualifiers = dirs.map((d) => d.slice(Math.max(0, d.length - depth)).join("/") || "/");
      group.forEach((e, n) => (labels[e.i][e.j].qualifier = qualifiers[n]));
      // Done as soon as this depth separates them; a deeper path would only be
      // noise. Two notes with the same name can't share a folder, so a depth
      // exists that works — unless one of them *is* at the root, which "/"
      // already distinguishes.
      if (new Set(qualifiers).size === qualifiers.length) break;
    }
  }
  return labels;
}

/**
 * Validate a persisted list. Anything malformed is dropped rather than trusted:
 * this is user-editable localStorage, and a bad `active` index or a duplicated
 * path would break invariants the rest of the module relies on.
 *
 * Reads the pre-split shape too — a tab was one `{path, mode, scroll}` before
 * panes existed — so an upgrade reopens what it had.
 */
export function normalize(raw: unknown): TabList {
  if (!raw || typeof raw !== "object") return noTabs();
  const { tabs, active } = raw as { tabs?: unknown; active?: unknown };
  if (!Array.isArray(tabs)) return noTabs();
  const seen = new Set<string>();
  const clean: Tab[] = [];
  for (const t of tabs) {
    if (!t || typeof t !== "object") continue;
    const rec = t as Record<string, unknown>;
    // Pre-split: the tab *was* the pane.
    const raws = Array.isArray(rec.panes) ? rec.panes : [rec];
    const panes: Pane[] = [];
    for (const r of raws) {
      if (!r || typeof r !== "object") continue;
      const { path, mode, scroll } = r as Record<string, unknown>;
      if (typeof path !== "string" || !path || seen.has(path)) continue;
      seen.add(path);
      panes.push({
        path,
        mode: mode === "preview" ? "preview" : "source",
        scroll: typeof scroll === "number" && scroll >= 0 && scroll <= 1 ? scroll : 0,
      });
      if (panes.length === 2) break; // two is the limit
    }
    if (!panes.length) continue;
    const focused = typeof rec.focused === "number" ? Math.trunc(rec.focused) : 0;
    const ratio = typeof rec.ratio === "number" ? rec.ratio : EVEN;
    clean.push({
      panes,
      focused: focused >= 0 && focused < panes.length ? focused : 0,
      ratio: ratio >= MIN_RATIO && ratio <= 1 - MIN_RATIO ? ratio : EVEN,
      preview: rec.preview === true && panes.length === 1,
    });
  }
  if (!clean.length) return noTabs();
  const i = typeof active === "number" ? Math.trunc(active) : 0;
  return { tabs: clean, active: i >= 0 && i < clean.length ? i : 0 };
}
