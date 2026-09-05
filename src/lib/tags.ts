// Inline `#tag` rules — the single frontend source of truth (#202).
//
// These mirror `extract_tags` in vault.rs, which is what search and the tag list
// use, so what renders as a tag is what is findable as one. The rules exist to
// keep tags from colliding with ordinary Markdown:
//
//   - `#` must start the text or follow whitespace, so `example.com/#anchor` and
//     `C#` inside a word are not tags.
//   - The character after `#` must be alphanumeric, which is exactly what
//     separates a tag from an ATX heading — `# Heading` has a space.
//   - `_`, `-` and `/` continue a tag (`#in/progress`, `#q3-goals`), but
//     trailing ones are trimmed, so `#work.` and `#work/` both yield `work`.
//
// Preview renders tags through marked and the editor decorates them in place, so
// both need the same rules. They lived in Preview.svelte alone until the source
// -mode decoration needed them too; a second copy would have drifted.

/// Matches a tag at the very start of the input. Used by marked's tokenizer,
/// which only ever hands us a slice beginning at a candidate.
export const TAG_RE = /^#([\p{L}\p{N}][\p{L}\p{N}_/-]*)/u;

/// Finds the next position where a tag could begin — start of text, or after
/// whitespace. marked's `start` hook needs this to locate candidates.
export const TAG_START_RE = /(?:^|\s)#[\p{L}\p{N}]/u;

/// Drop the trailing separators that `extract_tags` trims, so `#a/` is `a`.
export const trimTag = (tag: string) => tag.replace(/[-/_]+$/, "");

export type TagMatch = {
  /// Offset of the `#`.
  from: number;
  /// Offset one past the last character of the tag, trailing separators
  /// excluded — so the decorated range is exactly what the tag resolves to.
  to: number;
  /// The tag without its `#`, trimmed.
  tag: string;
};

/// Read a search query as a tag query, returning the bare (lower-cased) tag if
/// it is one — mirrors `as_tag_query` in vault.rs.
///
/// The palette leaves its tag-picker mode by rewriting the query, and a preview
/// chip seeds the query the same way, so a tag query can arrive with
/// surrounding whitespace; trimming here is what stops that whitespace becoming
/// part of the needle (#202). The body is validated with exactly the rules
/// `scanTags` uses, so a query only counts as a tag when it could have been
/// produced as one.
export function asTagQuery(query: string): string | null {
  const s = query.trim();
  if (!s.startsWith("#")) return null;
  const body = s.slice(1);
  if (!/^[\p{L}\p{N}][\p{L}\p{N}_/-]*$/u.test(body)) return null;
  const tag = trimTag(body);
  return tag ? tag.toLowerCase() : null;
}

/// The distinct tags in a note, in order of appearance, de-duplicated
/// case-insensitively — the `extract_tags` contract, which the vault's tag
/// counts and tag search are defined in terms of.
export function distinctTags(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const { tag } of scanTags(text)) {
    const key = tag.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(tag);
  }
  return out;
}

/// Whether `text` carries `tag` as a real inline tag rather than as loose text.
export const hasTag = (text: string, tag: string): boolean =>
  distinctTags(text).some((t) => t.toLowerCase() === tag.toLowerCase());

/// Scan `text` for inline tags, in order of appearance. Unlike `extract_tags`
/// this keeps positions and does not de-duplicate, because callers decorate
/// every occurrence rather than listing distinct tags.
export function scanTags(text: string): TagMatch[] {
  const out: TagMatch[] = [];
  for (let i = 0; i < text.length; i++) {
    if (text[i] !== "#") continue;
    // Start of text or preceded by whitespace, matching the Rust boundary rule.
    if (i > 0 && !/\s/.test(text[i - 1])) continue;
    const m = TAG_RE.exec(text.slice(i));
    if (!m) continue;
    const tag = trimTag(m[1]);
    if (tag) out.push({ from: i, to: i + 1 + tag.length, tag });
    // Skip the whole raw match, trimmed characters included, so a trailing
    // separator can't be read as the start of another tag.
    i += m[0].length - 1;
  }
  return out;
}
