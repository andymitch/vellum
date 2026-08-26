// Thin typed wrappers over the Rust commands. All logic lives in the backend;
// this is just the invoke surface.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type VaultInfo = { id: string; name: string; pending: boolean; hash: string };
export type TreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  children: TreeNode[];
};

export const listVaults = () => invoke<VaultInfo[]>("list_vaults");
export const createVault = (name: string) => invoke<VaultInfo>("create_vault", { name });
export const joinVault = (ticket: string) => invoke<VaultInfo>("join_vault", { ticket });
export const shareVault = (vault: string) => invoke<string>("share_vault", { vault });
export const forgetVault = (vault: string) => invoke<void>("forget_vault", { vault });
export const renameVault = (vault: string, name: string) =>
  invoke<void>("rename_vault", { vault, name });

export const listTree = (vault: string) => invoke<TreeNode[]>("list_tree", { vault });
export const readNote = (vault: string, path: string) =>
  invoke<string>("read_note", { vault, path });
// Writes note content. Filename and content are independent — renaming is an
// explicit file action (rename_path), never derived from the content. `base` is
// the text the editor loaded; the backend 3-way merges base→content against any
// concurrent peer edit so neither side is clobbered (#99). New notes pass "".
export const writeNote = (vault: string, path: string, content: string, base = "") =>
  invoke<void>("write_note", { vault, path, base, content });
// Returns the actual (possibly de-duplicated) path of the created note.
export const createNote = (vault: string, path: string) =>
  invoke<string>("create_note", { vault, path });
export const createFolder = (vault: string, path: string) =>
  invoke<void>("create_folder", { vault, path });
// Markdown export/import (#79). Bytes cross IPC as a number[]; the caller wraps
// them in a Uint8Array / Array.from for the fs plugin.
export const exportVault = (vault: string) => invoke<number[]>("export_vault", { vault });
export const importVault = (vault: string, data: number[]) =>
  invoke<number>("import_vault", { vault, data });
// Hand a note to the OS share sheet — email, Messages/SMS, AirDrop (#105).
// Backend-side because it needs AppKit / an Android intent; nothing crosses IPC
// but the ids, since the backend reads the saved note itself.
export const shareNote = (vault: string, path: string) =>
  invoke<void>("share_note", { vault, path });
export const renamePath = (vault: string, from: string, to: string, isDir: boolean) =>
  invoke<void>("rename_path", { vault, from, to, isDir });
export const deletePath = (vault: string, path: string, isDir: boolean) =>
  invoke<void>("delete_path", { vault, path, isDir });
export const watchVault = (vault: string) => invoke<void>("watch_vault", { vault });

// Toggle background "live sync": arm every vault as an always-on hub and flip
// the platform keep-alive (desktop tray + launch-at-login / Android foreground
// service) so syncing continues with no window open / while backgrounded.
export const setBackgroundSync = (enabled: boolean) =>
  invoke<void>("set_background_sync", { enabled });

export const onVaultChanged = (cb: (vaultId: string) => void): Promise<UnlistenFn> =>
  listen<string>("vault-changed", (e) => cb(e.payload));

// Emitted by the backend when background sync is changed outside the Settings
// toggle (e.g. "Turn off background sync" from the desktop tray).
export const onBackgroundSyncChanged = (cb: (on: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("background-sync", (e) => cb(e.payload));

// ---- local MCP server (#164) ----
// Lets agents (Claude Code, Claude Desktop, …) CRUD notes over a loopback,
// token-authenticated MCP endpoint hosted by this app. Desktop only — the
// mobile build answers with a disabled status.
export type McpStatus = {
  enabled: boolean;
  port: number | null;
  url: string | null;
  token: string;
  /// Ready-to-paste `claude mcp add …` line; null while stopped.
  command: string | null;
};

export const mcpStatus = () => invoke<McpStatus>("mcp_status");
export const setMcpEnabled = (enabled: boolean) =>
  invoke<McpStatus>("set_mcp_enabled", { enabled });

// ---- search + tags (#15) ----
// Both scan every note in the vault (each read is a CRDT merge), so callers
// should debounce rather than fire per keystroke.
export type SearchHit = {
  path: string;
  /// Matching lines as "{line}: {text}", at most 3 per note.
  lines: string[];
};
export type TagCount = { tag: string; count: number };

export const searchNotes = (vault: string, query: string, max?: number) =>
  invoke<SearchHit[]>("search_notes", { vault, query, max });
export const listTags = (vault: string) => invoke<TagCount[]>("list_tags", { vault });

// ---- link previews (#62) ----
// Open Graph metadata for an external link, fetched backend-side (CORS blocks
// nearly every cross-origin fetch from the webview, and a <meta> scrape needs no
// DOM). Results are cached in memory per URL. Resolves to null — not an error —
// for a non-http(s) URL, a failed request, or a page with no usable metadata,
// so the caller just keeps rendering a plain link.
export type LinkPreview = {
  url: string;
  title: string | null;
  description: string | null;
  site_name: string | null;
  image: string | null;
};
export const fetchLinkPreview = (url: string) =>
  invoke<LinkPreview | null>("fetch_link_preview", { url });

// Note types for the file tree's icons (#180/#181). Only typed notes come back —
// plain Markdown is the default and would be noise. This scans every note, so
// callers should debounce rather than refetch on each vault-changed event.
export type NoteTypeEntry = { path: string; note_type: string };
export const listNoteTypes = (vault: string) =>
  invoke<NoteTypeEntry[]>("list_note_types", { vault });

// ---- linked folders (#219) ----
// Mirrors a vault folder to a directory under app storage, kept in sync both
// ways, with a friendly `~/.vellum/local/<slug>` symlink for editors (e.g.
// Zed's "add folder to project") to point at. Desktop only.
export type LinkInfo = {
  id: string;
  vault: string;
  vault_name: string;
  /// Folder prefix without a trailing slash ("" links the whole vault).
  folder: string;
  /// The friendly path to add to an editor.
  path: string;
  enabled: boolean;
};

export const listLinks = () => invoke<LinkInfo[]>("list_links");
export const addLink = (vault: string, folder: string) =>
  invoke<LinkInfo>("add_link", { vault, folder });
export const removeLink = (id: string) => invoke<void>("remove_link", { id });
export const setLinkEnabled = (id: string, enabled: boolean) =>
  invoke<LinkInfo>("set_link_enabled", { id, enabled });
