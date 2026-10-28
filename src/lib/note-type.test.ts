import { expect, test } from "bun:test";
import { SCRATCH_PAD, isBlankNote, newNoteContent, noteTypeAt } from "./note-type";

test("blank: nothing, whitespace, a fresh typed note, empty journal cells", () => {
  for (const t of ["", "  \n\n", newNoteContent("journal"), newNoteContent("markdown")]) {
    expect(isBlankNote(t)).toBe(true);
  }
  expect(isBlankNote(`${newNoteContent("journal")}<!-- 2026-10-15T00:00:00Z -->\n\n`)).toBe(true);
});

test("not blank: any text, in the body or a journal cell", () => {
  expect(isBlankNote("x")).toBe(false);
  expect(isBlankNote(`${newNoteContent("journal")}<!-- 2026-10-15T00:00:00Z -->\nhi\n`)).toBe(false);
  expect(isBlankNote("---\ntitle: a\n---\nbody\n")).toBe(false);
  // Frontmatter or comments a person wrote are content, not scaffolding.
  expect(isBlankNote("---\nMeeting with Sam\n---\n")).toBe(false);
  expect(isBlankNote("---\ntype: journal\ntags: [x]\n---\n")).toBe(false);
  expect(isBlankNote("<!-- todo -->\n")).toBe(false);
});

test("the scratch pad is a journal by its path, with or without frontmatter", () => {
  expect(noteTypeAt(SCRATCH_PAD, "")).toBe("journal");
  expect(noteTypeAt(SCRATCH_PAD, "plain text")).toBe("journal");
  expect(noteTypeAt(`work/${SCRATCH_PAD}`, "")).toBe("markdown");
  expect(noteTypeAt("other.md", newNoteContent("journal"))).toBe("journal");
});
