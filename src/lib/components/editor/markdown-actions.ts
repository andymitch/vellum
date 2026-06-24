// Markdown editing commands for the mobile toolbar. Each takes the live
// EditorView, dispatches a transaction, and re-focuses so the soft keyboard
// stays up. Uses changeByRange so multi-range selections behave.
import type { EditorView } from "@codemirror/view";
import { EditorSelection } from "@codemirror/state";
import { indentMore, indentLess } from "@codemirror/commands";

/// Wrap each selection with `before`/`after` (bold, italic, strike, inline code).
/// Empty selection: insert the pair and place the cursor between them.
export function wrapInline(view: EditorView, before: string, after = before) {
  const { state } = view;
  view.dispatch(
    state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to);
      const insert = before + text + after;
      const sel =
        text.length === 0
          ? EditorSelection.cursor(range.from + before.length)
          : EditorSelection.range(
              range.from + before.length,
              range.to + before.length,
            );
      return { changes: { from: range.from, to: range.to, insert }, range: sel };
    }),
  );
  view.focus();
}

/// Toggle a line prefix (heading "# ", bullet "- ", quote "> ", checkbox
/// "- [ ] ") on every line the selection touches.
export function toggleLinePrefix(view: EditorView, prefix: string) {
  const { state } = view;
  const changes: { from: number; to: number; insert: string }[] = [];
  const seen = new Set<number>();
  for (const range of state.selection.ranges) {
    let pos = range.from;
    const end = Math.max(range.from, range.to);
    while (pos <= end) {
      const line = state.doc.lineAt(pos);
      if (!seen.has(line.number)) {
        seen.add(line.number);
        if (line.text.startsWith(prefix)) {
          changes.push({ from: line.from, to: line.from + prefix.length, insert: "" });
        } else {
          changes.push({ from: line.from, to: line.from, insert: prefix });
        }
      }
      if (line.to >= end) break;
      pos = line.to + 1;
    }
  }
  if (changes.length) {
    const cs = state.changes(changes);
    // Map the selection forward (assoc 1) so the cursor lands AFTER the inserted
    // prefix, not stranded before it.
    view.dispatch({ changes: cs, selection: state.selection.map(cs, 1) });
  }
  view.focus();
}

/// Insert a markdown link. Selected text becomes the label and the "url"
/// placeholder is selected for typing; with no selection, "text" is selected.
export function insertLink(view: EditorView) {
  const { state } = view;
  view.dispatch(
    state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to);
      if (text.length) {
        const insert = `[${text}](url)`;
        const urlFrom = range.from + 1 + text.length + 2; // past "](" -> "url"
        return {
          changes: { from: range.from, to: range.to, insert },
          range: EditorSelection.range(urlFrom, urlFrom + 3),
        };
      }
      const insert = `[text](url)`;
      return {
        changes: { from: range.from, to: range.to, insert },
        range: EditorSelection.range(range.from + 1, range.from + 5), // "text"
      };
    }),
  );
  view.focus();
}

/// Fence the selected lines in a ```` ``` ```` code block (or insert an empty one).
export function toggleCodeBlock(view: EditorView) {
  const { state } = view;
  const range = state.selection.main;
  if (range.empty) {
    const insert = "```\n\n```";
    view.dispatch({
      changes: { from: range.from, insert },
      selection: EditorSelection.cursor(range.from + 4),
    });
  } else {
    const startLine = state.doc.lineAt(range.from);
    const endLine = state.doc.lineAt(range.to);
    view.dispatch({
      changes: [
        { from: startLine.from, insert: "```\n" },
        { from: endLine.to, insert: "\n```" },
      ],
    });
  }
  view.focus();
}

export function indent(view: EditorView) {
  indentMore(view);
  view.focus();
}
export function outdent(view: EditorView) {
  indentLess(view);
  view.focus();
}
