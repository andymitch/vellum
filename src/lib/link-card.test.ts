// The internal link preview card (#62). It has to read like prose, so most of
// these are about what must NOT end up in the excerpt.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
import { noteCard } from "./link-card";

describe("noteCard", () => {
  test("titles the card with the first heading, else the filename", () => {
    expect(noteCard("# Trip *plans*\n\nPack light.", "travel/2026.md").title).toBe("Trip plans");
    expect(noteCard("Just text.", "travel/2026.md").title).toBe("2026");
  });

  test("excerpts prose, skipping fences, rules, tables, later headings and markers", () => {
    const md = [
      "---",
      "type: note",
      "---",
      "# Title",
      "```js",
      "const notProse = 1;",
      "```",
      "---",
      "| a | b |",
      "## Section",
      "- [ ] Buy **milk** and [eggs](https://x.test)",
      "> see [[shop/list|the list]]",
      "Never reached.",
    ].join("\n");
    expect(noteCard(md, "n.md")).toEqual({ title: "Title", excerpt: "Buy milk and eggs see the list" });
  });

  test("truncates long excerpts on a word boundary", () => {
    const { excerpt } = noteCard(`${"word ".repeat(60)}`, "n.md");
    expect(excerpt.length).toBeLessThanOrEqual(181);
    expect(excerpt.endsWith("word…")).toBe(true);
  });
});
