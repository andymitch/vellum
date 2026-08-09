// Inline `#tag` styling and click-through in source mode (#202).
//
// Tags were interactive only in preview: chips there open the search palette
// seeded with the tag. In the editor — the default view, and where most time is
// spent — they were undecorated text, so the feature read as inert.
//
// This is a mark decoration rather than a replacing widget on purpose. The text
// stays exactly as typed and stays editable: the caret moves through a tag
// character by character, backspace deletes one character, and selection behaves
// normally. A widget would make a tag atomic and break all three.
//
// A ViewPlugin (not a StateField) because marks don't affect line layout, and
// decorating only the viewport keeps a long note cheap.

import {
  Decoration,
  EditorView,
  ViewPlugin,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";
import { scanTags } from "$lib/tags";

// One shared mark for every tag — the tag text is read back from the document
// at click time, so no per-occurrence decoration is allocated.
const mark = Decoration.mark({ class: "cm-tag" });

function decorate(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to } of view.visibleRanges) {
    let pos = from;
    while (pos <= to) {
      const line = view.state.doc.lineAt(pos);
      for (const t of scanTags(line.text)) {
        builder.add(line.from + t.from, line.from + t.to, mark);
      }
      pos = line.to + 1;
    }
  }
  return builder.finish();
}

/// The tag covering document position `pos`, if any.
function tagAt(view: EditorView, pos: number): string | null {
  const line = view.state.doc.lineAt(pos);
  const off = pos - line.from;
  for (const t of scanTags(line.text)) {
    if (off >= t.from && off < t.to) return t.tag;
  }
  return null;
}

/// Style tags and route clicks on them to `ontag`, the same callback preview
/// chips use, so both surfaces open the palette identically.
export function tagMarks(ontag: (tag: string) => void) {
  return [
    ViewPlugin.fromClass(
      class {
        decorations: DecorationSet;
        constructor(view: EditorView) {
          this.decorations = decorate(view);
        }
        update(u: ViewUpdate) {
          if (u.docChanged || u.viewportChanged) this.decorations = decorate(u.view);
        }
      },
      { decorations: (v) => v.decorations },
    ),
    EditorView.domEventHandlers({
      // `click` rather than `mousedown` so this also covers a touch tap, and so
      // the selection below has already settled.
      click(e, view) {
        // Alt-click is the escape hatch: it places the caret inside a tag, which
        // is otherwise awkward when a plain click navigates away. Other
        // modifiers are native gestures (column select, extend selection).
        if (e.altKey || e.metaKey || e.ctrlKey || e.shiftKey) return false;
        // A drag that happens to end on a tag is a selection, not a click.
        if (!view.state.selection.main.empty) return false;
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        if (pos === null) return false;
        const tag = tagAt(view, pos);
        if (!tag) return false;
        e.preventDefault();
        ontag(tag);
        return true;
      },
    }),
  ];
}
