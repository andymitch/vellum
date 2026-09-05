// Tag rules that still live in TypeScript.
//
// `tags.ts` backs the editor: `render-markdown.ts` uses TAG_RE/TAG_START_RE to
// highlight tags as you type, and the sidebar turns a `#tag` query into a
// search. The *vault's* tag rules — which notes carry which tags, and their
// counts — are Rust now (`extract_tags`/`has_tag` in vellum-vault), tested
// there. These cases pin the editor-side half, and are the ones the old
// IndexedDB backend's suite covered before its rules moved into the vault.
/// <reference types="bun-types" />
import { describe, expect, test } from "bun:test";
import { asTagQuery, distinctTags, hasTag, scanTags, trimTag } from "./tags";

describe("distinctTags", () => {
  test("deduplicated case-insensitively, keeping the first spelling seen", () => {
    expect(distinctTags("#Work and #work and #work/next")).toEqual(["Work", "work/next"]);
  });

  test("a bare # or a heading is not a tag", () => {
    expect(distinctTags("# heading\nsome text")).toEqual([]);
    expect(distinctTags("nothing here")).toEqual([]);
  });

  test("trailing punctuation is not part of the tag", () => {
    expect(distinctTags("#work, #home.")).toEqual(["work", "home"]);
  });
});

describe("asTagQuery", () => {
  test("recognises a tag query and normalises it", () => {
    expect(asTagQuery("#work")).toBe("work");
    expect(asTagQuery("  #Work/Next  ")).toBe("work/next");
    expect(asTagQuery("#work/")).toBe("work");
  });

  test("plain text, a heading and a multi-word query are not tag queries", () => {
    expect(asTagQuery("work")).toBeNull();
    expect(asTagQuery("# heading")).toBeNull();
    expect(asTagQuery("#two words")).toBeNull();
  });
});

describe("hasTag", () => {
  test("matches whole tags only, so #work does not match #workflow", () => {
    expect(hasTag("planning #work today", "work")).toBe(true);
    expect(hasTag("planning #workflow today", "work")).toBe(false);
  });

  test("a nested tag matches itself, not its parent", () => {
    expect(hasTag("#work/next", "work/next")).toBe(true);
    expect(hasTag("#work/next", "work")).toBe(false);
  });
});

describe("trimTag", () => {
  test("drops trailing separators so '#work/' reads as 'work'", () => {
    expect(trimTag("work/")).toBe("work");
    expect(trimTag("work-")).toBe("work");
    expect(trimTag("work")).toBe("work");
  });
});

describe("scanTags", () => {
  test("reports each tag with its offsets, for the editor's highlighting", () => {
    const hits = scanTags("a #one and #two");
    expect(hits.map((h) => h.tag)).toEqual(["one", "two"]);
    // `from` is the '#'; `to` is one past the tag's last character.
    expect(hits[0].from).toBe(2);
    expect(hits[0].to).toBe(6);
  });
});
