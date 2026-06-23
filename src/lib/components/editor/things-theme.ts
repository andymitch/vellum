import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

/**
 * CodeMirror theme matching the Obsidian "Things" theme.
 * Colors are pulled from CSS custom properties defined in app.css so the
 * editor stays in sync with light/dark mode automatically.
 */
const thingsEditorTheme = EditorView.theme(
  {
  "&": {
    color: "var(--editor-fg)",
    backgroundColor: "var(--editor-bg)",
    height: "100%",
    fontSize: "16px",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-sans)",
    lineHeight: "1.7",
    padding: "1.5rem 0",
  },
  ".cm-content": {
    caretColor: "var(--editor-cursor)",
    maxWidth: "48rem",
    margin: "0 auto",
    padding: "0 1.5rem",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--editor-cursor)",
    borderLeftWidth: "2px",
  },
  "&.cm-focused": {
    outline: "none",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    {
      backgroundColor: "var(--editor-selection)",
    },
  ".cm-activeLine": {
    backgroundColor: "transparent",
  },
  ".cm-gutters": {
    backgroundColor: "var(--editor-bg)",
    color: "var(--editor-muted)",
    border: "none",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "transparent",
    color: "var(--editor-fg)",
  },
  ".cm-foldPlaceholder": {
    backgroundColor: "var(--editor-code-bg)",
    border: "none",
    color: "var(--editor-muted)",
  },
  },
  { dark: true },
);

const thingsHighlightStyle = HighlightStyle.define([
  // Headings — accent colored, bold, scaled up
  { tag: t.heading1, color: "var(--editor-fg)", fontWeight: "700", fontSize: "1.6em" },
  { tag: t.heading2, color: "var(--editor-fg)", fontWeight: "700", fontSize: "1.4em" },
  { tag: t.heading3, color: "var(--editor-fg)", fontWeight: "600", fontSize: "1.2em" },
  {
    tag: [t.heading4, t.heading5, t.heading6],
    color: "var(--editor-fg)",
    fontWeight: "600",
  },
  // Inline emphasis
  { tag: t.strong, color: "var(--editor-fg)", fontWeight: "700" },
  { tag: t.emphasis, color: "var(--editor-fg)", fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  // Links & references
  { tag: [t.link, t.url], color: "var(--editor-accent)", textDecoration: "underline" },
  // Code — only inline/fenced code is monospace (NOT all content)
  {
    tag: t.monospace,
    fontFamily: "var(--font-mono)",
    fontSize: "0.9em",
  },
  // Markdown syntax punctuation (#, *, -, etc.) — muted
  {
    tag: [t.processingInstruction, t.meta, t.contentSeparator],
    color: "var(--editor-muted)",
  },
  // Lists & quotes
  { tag: t.list, color: "var(--editor-accent)" },
  { tag: t.quote, color: "var(--editor-muted)", fontStyle: "italic" },

  // Code syntax highlighting (fenced blocks with a language)
  {
    tag: [t.comment, t.lineComment, t.blockComment],
    color: "var(--code-comment)",
    fontStyle: "italic",
  },
  {
    tag: [
      t.keyword,
      t.controlKeyword,
      t.moduleKeyword,
      t.operatorKeyword,
      t.definitionKeyword,
      t.self,
    ],
    color: "var(--code-keyword)",
  },
  {
    tag: [t.string, t.special(t.string), t.regexp, t.escape],
    color: "var(--code-string)",
  },
  {
    tag: [t.number, t.bool, t.null, t.atom],
    color: "var(--code-number)",
  },
  {
    tag: [t.function(t.variableName), t.function(t.propertyName)],
    color: "var(--code-function)",
  },
  {
    tag: [t.typeName, t.className, t.namespace, t.tagName],
    color: "var(--code-type)",
  },
  {
    tag: [t.variableName, t.propertyName, t.attributeName],
    color: "var(--code-variable)",
  },
]);

export const thingsTheme: Extension = [
  thingsEditorTheme,
  syntaxHighlighting(thingsHighlightStyle),
];
