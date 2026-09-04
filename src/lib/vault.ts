// The vault API the rest of the app talks to, and the choice of backend behind
// it.
//
// Two modules implement it: `vault-tauri.ts` invokes the Rust commands (desktop
// + Android), and `vault-web.ts` keeps notes in IndexedDB for the hosted web
// build and the iOS PWA (#221/#222). The types and every method's contract live
// here, because both backends and every caller are defined in terms of them.
//
// The destructuring export at the bottom is the whole switch: one list of
// names, checked against `VaultBackend`, so a command added to one backend and
// forgotten in the other doesn't compile.
import { isTauri } from "./platform";
import * as tauriBackend from "./vault-tauri";
import * as webBackend from "./vault-web";

export type VaultInfo = { id: string; name: string; pending: boolean; hash: string };
export type TreeNode = {
  name: string;
  path: string;
  is_dir: boolean;
  children: TreeNode[];
};

/// Cancels an event subscription.
export type Unlisten = () => void;

// ---- local MCP server (#164) ----
// Lets agents (Claude Code, Claude Desktop, …) CRUD notes over a loopback,
// token-authenticated MCP endpoint hosted by this app. Desktop only — the
// mobile and web builds answer with a disabled status.
export type McpStatus = {
  enabled: boolean;
  port: number | null;
  url: string | null;
  token: string;
  /// Ready-to-paste `claude mcp add …` line; null while stopped.
  command: string | null;
};

// ---- search + tags (#15) ----
// Both scan every note in the vault (each read is a CRDT merge), so callers
// should debounce rather than fire per keystroke.
export type SearchHit = {
  path: string;
  /// Matching lines as "{line}: {text}", at most 3 per note.
  lines: string[];
};
export type TagCount = { tag: string; count: number };

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

// Note types for the file tree's icons (#180/#181). Only typed notes come back —
// plain Markdown is the default and would be noise. This scans every note, so
// callers should debounce rather than refetch on each vault-changed event.
export type NoteTypeEntry = { path: string; note_type: string };

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

/// Everything the app can ask of a vault. Implemented twice — see the header.
///
/// A capability a backend genuinely lacks is expressed one of two ways, and the
/// choice matters at the call site:
///
///   - it *reports* itself off (`mcpStatus`, `listLinks`), for the things whose
///     UI reads a status and would just show a disabled control; or
///   - it *rejects* (`shareVault`, `joinVault`), for the things that would
///     otherwise silently do nothing. The UI hides those affordances up front
///     via the `platform.ts` flags, so the rejection is a backstop.
export interface VaultBackend {
  listVaults(): Promise<VaultInfo[]>;
  createVault(name: string): Promise<VaultInfo>;
  /// Join a vault from a share ticket. Web: rejects — there is no p2p node.
  joinVault(ticket: string): Promise<VaultInfo>;
  /// Write-capability ticket for a vault, rendered as a QR code by the caller.
  /// Web: rejects.
  shareVault(vault: string): Promise<string>;
  forgetVault(vault: string): Promise<void>;
  renameVault(vault: string, name: string): Promise<void>;

  listTree(vault: string): Promise<TreeNode[]>;
  readNote(vault: string, path: string): Promise<string>;
  /// Write note content. Filename and content are independent — renaming is an
  /// explicit file action (`renamePath`), never derived from the content.
  /// `base` is the text the editor loaded; the Tauri backend 3-way merges
  /// base→content against any concurrent peer edit so neither side is clobbered
  /// (#99). New notes pass "".
  writeNote(vault: string, path: string, content: string, base?: string): Promise<void>;
  /// Returns the actual (possibly de-duplicated) path of the created note.
  createNote(vault: string, path: string): Promise<string>;
  createFolder(vault: string, path: string): Promise<void>;
  /// Markdown export/import (#79): a zip of .md files mirroring the folder
  /// tree. Both backends read and write the same archive, which is how notes
  /// move between the installed app and the web app.
  exportVault(vault: string): Promise<Uint8Array>;
  /// Returns the number of notes added.
  importVault(vault: string, data: Uint8Array): Promise<number>;
  /// Hand a note to the OS share sheet — email, Messages/SMS, AirDrop (#105).
  /// Native on desktop/Android (AppKit / an Android intent); the Web Share API
  /// in the browser.
  shareNote(vault: string, path: string): Promise<void>;
  renamePath(vault: string, from: string, to: string, isDir: boolean): Promise<void>;
  deletePath(vault: string, path: string, isDir: boolean): Promise<void>;
  /// Start emitting `onVaultChanged` for this vault's peer edits. Web: no-op,
  /// since the only writer is this browser.
  watchVault(vault: string): Promise<void>;

  /// Toggle background "live sync": arm every vault as an always-on hub and
  /// flip the platform keep-alive (desktop tray + launch-at-login / Android
  /// foreground service) so syncing continues with no window open / while
  /// backgrounded. Web: no-op.
  setBackgroundSync(enabled: boolean): Promise<void>;

  /// Fires when a vault's contents changed underneath us — a peer edit, a
  /// linked-folder write, or (web) another tab.
  onVaultChanged(cb: (vaultId: string) => void): Promise<Unlisten>;
  /// Fires when background sync is changed outside the Settings toggle (e.g.
  /// "Turn off background sync" from the desktop tray).
  onBackgroundSyncChanged(cb: (on: boolean) => void): Promise<Unlisten>;

  mcpStatus(): Promise<McpStatus>;
  setMcpEnabled(enabled: boolean): Promise<McpStatus>;

  searchNotes(vault: string, query: string, max?: number): Promise<SearchHit[]>;
  listTags(vault: string): Promise<TagCount[]>;

  fetchLinkPreview(url: string): Promise<LinkPreview | null>;

  listNoteTypes(vault: string): Promise<NoteTypeEntry[]>;

  listLinks(): Promise<LinkInfo[]>;
  addLink(vault: string, folder: string): Promise<LinkInfo>;
  removeLink(id: string): Promise<void>;
  setLinkEnabled(id: string, enabled: boolean): Promise<LinkInfo>;
}

const backend: VaultBackend = isTauri ? tauriBackend : webBackend;

export const {
  listVaults,
  createVault,
  joinVault,
  shareVault,
  forgetVault,
  renameVault,
  listTree,
  readNote,
  writeNote,
  createNote,
  createFolder,
  exportVault,
  importVault,
  shareNote,
  renamePath,
  deletePath,
  watchVault,
  setBackgroundSync,
  onVaultChanged,
  onBackgroundSyncChanged,
  mcpStatus,
  setMcpEnabled,
  searchNotes,
  listTags,
  fetchLinkPreview,
  listNoteTypes,
  listLinks,
  addLink,
  removeLink,
  setLinkEnabled,
} = backend;
