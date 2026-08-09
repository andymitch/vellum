// Alternating block backgrounds for scratchpad notes (#177), the way Heynote
// bands its blocks and a spreadsheet bands its rows.
//
// A `---` separator is a thin rule that's easy to lose in a long note, and on a
// short mobile viewport it's easy to lose entirely. Tinting alternate blocks
// makes the note read as a stack of entries at a glance.
//
// Presentation only: the Markdown is untouched, so a scratchpad is still a plain
// `.md` file that exports, syncs, and renders in any other tool unchanged.
//
// Line decorations affect line layout, so this is a StateField rather than a
// view plugin — the same constraint as the frontmatter badge.

import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
import { RangeSetBuilder, StateField, type EditorState } from "@codemirror/state";
import { noteTypeOf, parseNote } from "$lib/note-type";

const band = Decoration.line({ class: "cm-block-band" });

// A separator line: a thematic break on its own. Matches what the separator
// button inserts and what `daySeparator` writes.
const SEPARATOR_LINE = /^[ \t]*---[ \t]*$/;

function bands(state: EditorState): DecorationSet {
  const head = state.doc.sliceString(0, Math.min(state.doc.length, 1024));
  if (noteTypeOf(head) !== "scratchpad") return Decoration.none;

  // Skip the frontmatter block: its closing `---` is a fence, not a separator,
  // and counting it would invert the banding for the whole note.
  const frontmatterEnd = parseNote(head).frontmatter.length;

  const builder = new RangeSetBuilder<Decoration>();
  let block = 0;
  for (let n = 1; n <= state.doc.lines; n++) {
    const line = state.doc.line(n);
    if (line.from < frontmatterEnd) continue;
    if (SEPARATOR_LINE.test(line.text)) {
      block++;
      continue; // the rule itself stays unbanded, so it reads as the boundary
    }
    // Band every other block. Ranges are added in ascending order, which is what
    // RangeSetBuilder requires.
    if (block % 2 === 1) builder.add(line.from, line.from, band);
  }
  return builder.finish();
}

export const blockBands = StateField.define<DecorationSet>({
  create: bands,
  update(value, tr) {
    return tr.docChanged ? bands(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});
