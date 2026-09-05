// Host services that differ between the Tauri shell and a plain browser (#221):
// opening an external URL, saving a file, picking one.
//
// Each capability is branched in exactly one place here, so call sites stay
// shell-agnostic. The Tauri paths are the ones that were inline before: the
// dialog plugin picks the path and the fs plugin moves the bytes (which is also
// what abstracts Android's SAF content URIs). The browser paths are an <a
// download> and a hidden <input type="file">.

import { save, open } from "@tauri-apps/plugin-dialog";
import { readFile, writeFile } from "@tauri-apps/plugin-fs";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isTauri } from "./platform";

/// Open a URL outside the app: the OS handler in Tauri, a new tab in a browser.
/// `mailto:` goes through location assignment instead of window.open, which
/// several browsers block for non-http schemes (and which would otherwise leave
/// a blank tab behind).
export async function openExternal(url: string): Promise<void> {
  if (isTauri) {
    await openUrl(url);
    return;
  }
  if (/^(mailto|tel|sms):/i.test(url)) {
    window.location.href = url;
    return;
  }
  window.open(url, "_blank", "noopener");
}

export type SaveFilter = { label: string; extensions: string[] };

/// Save `data` under a suggested `name`. Returns false only when the user
/// cancelled the desktop/Android save dialog — a browser download is handed to
/// the browser and gives no cancellable answer, so it always reports true.
export async function saveFile(
  name: string,
  data: Uint8Array | string,
  filter?: SaveFilter,
): Promise<boolean> {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  if (isTauri) {
    const path = await save({
      defaultPath: name,
      filters: filter ? [{ name: filter.label, extensions: filter.extensions }] : undefined,
    });
    if (!path) return false;
    await writeFile(path, bytes);
    return true;
  }
  // Blob over a fresh ArrayBuffer copy: a Uint8Array view of a larger buffer
  // would otherwise be written in full.
  const blob = new Blob([bytes.slice().buffer as ArrayBuffer], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  document.body.append(a);
  a.click();
  a.remove();
  // Revoke on the next tick; revoking synchronously can beat the download in
  // WebKit.
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return true;
}

export type PickedFile = { name: string; bytes: Uint8Array };

/// Pick a single file and read it. Resolves null when the user cancels.
///
/// `accept` is a browser `<input accept>` list and deliberately has no Tauri
/// equivalent here: Android's SAF maps extensions to MIME types unreliably (a
/// .zip can register as application/octet-stream and get hidden), so the native
/// dialog stays unfiltered and the caller validates what it got.
export async function pickFile(accept?: string): Promise<PickedFile | null> {
  if (isTauri) {
    const sel = await open({ multiple: false });
    if (!sel) return null;
    const path = sel as string;
    return { name: baseName(path), bytes: await readFile(path) };
  }
  const input = document.createElement("input");
  input.type = "file";
  if (accept) input.accept = accept;
  input.style.display = "none";
  document.body.append(input);
  try {
    const file = await new Promise<File | null>((resolve) => {
      input.addEventListener("change", () => resolve(input.files?.[0] ?? null), { once: true });
      // Supported in current Safari/Chrome/Firefox; where it isn't, a cancelled
      // picker simply never resolves, which reads to the user as "nothing
      // happened" — the same outcome as cancelling.
      input.addEventListener("cancel", () => resolve(null), { once: true });
      input.click();
    });
    if (!file) return null;
    return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
  } finally {
    input.remove();
  }
}

export const baseName = (p: string) => p.replace(/\\/g, "/").split("/").pop() ?? p;
