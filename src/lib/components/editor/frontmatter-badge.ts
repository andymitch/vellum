// Replace a note's leading frontmatter block with a compact type badge (#104).
//
// The block stays in the document — it is only hidden visually. Stripping it out
// of the editor's value would mean reconstructing it on every save and keeping a
// second merge base in step with it, which is a clobber risk (see the note on
// write_note_merged in vault.rs) for no real gain. The document stays whole, the
// save path is untouched, and the user still sees what type the note is.
//
// This is a StateField rather than a ViewPlugin because block decorations are
// not allowed from plugins in CodeMirror 6 — they change line layout, so they
// have to be part of the state.

import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import { StateField, type EditorState } from "@codemirror/state";
import { noteTypeInfo, parseNote } from "$lib/note-type";

class BadgeWidget extends WidgetType {
  constructor(readonly label: string) {
    super();
  }
  eq(other: BadgeWidget) {
    return other.label === this.label;
  }
  toDOM() {
    const el = document.createElement("div");
    el.className = "cm-type-badge";
    el.textContent = this.label;
    // Not editable, and skipped by the caret — the badge stands in for text the
    // user can still reach by deleting it from the source elsewhere.
    el.setAttribute("aria-label", `Note type: ${this.label}`);
    return el;
  }
  ignoreEvent() {
    return true;
  }
}

function badgeFor(state: EditorState): DecorationSet {
  // Only the first ~1KB can hold a frontmatter block, so don't stringify a large
  // document on every keystroke just to find it.
  const head = state.doc.sliceString(0, Math.min(state.doc.length, 1024));
  const { frontmatter, type } = parseNote(head);
  if (!frontmatter || type === "markdown") return Decoration.none;
  // A block decoration may not span a partial line, and `frontmatter` includes
  // the newline after the closing fence — so a range of 0..length would end at
  // the *start* of the body's first line and CodeMirror would reject it. End at
  // the closing fence's line end instead, leaving that newline in place as the
  // separator between the badge and the body.
  const end = frontmatter.replace(/\r?\n$/, "").length;
  if (end <= 0 || end > state.doc.length) return Decoration.none;
  return Decoration.set([
    Decoration.replace({
      widget: new BadgeWidget(noteTypeInfo(type).label),
      block: true,
    }).range(0, end),
  ]);
}

export const frontmatterBadge = StateField.define<DecorationSet>({
  create: badgeFor,
  update(value, tr) {
    return tr.docChanged ? badgeFor(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});
