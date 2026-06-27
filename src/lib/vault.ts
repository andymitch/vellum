// Thin typed wrappers over the Rust commands. All logic lives in the backend;
// this is just the invoke surface.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type VaultInfo = { id: string; name: string; pending: boolean };
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

export const listTree = (vault: string) => invoke<TreeNode[]>("list_tree", { vault });
export const readNote = (vault: string, path: string) =>
  invoke<string>("read_note", { vault, path });
// Writes note content. Filename and content are independent — renaming is an
// explicit file action (rename_path), never derived from the content.
export const writeNote = (vault: string, path: string, content: string) =>
  invoke<void>("write_note", { vault, path, content });
// Returns the actual (possibly de-duplicated) path of the created note.
export const createNote = (vault: string, path: string) =>
  invoke<string>("create_note", { vault, path });
export const createFolder = (vault: string, path: string) =>
  invoke<void>("create_folder", { vault, path });
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
