// The tab-list rules (#169). These are the ones a click in the sidebar or a
// rename on another device runs through, so they're pinned here rather than
// discovered in the UI.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
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
  tabLabels,
  updateActive,
  type TabList,
} from "./tabs";

/** Compact view of a list: `*` marks the active tab, `~` a preview tab. */
const show = (l: TabList) =>
  l.tabs.map((t, i) => `${i === l.active ? "*" : ""}${t.preview ? "~" : ""}${t.path}`).join(" ");

const opened = (...paths: string[]) =>
  paths.reduce((l, p) => openTab(l, p, { pin: true }), noTabs());

describe("openTab", () => {
  test("the first open becomes the only tab, and the active one", () => {
    expect(show(openTab(noTabs(), "a.md"))).toBe("*~a.md");
  });

  test("a single click replaces the preview tab instead of piling up", () => {
    let l = openTab(noTabs(), "a.md");
    l = openTab(l, "b.md");
    l = openTab(l, "c.md");
    expect(show(l)).toBe("*~c.md");
  });

  test("a pinned tab is kept, and the next click opens beside it", () => {
    let l = openTab(noTabs(), "a.md", { pin: true });
    l = openTab(l, "b.md");
    expect(show(l)).toBe("a.md *~b.md");
  });

  test("only ever one preview tab, wherever it sits", () => {
    // Preview tab in the middle: pin the tab after it, go back to the pinned
    // first tab, then open a note. The existing preview tab is reused in place.
    let l = openTab(noTabs(), "a.md", { pin: true });
    l = openTab(l, "b.md"); // preview, index 1
    l = openTab(l, "c.md", { newTab: true, pin: true });
    l = activate(l, 0);
    l = openTab(l, "d.md");
    expect(show(l)).toBe("a.md *~d.md c.md");
    expect(l.tabs.filter((t) => t.preview)).toHaveLength(1);
  });

  test("opening a note that is already open focuses its tab", () => {
    let l = opened("a.md", "b.md", "c.md");
    l = openTab(l, "a.md");
    expect(show(l)).toBe("*a.md b.md c.md");
    expect(l.tabs).toHaveLength(3);
  });

  test("a double-click pins the tab it focuses", () => {
    let l = openTab(noTabs(), "a.md");
    expect(activeTab(l)?.preview).toBe(true);
    l = openTab(l, "a.md", { pin: true });
    expect(activeTab(l)?.preview).toBe(false);
    expect(l.tabs).toHaveLength(1);
  });

  test("newTab opens beside the active tab, pinned, leaving the preview tab alone", () => {
    let l = openTab(noTabs(), "a.md"); // preview
    l = openTab(l, "b.md", { newTab: true });
    expect(show(l)).toBe("~a.md *b.md");
    expect(l.tabs.filter((t) => t.preview)).toHaveLength(1);
  });

  test("a new tab inherits the mode passed in, so preview reading carries over", () => {
    const l = openTab(noTabs(), "a.md", { mode: "preview" });
    expect(activeTab(l)?.mode).toBe("preview");
    expect(activeTab(openTab(noTabs(), "a.md"))?.mode).toBe("source");
  });
});

describe("closeTab", () => {
  test("closing the active tab focuses the one to its right", () => {
    let l = opened("a.md", "b.md", "c.md");
    l = activate(l, 1);
    expect(show(closeTab(l, 1))).toBe("a.md *c.md");
  });

  test("closing the last tab focuses its left-hand neighbour", () => {
    const l = opened("a.md", "b.md", "c.md"); // active is c.md
    expect(show(closeTab(l, 2))).toBe("a.md *b.md");
  });

  test("closing a tab left of the active one keeps the same note active", () => {
    const l = opened("a.md", "b.md", "c.md");
    expect(show(closeTab(l, 0))).toBe("b.md *c.md");
  });

  test("closing the only tab empties the list", () => {
    expect(closeTab(openTab(noTabs(), "a.md"), 0)).toEqual(noTabs());
  });

  test("an out-of-range index changes nothing", () => {
    const l = opened("a.md");
    expect(closeTab(l, 4)).toBe(l);
  });
});

