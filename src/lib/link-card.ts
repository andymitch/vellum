// Turning a note's Markdown into the title + excerpt shown on an internal link
// preview card (#62).
//
// Kept separate from Preview.svelte because it is pure string work with a lot of
// edge cases, and because the card must read like prose: a card showing
// "## Overview" or "- [ ] thing" is worse than the plain link it replaced.

import { parseNote } from "./note-type";

export type NoteCard = { title: string; excerpt: string };

/// Strip the inline Markdown that would otherwise show up as literal punctuation
/// in a one-line excerpt. Block syntax is handled by line filtering below.
function plainText(line: string): string {
  return (
    line
      // Images before links — `![alt](src)` would otherwise leave a stray "!".
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      // Wiki links keep their label, which is the half a reader cares about.
      .replace(/\[\[([^\]|]+)\|([^\]]+)\]\]/g, "$2")
      .replace(/\[\[([^\]]+)\]\]/g, "$1")
      .replace(/`([^`]*)`/g, "$1")
      .replace(/[*_~]{1,3}(?=\S)([\s\S]*?\S)[*_~]{1,3}/g, "$1")
      .replace(/\s+/g, " ")
      .trim()
  );
}

/// Lines that carry no prose: fences, rules, blockquote markers, table pipes,
/// and the frontmatter that `parseNote` has already removed from the body.
function isStructural(line: string): boolean {
  const t = line.trim();
  return (
    !t ||
    t.startsWith("```") ||
    t.startsWith("~~~") ||
    t.startsWith("|") ||
    /^([-*_])\s*\1\s*\1[\s\-*_]*$/.test(t) // thematic break: ---, ***, - - -
  );
}

/// Drop the leading list/quote/heading marker so "- [ ] Buy milk" excerpts as
/// "Buy milk" rather than as its own punctuation.
function stripMarker(line: string): string {
  return line
    .replace(/^\s{0,3}>+\s?/, "")
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/, "")
    .replace(/^\s*\[[ xX]\]\s*/, "")
    .replace(/^\s{0,3}#{1,6}\s+/, "");
}

const MAX_EXCERPT = 180;

/// Build the card for a note.
///
/// The title prefers the note's own first heading — that is what the author
/// called it — and falls back to the filename. The excerpt is the first line of
/// actual prose after that heading.
export function noteCard(markdown: string, path: string): NoteCard {
  const body = parseNote(markdown).body;
  const lines = body.split(/\r?\n/);

  const fileTitle = (path.split("/").pop() ?? path).replace(/\.md$/i, "");

  let title = "";
  const prose: string[] = [];
  let inFence = false;
  for (const raw of lines) {
    const t = raw.trim();
    // Never read inside a code fence — its contents are not prose.
    if (t.startsWith("```") || t.startsWith("~~~")) {
      inFence = !inFence;
      continue;
    }
    if (inFence || isStructural(raw)) continue;
    const heading = /^\s{0,3}#{1,6}\s+(.*)$/.exec(raw);
    if (heading) {
      // The first heading names the note; later ones are section labels and
      // make poor excerpt text, so they're skipped either way.
      if (!title) title = plainText(heading[1]);
      continue;
    }
    const text = plainText(stripMarker(raw));
    if (text) prose.push(text);
    // Two lines is plenty to fill the excerpt.
    if (prose.length >= 2) break;
  }

  let excerpt = prose.join(" ");
  if (excerpt.length > MAX_EXCERPT) {
    // Cut on a word boundary so the ellipsis doesn't land mid-word.
    const cut = excerpt.slice(0, MAX_EXCERPT);
    const sp = cut.lastIndexOf(" ");
    excerpt = `${(sp > MAX_EXCERPT * 0.6 ? cut.slice(0, sp) : cut).trimEnd()}…`;
  }

  return { title: title || fileTitle, excerpt };
}
