// Clickable checkboxes inside the editor, for TODO notes (#174).
//
// A TODO note renders in the editor rather than in preview, so you can type
// items and tick them in the same view — the original #104 implementation
// forced preview, which had no text entry at all and made the type unusable.
//
// Because we draw the checkbox ourselves, GFM's rule that a task item needs
// text after the marker doesn't apply: `- [ ] ` on its own still shows a real
// checkbox here, where `marked` would render a literal "[ ]".
//
// These are inline decorations, so unlike the frontmatter badge they can come
// from a view plugin — only block-level decorations must live in a state field.

import {
  Decoration,
  EditorView,
  ViewPlugin,
  WidgetType,
  type DecorationSet,
  type ViewUpdate,
} from "@codemirror/view";
import { RangeSetBuilder } from "@codemirror/state";
import { noteTypeOf } from "$lib/note-type";

// The `[ ]` / `[x]` of a task line, with the marker character captured. Anchored
// to a list bullet so a stray "[x]" mid-prose isn't turned into a checkbox.
const TASK_MARKER = /^([ \t]*(?:[-*+]|\d+[.)])[ \t]+)\[([ xX])\]/;

class CheckboxWidget extends WidgetType {
  constructor(
    readonly checked: boolean,
    /// Document position of the marker character between the brackets.
    readonly markerPos: number,
  ) {
    super();
  }

  eq(other: CheckboxWidget) {
    return other.checked === this.checked && other.markerPos === this.markerPos;
  }

  toDOM(view: EditorView) {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = this.checked;
    box.className = "cm-task-checkbox";
    box.setAttribute("aria-label", this.checked ? "Completed" : "Not completed");
    // Keep the caret where it was: without this, pressing on the box moves the
    // selection, which on mobile also summons the keyboard for a tap that was
    // only ever meant to tick something.
    box.addEventListener("mousedown", (e) => e.preventDefault());
    box.addEventListener("click", (e) => {
      e.preventDefault();
      // Flip just the one character. The document stays ordinary Markdown, and
      // the edit flows through the normal change path, so it autosaves and
      // CRDT-merges like any other keystroke.
      view.dispatch({
        changes: {
          from: this.markerPos,
          to: this.markerPos + 1,
          insert: this.checked ? " " : "x",
        },
      });
    });
    return box;
  }

  // The widget handles its own clicks; letting CodeMirror also process them
  // would move the selection underneath us.
  ignoreEvent() {
    return false;
  }
}

function checkboxes(view: EditorView): DecorationSet {
  const head = view.state.doc.sliceString(0, Math.min(view.state.doc.length, 1024));
  if (noteTypeOf(head) !== "todo") return Decoration.none;
  const builder = new RangeSetBuilder<Decoration>();
  // Only the visible ranges: a long list shouldn't cost a full-document scan on
  // every keystroke.
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = view.state.doc.lineAt(pos);
      const m = TASK_MARKER.exec(line.text);
      if (m) {
        const start = line.from + m[1].length;
        builder.add(
          start,
          start + 3, // the whole "[x]"
          Decoration.replace({
            widget: new CheckboxWidget(m[2] !== " ", start + 1),
          }),
        );
      }
      pos = line.to + 1;
    }
  }
  return builder.finish();
}

export const taskCheckboxes = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = checkboxes(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.viewportChanged) {
        this.decorations = checkboxes(update.view);
      }
    }
  },
  {
    decorations: (v) => v.decorations,
    // Without this the widget's click never reaches us — CodeMirror treats the
    // replaced range as plain text and handles the press itself.
    eventHandlers: {
      mousedown(event) {
        const t = event.target as HTMLElement;
        return t.classList?.contains("cm-task-checkbox");
      },
    },
  },
);