describe("moveTab", () => {
  // `opened` leaves the last note active, and a drag rearranges the strip
  // rather than navigating — so the active *note* keeps the star wherever the
  // move puts it.
  test("a tab dragged right lands at the index it was dropped on", () => {
    const l = opened("a.md", "b.md", "c.md");
    expect(show(moveTab(l, 0, 2))).toBe("b.md *c.md a.md");
  });

  test("a tab dragged left lands there too", () => {
    const l = opened("a.md", "b.md", "c.md");
    expect(show(moveTab(l, 2, 0))).toBe("*c.md a.md b.md");
  });

  test("the note that was active stays active, wherever it ends up", () => {
    let l = opened("a.md", "b.md", "c.md");
    l = activate(l, 1); // b.md
    // Drag the *active* tab to the end.
    expect(show(moveTab(l, 1, 2))).toBe("a.md c.md *b.md");
    // Drag a different tab past it: b.md is still the active note.
    expect(show(moveTab(l, 0, 2))).toBe("*b.md c.md a.md");
  });

  test("dropping a tab where it already is changes nothing", () => {
    const l = opened("a.md", "b.md");
    expect(moveTab(l, 1, 1)).toBe(l);
  });

  test("an index past the end clamps to the last position", () => {
    const l = opened("a.md", "b.md");
    expect(show(moveTab(l, 0, 9))).toBe("*b.md a.md");
  });

  test("dragging the only tab is a no-op", () => {
    const l = opened("a.md");
    expect(moveTab(l, 0, 0)).toBe(l);
    expect(moveTab(l, 0, 1)).toBe(l);
  });

  test("an out-of-range source changes nothing", () => {
    const l = opened("a.md", "b.md");
    expect(moveTab(l, 5, 0)).toBe(l);
    expect(moveTab(l, -1, 0)).toBe(l);
  });
});

describe("updateActive", () => {
  test("mode and scroll are remembered per tab", () => {
    let l = opened("a.md", "b.md");
    l = updateActive(l, { mode: "preview", scroll: 0.5 });
    l = activate(l, 0);
    expect(activeTab(l)?.mode).toBe("source");
    expect(activeTab(l)?.scroll).toBe(0);
    l = activate(l, 1);
    expect(activeTab(l)?.mode).toBe("preview");
    expect(activeTab(l)?.scroll).toBe(0.5);
  });

  test("a no-op patch returns the same list, so it can't re-fire an effect", () => {
    const l = opened("a.md");
    expect(updateActive(l, { mode: "source" })).toBe(l);
    expect(pin(l, 0)).toBe(l);
    expect(activate(l, 0)).toBe(l);
    const none = noTabs();
    expect(updateActive(none, { scroll: 0.2 })).toBe(none);
  });
});

describe("renamePaths", () => {
  test("a renamed note is followed by its tab", () => {
    const l = opened("a.md", "b.md");
    expect(show(renamePaths(l, "a.md", "z.md", false))).toBe("z.md *b.md");
  });

  test("renaming a folder moves every tab inside it", () => {
    const l = opened("d/a.md", "d/sub/b.md", "e.md");
    expect(show(renamePaths(l, "d", "moved", true))).toBe("moved/a.md moved/sub/b.md *e.md");
  });

  test("a folder rename doesn't touch a path that merely shares the prefix", () => {
    const l = opened("dir/a.md", "dirty/b.md");
    expect(show(renamePaths(l, "dir", "d2", true))).toBe("d2/a.md *dirty/b.md");
  });

  test("renaming onto an open path collapses the two tabs, keeping the active note", () => {
    let l = opened("a.md", "b.md", "c.md");
    l = activate(l, 0);
    const after = renamePaths(l, "a.md", "b.md", false);
    expect(show(after)).toBe("*b.md c.md");
  });

  test("a rename that touches nothing returns the same list", () => {
    const l = opened("a.md");
    expect(renamePaths(l, "other.md", "z.md", false)).toBe(l);
  });
});

