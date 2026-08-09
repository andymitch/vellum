import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import type { Extension } from "@codemirror/state";

/**
 * CodeMirror theme matching the Obsidian "Things" theme.
 * Colors are pulled from CSS custom properties defined in app.css so the
 * editor stays in sync with light/dark mode automatically.
 */
const thingsEditorTheme = (dark: boolean) =>
  EditorView.theme(
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
    // Top padding clears the floating mobile header (--chrome-h, 0 on desktop) on
    // top of the editor's own 1.5rem breathing room, so the first line isn't
    // hidden behind the chrome (mirrors <main>'s padding in preview). Setting it
    // here (not a :global rule) so it isn't lost to the theme's own .cm-scroller
    // padding, which is injected later and would otherwise win (#148).
    padding: "calc(var(--chrome-h, 0px) + 1.5rem) 0 1.5rem",
  },
  ".cm-content": {
    caretColor: "var(--editor-cursor)",
    maxWidth: "96rem",
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
  // The note-type badge that stands in for a hidden frontmatter block (#104).
  // Small and muted: it says what the note is without competing with the text.
  ".cm-type-badge": {
    display: "inline-block",
    margin: "0 0 0.6em",
    padding: "0.1em 0.5em",
    borderRadius: "0.375rem",
    fontSize: "0.75em",
    letterSpacing: "0.02em",
    textTransform: "uppercase",
    color: "var(--editor-muted)",
    background: "color-mix(in srgb, var(--editor-muted) 15%, transparent)",
    userSelect: "none",
  },
  // Editor task checkboxes (#174). Sized and spaced to sit on the text baseline
  // rather than pushing the line around.
  ".cm-task-checkbox": {
    appearance: "none",
    width: "1em",
    height: "1em",
    verticalAlign: "-0.12em",
    marginRight: "0.15em",
    border: "1.5px solid var(--editor-muted)",
    borderRadius: "0.25em",
    cursor: "pointer",
  },
  ".cm-task-checkbox:checked": {
    background: "var(--editor-accent)",
    borderColor: "var(--editor-accent)",
  },
  // Alternating journal sections (#177/#181). Derived from the editor foreground
  // so it lands correctly in all eight themes and in both light and dark, rather
  // than a hardcoded grey that works in one. Deliberately faint: this is banding
  // to aid scanning, not a highlight.
  ".cm-day-band": {
    background: "color-mix(in srgb, var(--editor-fg) 4%, transparent)",
  },
  // A day heading rendered as a full-width rule with the date inline. The rules
  // are flex children either side of the label, so they fill whatever width is
  // left however long the date is.
  ".cm-day-rule": {
    display: "flex",
    alignItems: "center",
    gap: "0.6em",
    margin: "1.2em 0 0.6em",
    fontSize: "0.8em",
    letterSpacing: "0.04em",
    textTransform: "uppercase",
    color: "var(--editor-muted)",
    userSelect: "none",
  },
  ".cm-day-rule::before, .cm-day-rule::after": {
    content: '""',
    flex: "1",
    borderTop: "1px solid color-mix(in srgb, var(--editor-muted) 35%, transparent)",
  },
  // The label sits between the two rules; the trailing rule is deliberately
  // short so it reads as "———— date ——" rather than centring the date.
  ".cm-day-rule::after": {
    flex: "0 0 1.5rem",
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
  { dark },
);

const thingsHighlightStyle = HighlightStyle.define([
  // Headings — Things colors them per level (H1/H6 stay neutral)
  { tag: t.heading1, color: "var(--editor-fg)", fontWeight: "700", fontSize: "1.6em" },
  { tag: t.heading2, color: "var(--md-h2)", fontWeight: "700", fontSize: "1.4em" },
  { tag: t.heading3, color: "var(--md-h3)", fontWeight: "600", fontSize: "1.2em" },
  { tag: t.heading4, color: "var(--md-h4)", fontWeight: "600" },
  { tag: t.heading5, color: "var(--md-h5)", fontWeight: "600" },
  { tag: t.heading6, color: "var(--editor-muted)", fontWeight: "600" },
  // Inline emphasis — Things uses pink for both bold and italic
  { tag: t.strong, color: "var(--md-strong)", fontWeight: "700" },
  { tag: t.emphasis, color: "var(--md-em)", fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  // Links & references
  { tag: [t.link, t.url], color: "var(--editor-accent)", textDecoration: "underline" },
  // Code — only inline/fenced code is monospace (NOT all content)
  {
    tag: t.monospace,
    fontFamily: "var(--font-mono)",
    fontSize: "0.9em",
    background: "var(--editor-code-bg)",
    borderRadius: "4px",
    padding: "0.1em 0.3em",
  },
  // Markdown syntax punctuation (#, *, -, etc.) — muted
  {
    tag: [t.processingInstruction, t.meta, t.contentSeparator],
    color: "var(--editor-muted)",
  },
  // Lists & quotes. NB: lezer-markdown tags *every* descendant of a list with
  // t.list ("BulletList/..."), so coloring t.list would tint list text — and
  // override bold/italic inside lists. Leave list content at the default fg to
  // match preview; list markers stay muted via processingInstruction above.
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

// Built per-mode: the editor chrome theme needs the correct `dark` flag (so
// CodeMirror picks matching defaults), while the syntax highlight style is
// var-driven and shared. Caller reconfigures this when the theme mode changes.
export const thingsTheme = (dark: boolean): Extension => [
  thingsEditorTheme(dark),
  syntaxHighlighting(thingsHighlightStyle),
];
