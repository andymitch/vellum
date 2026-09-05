// Markdown export/import (#79) and "email this note" (#105). Vault data stays
// backend-owned — these helpers only move bytes between a file and the backend.
//
// Saving, picking and opening a URL all go through host.ts, so one flow covers
// the desktop/Android dialogs and the browser's download/upload (#221).
import { baseName, openExternal, pickFile, saveFile } from "./host";
import { exportVault, importVault, readNote, createNote, writeNote } from "./vault";

// Export the whole vault as a .zip of .md files. Returns false if cancelled.
export async function exportVaultZip(vault: string, vaultName: string): Promise<boolean> {
  const bytes = await exportVault(vault);
  return saveFile(`${vaultName || "vault"}.zip`, bytes, {
    label: "Zip archive",
    extensions: ["zip"],
  });
}

// Import a .zip of .md files into the vault. Returns the number of notes added,
// or null if cancelled. The backend validates the archive on parse.
export async function importVaultZip(vault: string): Promise<number | null> {
  const file = await pickFile(".zip,application/zip");
  if (!file) return null;
  return importVault(vault, file.bytes);
}

// Export a single note as a .md file. Returns false if cancelled.
export async function exportNoteMd(vault: string, notePath: string): Promise<boolean> {
  const content = await readNote(vault, notePath);
  return saveFile(baseName(notePath), content, { label: "Markdown", extensions: ["md"] });
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
  await openExternal(url);
}

// Import a single .md file into `dir` ("" = root). Returns the created path, or
// null if cancelled.
export async function importNoteMd(vault: string, dir: string): Promise<string | null> {
  const file = await pickFile(".md,.markdown,.txt,text/markdown,text/plain");
  if (!file) return null;
  const text = new TextDecoder().decode(file.bytes);
  const name = baseName(file.name).replace(/\.(markdown|txt)$/i, ".md");
  const dest = dir ? `${dir}/${name}` : name;
  const created = await createNote(vault, dest);
  await writeNote(vault, created, text);
  return created;
}
