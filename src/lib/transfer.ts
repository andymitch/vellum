// Markdown export/import (#79). The dialog plugin picks the file; the fs plugin
// reads/writes it (and abstracts Android SAF content URIs). Vault data stays
// backend-owned — these helpers only move bytes between a file and the backend.
import { save, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
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

// Practical ceiling for a mailto: URL. There is no spec limit, but handlers cut
// long ones off silently — so truncate deliberately and say so in the body
// rather than letting a note end mid-sentence with no explanation.
const MAILTO_MAX = 8000;

// Email a note through the default mail client (#105).
//
// The OS share sheet only lists apps that register a *share extension*, which
// several mail clients don't — on a machine whose default handler is one of
// them, "share via email" simply isn't offered. `mailto:` has no such gap: it
// goes to whatever client actually handles mail, on both macOS and Android.
export async function emailNote(vault: string, notePath: string): Promise<void> {
  const content = await readNote(vault, notePath);
  const subject = baseName(notePath).replace(/\.md$/i, "");
  let body = content;
  if (body.length > MAILTO_MAX) {
    body = `${body.slice(0, MAILTO_MAX)}\n\n[…truncated — the full note is longer than an email link can carry]`;
  }
  // encodeURIComponent, not encodeURI: the note body legitimately contains &, #
  // and ? characters, which would otherwise be read as URL syntax.
  const url = `mailto:?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
  await openUrl(url);
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
