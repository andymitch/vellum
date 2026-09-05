// The Tauri backend: thin typed wrappers over the Rust commands. All logic
// lives in the backend; this is just the invoke surface. Selected by vault.ts
// whenever we're running inside the Tauri webview (desktop + Android).
//
// See vault.ts for the API's documentation — it is declared once there, as the
// `VaultBackend` interface both backends implement.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  LinkInfo,
  LinkPreview,
  McpStatus,
  NoteTypeEntry,
  SearchHit,
  TagCount,
  TreeNode,
  Unlisten,
  VaultInfo,
} from "./vault";

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
export const writeNote = (vault: string, path: string, content: string, base = "") =>
  invoke<void>("write_note", { vault, path, base, content });
export const createNote = (vault: string, path: string) =>
  invoke<string>("create_note", { vault, path });
export const createFolder = (vault: string, path: string) =>
  invoke<void>("create_folder", { vault, path });
// Bytes cross IPC as a number[]; the conversion is confined to these two
// wrappers so callers only ever see a Uint8Array.
export const exportVault = async (vault: string) =>
  new Uint8Array(await invoke<number[]>("export_vault", { vault }));
export const importVault = (vault: string, data: Uint8Array) =>
  invoke<number>("import_vault", { vault, data: Array.from(data) });
export const shareNote = (vault: string, path: string) =>
  invoke<void>("share_note", { vault, path });
export const renamePath = (vault: string, from: string, to: string, isDir: boolean) =>
  invoke<void>("rename_path", { vault, from, to, isDir });
export const deletePath = (vault: string, path: string, isDir: boolean) =>
  invoke<void>("delete_path", { vault, path, isDir });
export const watchVault = (vault: string) => invoke<void>("watch_vault", { vault });

export const setBackgroundSync = (enabled: boolean) =>
  invoke<void>("set_background_sync", { enabled });

export const onVaultChanged = (cb: (vaultId: string) => void): Promise<Unlisten> =>
  listen<string>("vault-changed", (e) => cb(e.payload));

export const onBackgroundSyncChanged = (cb: (on: boolean) => void): Promise<Unlisten> =>
  listen<boolean>("background-sync", (e) => cb(e.payload));

export const mcpStatus = () => invoke<McpStatus>("mcp_status");
export const setMcpEnabled = (enabled: boolean) =>
  invoke<McpStatus>("set_mcp_enabled", { enabled });

export const searchNotes = (vault: string, query: string, max?: number) =>
  invoke<SearchHit[]>("search_notes", { vault, query, max });
export const listTags = (vault: string) => invoke<TagCount[]>("list_tags", { vault });

export const fetchLinkPreview = (url: string) =>
  invoke<LinkPreview | null>("fetch_link_preview", { url });

export const listNoteTypes = (vault: string) =>
  invoke<NoteTypeEntry[]>("list_note_types", { vault });

export const listLinks = () => invoke<LinkInfo[]>("list_links");
export const addLink = (vault: string, folder: string) =>
  invoke<LinkInfo>("add_link", { vault, folder });
export const removeLink = (id: string) => invoke<void>("remove_link", { id });
export const setLinkEnabled = (id: string, enabled: boolean) =>
  invoke<LinkInfo>("set_link_enabled", { id, enabled });
