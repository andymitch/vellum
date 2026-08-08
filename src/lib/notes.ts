import { createNote, writeNote, readNote, type TreeNode } from "./vault";
import { newNoteContent, type NoteType } from "./note-type";

/// Create an empty note named `name` in `dir` (empty string = vault root) and
/// open it. The filename is independent of the content — callers prompt for the
/// name. `createNote` de-duplicates against existing siblings. Returns the path.
export async function createAndOpenNote(
  vault: string,
  dir: string,
  name: string,
  open: (vault: string, path: string, focus?: boolean) => void,
  type: NoteType = "markdown",
): Promise<string> {
  const file = name.endsWith(".md") ? name : `${name}.md`;
  const path = await createNote(vault, dir ? `${dir}/${file}` : file);
  // Seed the type's frontmatter + starter body (#104). "markdown" seeds nothing,
  // so an ordinary note is created exactly as it always was.
  const seed = newNoteContent(type);
  if (seed) await writeNote(vault, path, seed);
  // A brand-new note opens in source mode with the editor focused so the user
  // can type immediately (#50).
  open(vault, path, true);
  return path;
}

function* walkFiles(nodes: TreeNode[]): Generator<TreeNode> {
  for (const n of nodes) {
    if (!n.is_dir) yield n;
    if (n.children) yield* walkFiles(n.children);
  }
}

/// Duplicate the note at `path` into the same folder as "X (copy)" (or
/// "X (copy N)"). Content is copied verbatim — the filename is independent of
/// it. `tree` supplies the sibling names to dedupe against. Returns the new
/// note's path.
export async function duplicateNote(
  vault: string,
  path: string,
  tree: TreeNode[],
): Promise<string> {
  const md = await readNote(vault, path);
  const slash = path.lastIndexOf("/");
  const dir = slash === -1 ? "" : path.slice(0, slash);
  const stem = path.slice(slash + 1).replace(/\.md$/, "");
  const siblings = new Set(
    [...walkFiles(tree)]
      .map((n) => {
        const s = n.path.lastIndexOf("/");
        return { d: s === -1 ? "" : n.path.slice(0, s), name: n.path.slice(s + 1) };
      })
      .filter((x) => x.d === dir)
      .map((x) => x.name),
  );
  let name = `${stem} (copy).md`;
  for (let i = 2; siblings.has(name); i++) name = `${stem} (copy ${i}).md`;
  const dest = dir ? `${dir}/${name}` : name;
  const created = await createNote(vault, dest);
  await writeNote(vault, created, md);
  return created;
}
