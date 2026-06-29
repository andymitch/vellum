// Markdown export/import (#79). The dialog plugin picks the file; the fs plugin
// reads/writes it (and abstracts Android SAF content URIs). Vault data stays
// backend-owned — these helpers only move bytes between a file and the backend.
import { save, open } from "@tauri-apps/plugin-dialog";
import { readFile, writeFile, readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { exportVault, importVault, readNote, createNote, writeNote } from "./vault";

const baseName = (p: string) => p.replace(/\\/g, "/").split("/").pop() ?? p;

// Export the whole vault as a .zip of .md files. Returns false if cancelled.
export async function exportVaultZip(vault: string, vaultName: string): Promise<boolean> {
  const bytes = await exportVault(vault);
  const path = await save({
    defaultPath: `${vaultName || "vault"}.zip`,
    filters: [{ name: "Zip archive", extensions: ["zip"] }],
  });
  if (!path) return false;
  await writeFile(path, new Uint8Array(bytes));
  return true;
}

// Import a .zip of .md files into the vault. Returns the number of notes added,
// or null if cancelled.
export async function importVaultZip(vault: string): Promise<number | null> {
  // No extension filter — Android SAF maps extensions to MIME types unreliably
  // (a zip can register as application/octet-stream and get hidden). The backend
  // validates the archive on parse.
  const sel = await open({ multiple: false });
  if (!sel) return null;
  const path = sel as string;
  const bytes = await readFile(path);
  return importVault(vault, Array.from(bytes));
}

// Export a single note as a .md file. Returns false if cancelled.
export async function exportNoteMd(vault: string, notePath: string): Promise<boolean> {
  const content = await readNote(vault, notePath);
  const path = await save({
    defaultPath: baseName(notePath),
    filters: [{ name: "Markdown", extensions: ["md"] }],
  });
  if (!path) return false;
  await writeTextFile(path, content);
  return true;
}

// Import a single .md file into `dir` ("" = root). Returns the created path, or
// null if cancelled.
export async function importNoteMd(vault: string, dir: string): Promise<string | null> {
  const sel = await open({ multiple: false });
  if (!sel) return null;
  const path = sel as string;
  const text = await readTextFile(path);
  const name = baseName(path).replace(/\.(markdown|txt)$/i, ".md");
  const dest = dir ? `${dir}/${name}` : name;
  const created = await createNote(vault, dest);
  await writeNote(vault, created, text);
  return created;
}
