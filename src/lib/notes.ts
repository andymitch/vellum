import { createNote, writeNote } from "./vault";

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