describe("removePaths", () => {
  test("deleting a note in a background tab drops that tab", () => {
    const l = opened("a.md", "b.md", "c.md"); // active is c.md
    expect(show(removePaths(l, "a.md", false))).toBe("b.md *c.md");
  });

  test("deleting the active note focuses a surviving tab", () => {
    const l = opened("a.md", "b.md");
    expect(show(removePaths(l, "b.md", false))).toBe("*a.md");
  });

  test("deleting a folder drops every tab inside it", () => {
    const l = opened("d/a.md", "d/b.md", "e.md");
    expect(show(removePaths(l, "d", true))).toBe("*e.md");
  });

  test("deleting the last open note empties the list", () => {
    expect(removePaths(opened("a.md"), "a.md", false)).toEqual(noTabs());
  });
});

describe("pruneTabs", () => {
  test("tabs for notes that no longer exist are dropped on restore", () => {
    const l = opened("a.md", "gone.md", "c.md");
    const kept = pruneTabs(l, (p) => p !== "gone.md");
    expect(show(kept)).toBe("a.md *c.md");
  });

  test("losing the active note falls back to a tab that survived", () => {
    let l = opened("a.md", "b.md");
    l = activate(l, 1);
    expect(show(pruneTabs(l, (p) => p === "a.md"))).toBe("*a.md");
  });

  test("nothing missing is a no-op", () => {
    const l = opened("a.md");
    expect(pruneTabs(l, () => true)).toBe(l);
  });
});

describe("tabLabels", () => {
  const labels = (...paths: string[]) =>
    tabLabels(opened(...paths).tabs).map((l) => (l.qualifier ? `${l.name} · ${l.qualifier}` : l.name));

  test("distinct names are shown bare", () => {
    expect(labels("a.md", "d/b.md")).toEqual(["a", "b"]);
  });

  test("colliding names are qualified by their folder", () => {
    expect(labels("journal/Notes.md", "projects/Notes.md")).toEqual([
      "Notes · journal",
      "Notes · projects",
    ]);
  });

  test("a root note's qualifier is the root itself", () => {
    expect(labels("Notes.md", "journal/Notes.md")).toEqual(["Notes · /", "Notes · journal"]);
  });

  test("the qualifier grows only as deep as it must to separate them", () => {
    expect(labels("work/q3/Notes.md", "home/q3/Notes.md")).toEqual([
      "Notes · work/q3",
      "Notes · home/q3",
    ]);
    // One level is enough here, so the deeper folders stay out of it.
    expect(labels("work/q3/Notes.md", "work/q4/Notes.md")).toEqual([
      "Notes · q3",
      "Notes · q4",
    ]);
  });

  test("a collision between three notes qualifies all of them", () => {
    expect(labels("a/N.md", "b/N.md", "c/N.md")).toEqual(["N · a", "N · b", "N · c"]);
  });

  test("only the colliding names are qualified", () => {
    expect(labels("a/N.md", "b/N.md", "other.md")).toEqual(["N · a", "N · b", "other"]);
  });

  test("no tabs, no labels", () => {
    expect(tabLabels([])).toEqual([]);
  });
});

describe("normalize", () => {
  test("reads back a persisted list", () => {
    const raw = { tabs: [{ path: "a.md", mode: "preview", scroll: 0.5, preview: false }], active: 0 };
    expect(normalize(raw)).toEqual({
      tabs: [{ path: "a.md", mode: "preview", scroll: 0.5, preview: false }],
      active: 0,
    });
  });

  test("malformed entries are dropped rather than trusted", () => {
    const raw = {
      tabs: [
        { path: "a.md", mode: "nonsense", scroll: 42, preview: "yes" },
        { path: "a.md" }, // duplicate
        { path: "" },
        null,
        { mode: "source" }, // no path
      ],
      active: 9,
    };
    expect(normalize(raw)).toEqual({
      tabs: [{ path: "a.md", mode: "source", scroll: 0, preview: false }],
      active: 0,
    });
  });

  test("junk in localStorage reads as no tabs at all", () => {
    expect(normalize(undefined)).toEqual(noTabs());
    expect(normalize("nope")).toEqual(noTabs());
    expect(normalize({})).toEqual(noTabs());
    expect(normalize({ tabs: [] })).toEqual(noTabs());
  });
});
