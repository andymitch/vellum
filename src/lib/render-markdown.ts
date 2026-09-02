// Markdown -> HTML rendering shared by every read-only render of note
// content — same extensions, same escaping, same checkbox handling, wherever
// it's used.
//
// Deliberately excludes DOM-mutation effects such as heading-slug/wikilink
// resolution, Mermaid diagram rendering, and link-preview card fetching:
// those are async, per-full-note effects that only make sense applied once,
// over a whole rendered note, not for every small piece of content rendered
// through this module.

import { Marked, type TokenizerAndRendererExtension } from "marked";
import { markedHighlight } from "marked-highlight";
import hljs from "highlight.js/lib/common";
import { TAG_RE, TAG_START_RE, trimTag } from "./tags";

const escHtml = (s: string) =>
  s.replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[c]!);
const escAttr = (s: string) => escHtml(s).replace(/"/g, "&quot;");

// `[[target]]`, `[[target|label]]`, `[[target#heading]]`, or `[[#heading]]`
// (same note) → an internal link. Resolution against the vault's notes happens
// after render, since the note list is reactive and lives outside this marked
// instance — see Preview.svelte's wikilink-resolution effect.
const wikiLink: TokenizerAndRendererExtension = {
  name: "wikilink",
  level: "inline",
  start(src) {
    return src.indexOf("[[");
  },
  tokenizer(src) {
    const m = /^\[\[([^\]\n]+?)\]\]/.exec(src);
    if (!m) return;
    const [link, label] = m[1].split("|");
    const hash = link.indexOf("#");
    const target = (hash >= 0 ? link.slice(0, hash) : link).trim();
    const fragment = hash >= 0 ? link.slice(hash + 1).trim() : "";
    return {
      type: "wikilink",
      raw: m[0],
      target,
      fragment,
      label: (label ?? link).trim(),
    };
  },
  renderer(token) {
    return `<a href="#" class="wikilink" data-target="${escAttr(token.target)}" data-fragment="${escAttr(token.fragment)}">${escHtml(token.label)}</a>`;
  },
};

// Inline `#tag` -> a clickable chip. The rules live in $lib/tags (shared with
// the source-mode decoration, and mirroring `extract_tags` in vault.rs) so
// what renders as a tag is what is findable as one. marked only offers us the
// start of an inline token, so the "preceded by whitespace" half of the rule
// is enforced by `start` returning a boundary-anchored match.
const tagChip: TokenizerAndRendererExtension = {
  name: "tagchip",
  level: "inline",
  start(src) {
    const m = TAG_START_RE.exec(src);
    if (!m) return;
    // Point at the '#', not at the whitespace before it.
    return m.index + (m[0].startsWith("#") ? 0 : 1);
  },
  tokenizer(src) {
    const m = TAG_RE.exec(src);
    if (!m) return;
    const tag = trimTag(m[1]);
    if (!tag) return;
    return { type: "tagchip", raw: m[0], tag };
  },
  renderer(token) {
    return `<a href="#" class="tagchip" data-tag="${escAttr(token.tag)}">#${escHtml(token.tag)}</a>`;
  },
};

// Local Marked instance with highlight.js. We emit hljs token classes and
// style them via our --code-* vars (same palette as the editor), so no
// highlight.js theme stylesheet is imported.
const marked = new Marked(
  { gfm: true, breaks: false },
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code, lang) {
      const language = hljs.getLanguage(lang) ? lang : "plaintext";
      return hljs.highlight(code, { language }).value;
    },
  }),
);
marked.use({ extensions: [wikiLink, tagChip] });

const stripExt = (s: string) => s.replace(/\.[a-z0-9]+$/i, "");

/// Resolve a wiki-link target to an actual note path: try the path as-is and
/// with a .md extension (exact, then case-insensitive), then fall back to a
/// basename match so `[[todo]]` finds `work/todo.md`.
export function resolveWikiLink(target: string, notePaths: string[]): string | null {
  const t = target.replace(/\\/g, "/").replace(/^\/+/, "");
  if (!t) return null;
  const withMd = /\.[a-z0-9]+$/i.test(t) ? t : `${t}.md`;
  if (notePaths.includes(t)) return t;
  if (notePaths.includes(withMd)) return withMd;
  const lc = withMd.toLowerCase();
  const ci = notePaths.find((p) => p.toLowerCase() === lc);
  if (ci) return ci;
  const base = stripExt(t.split("/").pop() ?? "").toLowerCase();
  return notePaths.find((p) => stripExt(p.split("/").pop() ?? "").toLowerCase() === base) ?? null;
}

/// Render Markdown body text to HTML. The caller's content is the user's own
/// local notes, rendered in a desktop app — no untrusted input — so the
/// output is safe to insert directly via `{@html}`. GFM task-list checkboxes
/// are rendered `disabled` by marked; strip that so they're interactive
/// (toggling rewrites the source — see Preview.svelte's onToggle).
export function renderMarkdown(body: string): string {
  return (marked.parse(body) as string).replace(
    /(<input\b[^>]*?)\s+disabled(?:="")?/g,
    "$1",
  );
}
