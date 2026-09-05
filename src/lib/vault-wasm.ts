// The browser backend: the *same* Rust vault the desktop runs, compiled to
// wasm and driven in a worker (#221/#222).
//
// This replaces the IndexedDB backend it grew out of. That one re-implemented
// vault.rs's rules in TypeScript — tree shape, search ranking, tag counts,
// de-duplication — and kept a second source of truth for note text, which is
// the shape of bug #167. Here the rules are the rules: one iroh-docs replica,
// one yrs merge, one implementation.
//
// What that buys beyond correctness is sync. The IndexedDB backend had none,
// so `joinVault`/`shareVault` rejected and notes moved between devices by zip.
// A real node means a browser tab is a peer.
//
// See vault.ts for the API's documentation — it is declared once there, as the
// `VaultBackend` interface both backends implement.
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

// One vault database per origin, holding every vault this browser knows about —
// the same shape as the desktop's app data dir, not one file per vault.
const VAULT_DB = "vellum.redb";

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };

let worker: Worker | null = null;
let next = 0;
const pending = new Map<number, Pending>();
const changeListeners = new Set<(vaultId: string) => void>();
let opened: Promise<void> | null = null;

function start(): Promise<void> {
  if (opened) return opened;
  worker = new Worker(new URL("./wasm/worker.js", import.meta.url), { type: "module" });
  worker.onmessage = (e: MessageEvent) => {
    const data = e.data;
    // Unsolicited messages are events, not replies: the worker forwarding the
    // node's change channel, which is what the desktop turns into a Tauri event.
    if (data?.event === "vault-changed") {
      for (const cb of changeListeners) cb(data.vault);
      return;
    }
    const settle = pending.get(data.id);
    if (!settle) return;
    pending.delete(data.id);
    data.ok ? settle.resolve(data.value) : settle.reject(new Error(data.error));
  };
  opened = call<void>("open", VAULT_DB);
  return opened;
}

function call<T>(cmd: string, ...args: unknown[]): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const id = ++next;
    pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
    worker!.postMessage({ id, cmd, args });
  });
}

/// Every command boots the node first. It is one promise, so concurrent callers
/// on a cold start queue behind the same open rather than racing to build a
/// second node on the same database — which OPFS would refuse anyway.
async function cmd<T>(name: string, ...args: unknown[]): Promise<T> {
  await start();
  return call<T>(name, ...args);
}

const unsupported = (what: string): Promise<never> =>
  Promise.reject(new Error(`${what} isn't available in the browser version.`));

export const listVaults = () => cmd<VaultInfo[]>("list_vaults");
export const createVault = (name: string) => cmd<VaultInfo>("create_vault", name);
export const joinVault = (ticket: string) => cmd<VaultInfo>("join_vault", ticket);
export const shareVault = (vault: string) => cmd<string>("share_vault", vault);
export const forgetVault = (vault: string) => cmd<void>("forget_vault", vault);
export const renameVault = (vault: string, name: string) =>
  cmd<void>("rename_vault", vault, name);

export const listTree = (vault: string) => cmd<TreeNode[]>("list_tree", vault);
export const readNote = (vault: string, path: string) => cmd<string>("read_note", vault, path);
export const writeNote = (vault: string, path: string, content: string, base = "") =>
  cmd<void>("write_note", vault, path, base, content);
export const createNote = (vault: string, path: string) =>
  cmd<string>("create_note", vault, path);
export const createFolder = (vault: string, path: string) =>
  cmd<void>("create_folder", vault, path);
export const renamePath = (vault: string, from: string, to: string, isDir: boolean) =>
  cmd<void>("rename_path", vault, from, to, isDir);
export const deletePath = (vault: string, path: string, isDir: boolean) =>
  cmd<void>("delete_path", vault, path, isDir);

export const searchNotes = (vault: string, query: string, max = 50) =>
  cmd<SearchHit[]>("search_notes", vault, query, max);
export const listTags = (vault: string) => cmd<TagCount[]>("list_tags", vault);
export const listNoteTypes = (vault: string) => cmd<NoteTypeEntry[]>("list_note_types", vault);

/// The same zip the installed app reads and writes (#79) — produced by the same
/// Rust code, so interop is a property of the build rather than of two
/// implementations agreeing.
export const exportVault = (vault: string) => cmd<Uint8Array>("export_vault", vault);
export const importVault = (vault: string, data: Uint8Array) =>
  cmd<number>("import_vault", vault, data);

export const watchVault = (vault: string) => cmd<void>("watch_vault", vault);
/// A tab cannot stay alive in the background, so there is no keep-alive to
/// flip; arming every vault still makes an open tab a hub for peers that are
/// only intermittently online.
export const setBackgroundSync = (enabled: boolean) =>
  cmd<void>("set_background_sync", enabled);

export async function onVaultChanged(cb: (vaultId: string) => void): Promise<Unlisten> {
  await start();
  changeListeners.add(cb);
  return () => changeListeners.delete(cb);
}

/// Nothing outside the Settings toggle changes background sync here, so this
/// never fires.
export const onBackgroundSyncChanged = (_cb: (on: boolean) => void): Promise<Unlisten> =>
  Promise.resolve(() => {});

const MCP_OFF: McpStatus = { enabled: false, port: null, url: null, token: "", command: null };

/// The MCP server is an HTTP listener in the app process; a tab can't host one.
/// Reported off, as the mobile Rust build does.
export const mcpStatus = (): Promise<McpStatus> => Promise.resolve(MCP_OFF);
export const setMcpEnabled = (_enabled: boolean): Promise<McpStatus> => Promise.resolve(MCP_OFF);

/// Needs a cross-origin fetch of arbitrary sites, which CORS blocks from a
/// page. The desktop does it backend-side; here a plain link is the answer.
export const fetchLinkPreview = (_url: string): Promise<LinkPreview | null> =>
  Promise.resolve(null);

/// Linked folders mirror a vault into a real directory. A browser has none.
export const listLinks = (): Promise<LinkInfo[]> => Promise.resolve([]);
export const addLink = (_vault: string, _folder: string): Promise<LinkInfo> =>
  unsupported("Linked folders");
export const removeLink = (_id: string): Promise<void> => unsupported("Linked folders");
export const setLinkEnabled = (_id: string, _enabled: boolean): Promise<LinkInfo> =>
  unsupported("Linked folders");

/// The Web Share API is the browser's share sheet — on iOS that is the same
/// sheet the native build reaches through AppKit/intents. Where it doesn't
/// exist (most desktop browsers), say so instead of failing silently.
export async function shareNote(vault: string, path: string): Promise<void> {
  const text = await readNote(vault, path);
  const title = (path.split("/").pop() ?? path).replace(/\.md$/i, "");
  if (!navigator.share) {
    throw new Error("This browser has no share sheet — use Export or Email instead.");
  }
  try {
    await navigator.share({ title, text });
  } catch (e) {
    // A dismissed sheet rejects with AbortError; that isn't a failure worth
    // reporting to the user.
    if (e instanceof DOMException && e.name === "AbortError") return;
    throw e;
  }
}
