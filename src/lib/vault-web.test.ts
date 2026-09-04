// Tests for the browser vault's rules (#221).
//
// These are the parts of vault.rs that had to be re-implemented in TypeScript,
// and the risk is drift: the same vault opened in the app and in the browser
// should show the same tree, find the same notes and count the same tags. The
// cases below are the ones vault.rs's own tests pin down, restated here — so if
// one side changes, this fails rather than the two quietly disagreeing.
//
// Run with `bun test`.

// Referenced here rather than added to tsconfig's `types`, so bun's globals
// stay scoped to the test files that actually want them.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
import {
  buildTree,
  freeKey,
  isHiddenPath,
  noteTypesIn,
  searchNotesIn,
  tagCountsIn,
} from "./vault-web";
import { asTagQuery, distinctTags } from "./tags";
import { readZip, writeZip } from "./zip";

describe("buildTree", () => {
  test("folders before files, each alphabetical", () => {
    const tree = buildTree(["zeta.md", "alpha.md", "work/b.md", "work/a.md", "admin/x.md"]);
    expect(tree.map((n) => n.name)).toEqual(["admin", "work", "alpha.md", "zeta.md"]);
    const work = tree.find((n) => n.name === "work")!;
    expect(work.is_dir).toBe(true);
    expect(work.children.map((n) => n.path)).toEqual(["work/a.md", "work/b.md"]);
  });

  test("a .keep marker makes an empty folder exist without becoming a node", () => {
    const tree = buildTree(["empty/.keep", "empty/deep/.keep"]);
    expect(tree).toHaveLength(1);
    expect(tree[0]).toMatchObject({ name: "empty", path: "empty", is_dir: true });
    expect(tree[0].children.map((n) => n.name)).toEqual(["deep"]);
    expect(tree[0].children[0].children).toEqual([]);
  });

  test("a .keep at the root names no folder", () => {
    expect(buildTree([".keep"])).toEqual([]);
  });

  test("folders are implied by a note's path", () => {
    const tree = buildTree(["a/b/c.md"]);
    expect(tree[0].path).toBe("a");
    expect(tree[0].children[0].path).toBe("a/b");
    expect(tree[0].children[0].children[0].path).toBe("a/b/c.md");
  });
});

describe("freeKey", () => {
  test("keeps a free name", () => {
    expect(freeKey(new Set(["a.md"]), "b.md")).toBe("b.md");
  });

  test("numbers a taken name before the extension", () => {
    expect(freeKey(new Set(["Untitled.md"]), "Untitled.md")).toBe("Untitled 1.md");
    expect(freeKey(new Set(["Untitled.md", "Untitled 1.md"]), "Untitled.md")).toBe("Untitled 2.md");
  });

  test("handles a name with no extension", () => {
    expect(freeKey(new Set(["notes"]), "notes")).toBe("notes 1");
  });
});

test("folder markers and trashed notes are hidden from scans", () => {
  expect(isHiddenPath("work/.keep")).toBe(true);
  expect(isHiddenPath("work/")).toBe(true);
  expect(isHiddenPath(".trash")).toBe(true);
  expect(isHiddenPath(".trash/old.md")).toBe(true);
  expect(isHiddenPath("work/todo.md")).toBe(false);
});

