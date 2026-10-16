import { expect, test } from "bun:test";
import { isBlankNote, newNoteContent } from "./note-type";

test("blank: nothing, whitespace, a fresh typed note, empty journal cells", () => {
  for (const t of ["", "  \n\n", newNoteContent("journal"), newNoteContent("markdown")]) {
    expect(isBlankNote(t)).toBe(true);
  }
  expect(isBlankNote(`${newNoteContent("journal")}<!-- 2026-10-15T00:00:00Z -->\n\n`)).toBe(true);
});

test("not blank: any text, in the body or a journal cell", () => {
  expect(isBlankNote("x")).toBe(false);
  expect(isBlankNote(`${newNoteContent("journal")}<!-- 2026-10-15T00:00:00Z -->\nhi\n`)).toBe(false);
  // Frontmatter alone is set aside, but text after it is not.
  expect(isBlankNote("---\ntitle: a\n---\nbody\n")).toBe(false);
});
