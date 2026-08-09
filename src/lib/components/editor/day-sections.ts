// Journal day sections (#181): alternating block tints, plus each `## ISO-date`
// heading rendered as a full-width rule with the date inline —
//
//     ———————————————————————— Jan 01, 2026 ———
//
// Presentation only. The Markdown stays a plain `## 2026-01-01` heading, so the
// note still sorts, greps and exports as ordinary Markdown, and renders sensibly
// in any other tool. Only the *display* is human-formatted.
//
// Both decorations affect line layout — a block widget and line decorations — so
// this is a StateField; CodeMirror doesn't accept either from a view plugin.

import { Decoration, EditorView, WidgetType, type DecorationSet } from "@codemirror/view";
import { RangeSetBuilder, StateField, type EditorState } from "@codemirror/state";
import { DAY_HEADING, formatDay, noteTypeOf, parseNote } from "$lib/note-type";

const band = Decoration.line({ class: "cm-day-band" });

class DayRuleWidget extends WidgetType {
  constructor(readonly label: string) {
    super();
  }
  eq(other: DayRuleWidget) {
    return other.label === this.label;
  }
  toDOM() {
    const el = document.createElement("div");
    el.className = "cm-day-rule";
    const span = document.createElement("span");
    span.textContent = this.label;
    el.appendChild(span);
    return el;
  }
  ignoreEvent() {
    return true;
  }
}

function decorate(state: EditorState): DecorationSet {
  const head = state.doc.sliceString(0, Math.min(state.doc.length, 1024));
  if (noteTypeOf(head) !== "journal") return Decoration.none;

  // Skip the frontmatter block — nothing in it is a section.
  const frontmatterEnd = parseNote(head).frontmatter.length;

  const builder = new RangeSetBuilder<Decoration>();
  let section = 0;
  for (let n = 1; n <= state.doc.lines; n++) {
    const line = state.doc.line(n);
    if (line.from < frontmatterEnd) continue;
    const m = DAY_HEADING.exec(line.text);
    if (m) {
      section++;
      // Replace the heading with the dated rule. A zero-length line still needs
      // a valid range, so guard the empty case.
      if (line.to > line.from) {
        builder.add(
          line.from,
          line.to,
          Decoration.replace({
            widget: new DayRuleWidget(formatDay(`${m[1]}-${m[2]}-${m[3]}`)),
            block: true,
          }),
        );
      }
      continue;
    }
    // Band alternate sections so they read as distinct blocks.
    if (section % 2 === 0) builder.add(line.from, line.from, band);
  }
  return builder.finish();
}

export const daySections = StateField.define<DecorationSet>({
  create: decorate,
  update(value, tr) {
    return tr.docChanged ? decorate(tr.state) : value;
  },
  provide: (f) => EditorView.decorations.from(f),
});
