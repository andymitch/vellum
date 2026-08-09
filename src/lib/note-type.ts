// Note types (#104). A note's type lives in a YAML frontmatter block at the top
// of its Markdown:
//
//     ---
//     type: todo
//     ---
//     - [ ] buy milk
//
// Frontmatter was chosen over a sidecar metadata entry precisely because it is
// just text: it syncs through the same CRDT as the body, survives .md
// export/import, and there is no parallel record for rename/delete to keep in
// step — which is the failure mode that produced #167.
//
// This module is the only place that knows about the block. Everything else
// treats a note as ordinary Markdown.

export type NoteType = "markdown" | "todo" | "journal";

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
  { id: "todo", label: "TODO list", initialBody: "- [ ] ", singleView: true },
  { id: "journal", label: "Journal", initialBody: "", singleView: true },
];

export const noteTypeInfo = (t: NoteType): NoteTypeInfo =>
  NOTE_TYPES.find((n) => n.id === t) ?? NOTE_TYPES[0];

// `scratchpad` was the name in the v8 betas before the type was simplified into
// Journal (#181). Read it as an alias so a beta-era note keeps working rather
// than silently reverting to plain Markdown.
const ALIASES: Record<string, NoteType> = { scratchpad: "journal" };

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

// ---- TODO helpers ----

// A GFM task line, capturing the checkbox state. Mirrors the marker rewriting
// Preview.svelte already does when a checkbox is clicked.
const TASK_LINE = /^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[([ xX])\]/;

/// How many checked items the note has — drives whether the sweep button shows.
export const countChecked = (text: string): number =>
  text.split("\n").filter((l) => /^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[[xX]\]/.test(l)).length;

/// Remove every checked task line. Unchecked items, prose, and any frontmatter
/// are left exactly as they were.
export function sweepChecked(text: string): string {
  const { body, frontmatter } = parseNote(text);
  const kept = body
    .split("\n")
    .filter((l) => !/^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[[xX]\]/.test(l));
  return frontmatter + kept.join("\n");
}

/// The `- [ ] ` continuation for pressing Enter on a task line, or null when the
/// cursor isn't on one (so Enter behaves normally).
export function taskContinuation(line: string): string | null {
  const m = TASK_LINE.exec(line);
  if (!m) return null;
  // An empty task line means "stop the list" — the same convention editors use
  // for bullets — so Enter there ends it rather than adding another empty box.
  if (/^[ \t]*(?:[-*+]|\d+[.)])[ \t]+\[[ xX]\][ \t]*$/.test(line)) return null;
  const indent = /^[ \t]*/.exec(line)?.[0] ?? "";
  return `${indent}- [ ] `;
}

// ---- Scratchpad helpers ----

/// `YYYY-MM-DD` in local time. Not UTC: "a new day" means the user's day, and
/// toISOString would roll over at the wrong moment for most of the world.
export function localDay(d: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/// A day section is a single heading line. Dropping the `---` the scratchpad
/// used to write removes the ambiguity with the frontmatter fence entirely, and leaves
/// one line to decorate as the dated rule rather than two.
export const daySeparator = (day: string = localDay()) => `\n\n## ${day}\n\n`;

/// A section heading line, e.g. `## 2026-08-08`. The ISO date stays in the file
/// so the note still sorts and greps; only the rendering is human-formatted.
export const DAY_HEADING = /^##[ \t]+(\d{4})-(\d{2})-(\d{2})[ \t]*$/;

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `2026-01-01` -> `Jan 01, 2026`, for the inline label on the rule.
export function formatDay(iso: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!m) return iso;
  const month = MONTHS[Number(m[2]) - 1] ?? m[2];
  return `${month} ${m[3]}, ${m[1]}`;
}

/// Whether the note already carries a section for `day`.
export const hasDaySection = (text: string, day: string = localDay()): boolean =>
  new RegExp(`^##[ \\t]+${day}[ \\t]*$`, "m").test(text);

/// Append a dated separator for today unless the note already has one.
///
/// Idempotent on purpose: this runs on the first edit of a day, and two synced
/// devices can both reach that point on the same morning. Checking for the
/// marker first means the loser of that race adds nothing rather than producing
/// a duplicate heading.
export function ensureDaySection(text: string, day: string = localDay()): string {
  if (hasDaySection(text, day)) return text;
  const { body, frontmatter } = parseNote(text);
  // A brand-new journal opens straight into its first day, with no leading
  // rule above it.
  if (!body.trim()) return `${frontmatter}## ${day}\n\n`;
  return text.replace(/\s*$/, "") + daySeparator(day);
}
