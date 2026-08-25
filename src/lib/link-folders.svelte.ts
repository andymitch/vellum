// Linked folders (#219) — the backend is the source of truth (links.json in
// the app data dir), so this store reads from it rather than mirroring a
// localStorage value, same reasoning as mcp.svelte.ts.

import { listLinks, addLink, removeLink, setLinkEnabled, type LinkInfo } from "./vault";

let links = $state<LinkInfo[]>([]);
let busy = $state(false);

/// Read the current links once at startup. Fire-and-forget: on mobile the
/// command reports an empty list, and a failure just leaves it empty.
export async function initLinkFolders() {
  try {
    links = await listLinks();
  } catch {
    links = [];
  }
}

export const linkFolders = {
  get all() {
    return links;
  },
  get busy() {
    return busy;
  },
  async add(vault: string, folder: string) {
    if (busy) return null;
    busy = true;
    try {
      const created = await addLink(vault, folder);
      links = [...links, created];
      return created;
    } finally {
      busy = false;
    }
  },
  async remove(id: string) {
    if (busy) return;
    busy = true;
    try {
      await removeLink(id);
      links = links.filter((l) => l.id !== id);
    } finally {
      busy = false;
    }
  },
  async toggle(id: string, enabled: boolean) {
    if (busy) return;
    busy = true;
    try {
      const updated = await setLinkEnabled(id, enabled);
      links = links.map((l) => (l.id === id ? updated : l));
    } catch (e) {
      console.error("[link-folders] toggle failed", e);
      // Re-read rather than assume, same reasoning as mcp.svelte.ts's toggle.
      await initLinkFolders();
    } finally {
      busy = false;
    }
  },
};
