// Note types (#104). A note's type lives in a YAML frontmatter block at the top
// of its Markdown:
//
//     ---
//     type: journal
//     ---
//     entry text
//
// Frontmatter was chosen over a sidecar metadata entry precisely because it is
// just text: it syncs through the same CRDT as the body, survives .md
// export/import, and there is no parallel record for rename/delete to keep in
// step — which is the failure mode that produced #167.
//
// This module is the only place that knows about the block. Everything else
// treats a note as ordinary Markdown.

export type NoteType = "markdown" | "journal";

export type NoteTypeInfo = {
  id: NoteType;
  label: string;
  /// Seeded into a newly created note of this type, after the frontmatter.
  initialBody: string;
  /// Typed notes render one way only — no source/preview toggle.
  singleView: boolean;
};

export const NOTE_TYPES: NoteTypeInfo[] = [
  { id: "markdown", label: "Markdown", initialBody: "", singleView: false },
  { id: "journal", label: "Journal", initialBody: "", singleView: true },
];

export const noteTypeInfo = (t: NoteType): NoteTypeInfo =>
  NOTE_TYPES.find((n) => n.id === t) ?? NOTE_TYPES[0];

// `scratchpad` was the name in the v8 betas before the type was simplified into
// Journal (#181). `todo` was a dedicated checklist type before it was dropped
// (#243) in favor of plain Markdown, which already renders task lines as
// checkboxes. Read both as aliases so an existing note keeps behaving the same
// way rather than silently reverting to a different type.
const ALIASES: Record<string, NoteType> = { scratchpad: "journal", todo: "markdown" };

const asType = (s: string): NoteType | null =>
  NOTE_TYPES.some((t) => t.id === s) ? (s as NoteType) : (ALIASES[s] ?? null);

export type ParsedNote = {
  type: NoteType;
  /// The note minus its frontmatter block.
  body: string;
  /// The raw frontmatter block including both `---` fences and its trailing
  /// newline, or "" when the note has none. `frontmatter + body === original`.
  frontmatter: string;
};

// A frontmatter block only counts at the very start of the note. This matters:
// a `---` mid-document is a thematic break the user typed, and one of those
// must never be mistaken for frontmatter.
const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/;

/// Split a note into its frontmatter and body, and read its type. A note with no
/// frontmatter — or with an unrecognised `type:` — reads as "markdown", so every
/// note that already exists keeps working and a note authored elsewhere is never
/// broken by us.
export function parseNote(text: string): ParsedNote {
  const m = FRONTMATTER_RE.exec(text);
  if (!m) return { type: "markdown", body: text, frontmatter: "" };
  const frontmatter = m[0];
  const body = text.slice(frontmatter.length);
  // Deliberately not a YAML parser: we own this block and only read one scalar
  // key from it. Anything else in there is preserved untouched by `withType`.
  const typeLine = /^[ \t]*type[ \t]*:[ \t]*(\S+)[ \t]*$/m.exec(m[1]);
  const raw = typeLine?.[1]?.replace(/^["']|["']$/g, "") ?? "";
  return { type: asType(raw) ?? "markdown", body, frontmatter };
}

/// The type of a note, without needing the rest of the split.
export const noteTypeOf = (text: string): NoteType => parseNote(text).type;

/// Return `text` carrying `type`, adding, replacing or removing the frontmatter
/// block as needed. The body is never touched.
export function withType(text: string, type: NoteType): string {
  const { body, frontmatter } = parseNote(text);
  if (type === "markdown") {
    // The default type is the absence of a marker, so a plain note stays plain
    // rather than gaining a block that says nothing.
    if (!frontmatter) return text;
    const inner = stripTypeKey(frontmatter);
    return inner ? inner + body : body;
  }
  if (!frontmatter) return `---\ntype: ${type}\n---\n${body}`;
  const existing = /^[ \t]*type[ \t]*:.*$/m;
  if (existing.test(frontmatter)) {
    return frontmatter.replace(existing, `type: ${type}`) + body;
  }
  // Frontmatter present but no type key — insert one after the opening fence,
  // keeping whatever else the block holds.
  return frontmatter.replace(/^---\r?\n/, `---\ntype: ${type}\n`) + body;
}

/// Drop just the `type:` line from a frontmatter block, returning "" if that
/// leaves the block empty (so we don't strand an empty `---\n---`).
function stripTypeKey(frontmatter: string): string {
  const withoutType = frontmatter.replace(/^[ \t]*type[ \t]*:.*(?:\r?\n)?/m, "");
  const inner = /^---\r?\n([\s\S]*?)\r?\n?---/.exec(withoutType)?.[1] ?? "";
  return inner.trim() ? withoutType : "";
}

/// A new note of `type`: its frontmatter plus the type's starter body.
export const newNoteContent = (type: NoteType): string =>
  withType(noteTypeInfo(type).initialBody, type);

// ---- Journal helpers ----

/// One cell of a Journal note (#227) — a long-running note divided into
/// separately editable chunks, not a file-per-day log or a chat transcript.
/// `created` is the ISO 8601 instant the cell was made, and `updated` the
/// instant its text last changed (present only once it differs from
/// `created`, i.e. the cell has actually been edited since). Both are null
/// for content with no marker line (legacy content from before this format,
/// or something a peer wrote by hand) — kept rather than discarded, since this
/// module never destroys content it doesn't recognize.
export type JournalCell = { created: string | null; updated: string | null; text: string };

/// A cell's marker: an HTML comment on its own line, invisible wherever the
/// note is rendered (this app or any other), so the file still reads as
/// plain prose everywhere except here. Holds the created time, and — once
/// the cell has been edited — the updated time after it, space-separated.
const TIME_MARKER = /^<!--[ \t]*(\S+)(?:[ \t]+(\S+))?[ \t]*-->\r?\n?/;

/// Split a Journal note's body into cells for the notebook UI. Cells are
/// separated by a blank line, same as ordinary Markdown paragraphs.
export function parseJournalCells(text: string): JournalCell[] {
  const { body } = parseNote(text);
  return body
    .split(/\n{2,}/)
    .map((chunk) => chunk.trim())
    .filter(Boolean)
    .map((chunk) => {
      const m = TIME_MARKER.exec(chunk);
      if (!m || Number.isNaN(Date.parse(m[1]))) return { created: null, updated: null, text: chunk };
      const updated = m[2] && !Number.isNaN(Date.parse(m[2])) ? m[2] : null;
      return { created: m[1], updated, text: chunk.slice(m[0].length) };
    });
}

/// Rebuild the note from its cells, preserving the frontmatter untouched.
export function serializeJournalCells(original: string, cells: JournalCell[]): string {
  const { frontmatter } = parseNote(original);
  const body = cells
    .map((c) => {
      if (!c.created) return c.text;
      const marker = c.updated && c.updated !== c.created ? `${c.created} ${c.updated}` : c.created;
      return `<!-- ${marker} -->\n${c.text}`;
    })
    .join("\n\n");
  return frontmatter + body + (body ? "\n" : "");
}
