import { createNote, writeNote, readNote, type TreeNode } from "./vault";

/// Create a new "Untitled" note in `dir` (empty string = vault root), seed it
/// with an H1 title, and open it for editing with the title preselected. Returns
/// the note's final path. Shared by the sidebar New Note button and the mobile FAB.
export async function createAndOpenNote(
  vault: string,
  dir: string,
  open: (vault: string, path: string, selectTitle?: boolean) => void,
): Promise<string> {
  const path = await createNote(vault, dir ? `${dir}/Untitled.md` : "Untitled.md");
  const title = path.split("/").pop()!.replace(/\.md$/, "");
  const finalPath = await writeNote(vault, path, `# ${title}\n`);
  open(vault, finalPath, true);
  return finalPath;
}

function* walkFiles(nodes: TreeNode[]): Generator<TreeNode> {
  for (const n of nodes) {
    if (!n.is_dir) yield n;
    if (n.children) yield* walkFiles(n.children);
  }
}

/// Duplicate the note at `path` into the same folder as "X (copy)" (or
/// "X (copy N)"), rewriting the first H1 to match the new name so the in-doc
/// title and filename stay in sync. `tree` supplies the sibling names to dedupe
/// against. Returns the new note's path.
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
  // Rewrite the first H1 so the in-doc title matches the new filename (the
  // filename follows the H1, so leaving it stale would rename the copy back).
  const newStem = name.replace(/\.md$/, "");
  const dupMd = md.replace(/^#[ \t]+.*$/m, `# ${newStem}`);
  const created = await createNote(vault, dest);
  return writeNote(vault, created, dupMd, false);
}