describe("search", () => {
  const notes = [
    { path: "one.md", text: "buy milk\nand bread" },
    { path: "two.md", text: "MILK is here" },
    { path: "work/.keep", text: "" },
  ];

  test("is case-insensitive and numbers the matching lines", () => {
    expect(searchNotesIn(notes, "milk", 20)).toEqual([
      { path: "one.md", lines: ["1: buy milk"] },
      { path: "two.md", lines: ["1: MILK is here"] },
    ]);
  });

  test("an empty query matches nothing", () => {
    expect(searchNotesIn(notes, "   ", 20)).toEqual([]);
  });

  test("caps results at max", () => {
    expect(searchNotesIn(notes, "milk", 1)).toHaveLength(1);
  });

  test("shows at most three lines per note", () => {
    const many = [{ path: "n.md", text: "x\nx\nx\nx\nx" }];
    expect(searchNotesIn(many, "x", 20)[0].lines).toEqual(["1: x", "2: x", "3: x"]);
  });

  test("a tag query matches tag identity, not the characters", () => {
    const tagged = [
      { path: "hit.md", text: "planning #bar" },
      { path: "miss.md", text: "a #barbecue and https://x.test/#bar" },
    ];
    expect(searchNotesIn(tagged, "#bar", 20).map((h) => h.path)).toEqual(["hit.md"]);
  });

  test("a tag query tolerates the palette's trailing space", () => {
    const tagged = [{ path: "hit.md", text: "ends with #bar" }];
    expect(searchNotesIn(tagged, "#bar ", 20).map((h) => h.path)).toEqual(["hit.md"]);
  });

  test("a text query promotes notes carrying it as a tag", () => {
    const mixed = [
      { path: "plain.md", text: "some work to do" },
      { path: "tagged.md", text: "list #work" },
    ];
    expect(searchNotesIn(mixed, "work", 20).map((h) => h.path)).toEqual([
      "tagged.md",
      "plain.md",
    ]);
  });
});

describe("tags", () => {
  test("de-duplicates within a note, case-insensitively", () => {
    expect(distinctTags("#Work and #work and #work/next")).toEqual(["Work", "work/next"]);
  });

  test("counts notes per tag, most used first then alphabetical", () => {
    expect(
      tagCountsIn([
        { path: "a.md", text: "#zeta #alpha" },
        { path: "b.md", text: "#alpha" },
        { path: "hidden/.keep", text: "#ignored" },
      ]),
    ).toEqual([
      { tag: "alpha", count: 2 },
      { tag: "zeta", count: 1 },
    ]);
  });

  test("only reads a query as a tag when it could have been produced as one", () => {
    expect(asTagQuery("#work")).toBe("work");
    expect(asTagQuery("  #Work/Next  ")).toBe("work/next");
    expect(asTagQuery("#work/")).toBe("work");
    expect(asTagQuery("work")).toBeNull();
    expect(asTagQuery("# heading")).toBeNull();
    expect(asTagQuery("#two words")).toBeNull();
  });
});

test("note types list only typed notes", () => {
  expect(
    noteTypesIn([
      { path: "plain.md", text: "# just markdown" },
      { path: "list.md", text: "---\ntype: todo\n---\n- [ ] milk" },
      { path: "break.md", text: "text\n\n---\ntype: todo\n---\n" },
    ]),
  ).toEqual([{ path: "list.md", note_type: "todo" }]);
});

// The container logic is exercised here; DEFLATE comes from the platform, which
// bun doesn't provide yet (browsers and node do). Skipped rather than dropped so
// it starts running when it can.
const hasCompression = typeof CompressionStream !== "undefined";
describe.skipIf(!hasCompression)("zip", () => {
  test("round-trips notes and empty folders", async () => {
    const encoder = new TextEncoder();
    const bytes = await writeZip([
      { name: "note.md", data: encoder.encode("# hello\n") },
      { name: "work/deep.md", data: encoder.encode("x".repeat(5000)) },
      { name: "empty/", data: null },
    ]);
    const back = await readZip(bytes);
    expect(back.map((e) => e.name)).toEqual(["note.md", "work/deep.md", "empty/"]);
    const decoder = new TextDecoder();
    expect(decoder.decode(back[0].data!)).toBe("# hello\n");
    expect(decoder.decode(back[1].data!)).toHaveLength(5000);
    expect(back[2].data).toBeNull();
  });

  test("rejects data that isn't an archive", async () => {
    await expect(readZip(new TextEncoder().encode("not a zip"))).rejects.toThrow(
      "not a zip archive",
    );
  });
});
