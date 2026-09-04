// The browser backend: vaults stored in IndexedDB, for the hosted web build and
// the iOS PWA (#221/#222). Selected by vault.ts when we're not in Tauri.
//
// The Tauri builds keep each vault in an iroh-docs replica, which is what makes
// them sync peer-to-peer; a browser has none of that. So this stores the same
// *shape* of data — one flat `path -> text` map per vault, using exactly the
// keys the Rust side uses, `.keep` folder markers included — and derives the
// tree, search, tags and note types from it with the same rules. Keeping the
// key layout identical is what lets one set of callers, and one zip format,
// serve both shells.
//
// The rules themselves are shared where they already existed in TypeScript
// (`tags.ts`, `note-type.ts`, which mirror vault.rs and are tested against it);
// the rest — tree shape, name de-duplication, search ranking, the scan cap — is
// ported here as pure functions over records, which is also what makes them
// testable (see vault-web.test.ts).
//
// What a browser cannot do, it says so plainly rather than pretending: sharing
// and joining a vault reject (the UI hides both), and the MCP server, linked
// folders and background sync report themselves off, the same way the mobile
// Rust build does.

import { parseNote } from "./note-type";
import { asTagQuery, distinctTags, hasTag } from "./tags";
import { readZip, writeZip, type ZipEntry } from "./zip";
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

/// Marker entry that keeps an otherwise-empty folder alive, as in vault.rs.
const KEEP = ".keep";
/// Where the MCP server's soft-deletes land in the Tauri backends. Nothing in
/// the browser writes it, but an imported archive can carry it, and it must
/// stay out of listings the same way.
const TRASH = ".trash";
/// Mirrors SEARCH_SCAN_LIMIT: a cap on notes scanned per query, so a huge
/// imported vault degrades instead of hanging the UI thread.
const SCAN_LIMIT = 2000;

// ============================ storage ============================

const DB_NAME = "vellum";
const DB_VERSION = 1;
const VAULTS = "vaults";
const ENTRIES = "entries";

type VaultRecord = { id: string; name: string; created: number };
/// One note (or folder marker). `key` is the vault-relative path.
type EntryRecord = { vault: string; key: string; text: string; modified: number };

let handle: Promise<IDBDatabase> | null = null;

function db(): Promise<IDBDatabase> {
  if (!handle) handle = openDb();
  return handle;
}

function openDb(): Promise<IDBDatabase> {
  // Ask for persistent storage: Safari evicts script-writable storage for sites
  // not used in seven days, which would quietly delete someone's notes. An
  // installed PWA (added to the home screen) is exempt, and Chrome grants this
  // on engagement — so this is best-effort, and the export in Settings is the
  // real guarantee.
  void navigator.storage?.persist?.().catch(() => {});
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const d = req.result;
      if (!d.objectStoreNames.contains(VAULTS)) d.createObjectStore(VAULTS, { keyPath: "id" });
      // Compound key, so every read of a vault is one contiguous range scan.
      if (!d.objectStoreNames.contains(ENTRIES))
        d.createObjectStore(ENTRIES, { keyPath: ["vault", "key"] });
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("could not open the notes database"));
  });
}

const request = <T>(req: IDBRequest<T>): Promise<T> =>
  new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });

const finished = (t: IDBTransaction): Promise<void> =>
  new Promise((resolve, reject) => {
    t.oncomplete = () => resolve();
    t.onerror = () => reject(t.error);
    t.onabort = () => reject(t.error ?? new Error("the write was aborted"));
  });

/// Every entry in a vault, newest first — the order `list_entries` returns, which
/// search and tag counts are defined in terms of. Ties break on key so the order
/// is stable across reloads.
async function entriesOf(vault: string): Promise<EntryRecord[]> {
  const d = await db();
  const range = IDBKeyRange.bound([vault, ""], [vault, "\uffff"]);
  const all = await request<EntryRecord[]>(
    d.transaction(ENTRIES, "readonly").objectStore(ENTRIES).getAll(range),
  );
  return all.sort((a, b) => b.modified - a.modified || (a.key < b.key ? -1 : 1));
}

async function entryOf(vault: string, key: string): Promise<EntryRecord | undefined> {
  const d = await db();
  return request<EntryRecord | undefined>(
    d.transaction(ENTRIES, "readonly").objectStore(ENTRIES).get([vault, key]),
  );
}

/// Apply a set of writes and deletions in one transaction, then announce the
/// change to any other open tab.
async function apply(
  vault: string,
  changes: { put?: { key: string; text: string; modified?: number }[]; drop?: string[] },
): Promise<void> {
  const d = await db();
  const t = d.transaction(ENTRIES, "readwrite");
  const store = t.objectStore(ENTRIES);
  for (const key of changes.drop ?? []) store.delete([vault, key]);
  for (const e of changes.put ?? [])
    store.put({ vault, key: e.key, text: e.text, modified: e.modified ?? Date.now() });
  await finished(t);
  announce(vault);
}

// Other tabs of the same web app are the only "peer" a browser vault has, so
// they get the same vault-changed notification the Rust backend emits for a
// real peer. A BroadcastChannel never delivers to the tab that posted, which is
// exactly right: the tab that made the change refreshes itself.
const CHANNEL = "vellum-vault-changed";
let channel: BroadcastChannel | null = null;

function announce(vault: string) {
  if (typeof BroadcastChannel === "undefined") return;
  channel ??= new BroadcastChannel(CHANNEL);
  channel.postMessage(vault);
}

// ============================ pure rules ============================

/// Keys that aren't user-visible notes: folder markers and anything trashed.
/// Mirrors `is_hidden_path`.
export function isHiddenPath(path: string): boolean {
  return (
    path.endsWith(KEEP) || path.endsWith("/") || path === TRASH || path.startsWith(`${TRASH}/`)
  );
}

/// Build the folder tree from a vault's keys: folders (alphabetical) before
/// files (alphabetical) at every level, folders implied by key prefixes, and a
/// `<folder>/.keep` marker standing in for an empty folder. Mirrors
/// `build_tree`.
export function buildTree(keys: string[]): TreeNode[] {
  type Dir = { dirs: Map<string, Dir>; files: Map<string, string> };
  const root: Dir = { dirs: new Map(), files: new Map() };
  const dirAt = (segments: string[]): Dir =>
    segments.reduce((dir, name) => {
      let child = dir.dirs.get(name);
      if (!child) dir.dirs.set(name, (child = { dirs: new Map(), files: new Map() }));
      return child;
    }, root);

  for (const key of keys) {
    const segments = key.split("/");
    const last = segments[segments.length - 1];
    if (last === KEEP) {
      // The marker itself is not a node; it only proves its folder exists.
      if (segments.length > 1) dirAt(segments.slice(0, -1));
    } else {
      dirAt(segments.slice(0, -1)).files.set(last, key);
    }
  }

  const sorted = <T>(m: Map<string, T>) =>
    [...m.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  const nodes = (dir: Dir, prefix: string): TreeNode[] => [
    ...sorted(dir.dirs).map(([name, child]) => {
      const path = prefix ? `${prefix}/${name}` : name;
      return { name, path, is_dir: true, children: nodes(child, path) };
    }),
    ...sorted(dir.files).map(([name, path]) => ({
      name,
      path,
      is_dir: false,
      children: [],
    })),
  ];
  return nodes(root, "");
}

/// Find a free key by inserting/incrementing a numeric suffix before the
/// extension: `Untitled.md` → `Untitled 1.md` → `Untitled 2.md` … Mirrors
/// `free_key`.
export function freeKey(taken: ReadonlySet<string>, path: string): string {
  if (!taken.has(path)) return path;
  const dot = path.lastIndexOf(".");
  const [stem, ext] = dot === -1 ? [path, ""] : [path.slice(0, dot), path.slice(dot)];
  for (let n = 1; ; n++) {
    const candidate = `${stem} ${n}${ext}`;
    if (!taken.has(candidate)) return candidate;
  }
}

/// A note as the pure scans see it.
export type ScannedNote = { path: string; text: string };

/// Case-insensitive substring search across a vault's notes, with tag queries
/// and tag promotion — mirrors `search` in vault.rs (#202):
///
///   - `#work` is a TAG query: it matches tag identity, never raw text, so it
///     finds a line-final `#work` and does not match `#workout`, `x#work` or a
///     `…/#work` URL fragment.
///   - `work` is a TEXT query: ordinary substring search, but notes carrying it
///     as a tag rank above notes that merely contain the word.
export function searchNotesIn(notes: ScannedNote[], query: string, max: number): SearchHit[] {
  if (!query.trim()) return [];
  const tag = asTagQuery(query);
  const needle = query.toLowerCase();
  // For a text query, the tag whose carriers get promoted — only when the query
  // could be a tag at all ("work" can, "work stuff" cannot).
  const boost = tag ? null : asTagQuery(`#${query.trim()}`);
  const tagged: SearchHit[] = [];
  const rest: SearchHit[] = [];
  let scanned = 0;
  for (const note of notes) {
    // Stop early only once the top bucket is full: a note scanned later may
    // still carry the tag and outrank everything already in `rest`.
    const full = tag || boost ? tagged.length >= max : rest.length >= max;
    if (full || scanned >= SCAN_LIMIT) break;
    if (isHiddenPath(note.path)) continue;
    scanned++;
    const wanted = tag ?? boost;
    const carries = wanted !== null && hasTag(note.text, wanted);
    // A tag query is satisfied by tag identity alone, so a note that merely
    // contains the characters is not a hit at all.
    if (tag && !carries) continue;
    const lines: string[] = [];
    const all = note.text.split(/\r?\n/);
    for (let i = 0; i < all.length && lines.length < 3; i++) {
      const matched = tag ? hasTag(all[i], tag) : all[i].toLowerCase().includes(needle);
      if (matched) lines.push(`${i + 1}: ${all[i].trim()}`);
    }
    if (!lines.length) continue;
    const hit = { path: note.path, lines };
    if (carries) tagged.push(hit);
    else if (rest.length < max) rest.push(hit);
  }
  return [...tagged, ...rest].slice(0, max);
}

/// Every inline tag in a vault with the number of notes carrying it, most-used
/// first (ties alphabetical, so the order is stable). Mirrors `tags_in_vault`.
export function tagCountsIn(notes: ScannedNote[]): TagCount[] {
  const counts = new Map<string, number>();
  let scanned = 0;
  for (const note of notes) {
    if (scanned >= SCAN_LIMIT) break;
    if (isHiddenPath(note.path)) continue;
    scanned++;
    for (const tag of distinctTags(note.text)) counts.set(tag, (counts.get(tag) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count || (a.tag < b.tag ? -1 : a.tag > b.tag ? 1 : 0));
}

/// Every note that declares a type, for the file tree's icons (#180/#181).
/// Plain Markdown notes are omitted — they're the default and the majority.
/// Mirrors `note_types`.
export function noteTypesIn(notes: ScannedNote[]): NoteTypeEntry[] {
  const out: NoteTypeEntry[] = [];
  let scanned = 0;
  for (const note of notes) {
    if (scanned >= SCAN_LIMIT) break;
    if (isHiddenPath(note.path)) continue;
    scanned++;
    const { type } = parseNote(note.text);
    if (type !== "markdown") out.push({ path: note.path, note_type: type });
  }
  return out;
}

// ============================ commands ============================

const unsupported = (what: string) =>
  Promise.reject(
    new Error(`${what} needs the desktop or Android app — the web version can't do it.`),
  );

/// 6 hex chars of the id, shown after the name to disambiguate vaults that
/// share a display name (#120) — the same suffix the Rust backend derives from
/// the namespace id.
const infoOf = (v: VaultRecord): VaultInfo => ({
  id: v.id,
  name: v.name,
  pending: false, // nothing to wait for: there are no peers
  hash: v.id.slice(0, 6),
});

const newId = () =>
  [...crypto.getRandomValues(new Uint8Array(16))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

export async function listVaults(): Promise<VaultInfo[]> {
  const d = await db();
  const all = await request<VaultRecord[]>(
    d.transaction(VAULTS, "readonly").objectStore(VAULTS).getAll(),
  );
  return all.sort((a, b) => a.created - b.created).map(infoOf);
}

export async function createVault(name: string): Promise<VaultInfo> {
  const record: VaultRecord = { id: newId(), name: name.trim(), created: Date.now() };
  const d = await db();
  const t = d.transaction(VAULTS, "readwrite");
  t.objectStore(VAULTS).put(record);
  await finished(t);
  return infoOf(record);
}

export const joinVault = (_ticket: string): Promise<VaultInfo> => unsupported("Joining a vault");
export const shareVault = (_vault: string): Promise<string> => unsupported("Sharing a vault");

export async function forgetVault(vault: string): Promise<void> {
  const keys = (await entriesOf(vault)).map((e) => e.key);
  await apply(vault, { drop: keys });
  const d = await db();
  const t = d.transaction(VAULTS, "readwrite");
  t.objectStore(VAULTS).delete(vault);
  await finished(t);
}

export async function renameVault(vault: string, name: string): Promise<void> {
  const trimmed = name.trim();
  // The Rust backend treats an empty name as "clear my local override" and
  // falls back to the synced meta name; there is no synced meta here, so an
  // empty name would leave the vault unnamed. Keep the current one.
  if (!trimmed) return;
  const d = await db();
  const existing = await request<VaultRecord | undefined>(
    d.transaction(VAULTS, "readonly").objectStore(VAULTS).get(vault),
  );
  if (!existing) return;
  const t = d.transaction(VAULTS, "readwrite");
  t.objectStore(VAULTS).put({ ...existing, name: trimmed });
  await finished(t);
  announce(vault);
}

export async function listTree(vault: string): Promise<TreeNode[]> {
  return buildTree((await entriesOf(vault)).map((e) => e.key));
}

export async function readNote(vault: string, path: string): Promise<string> {
  return (await entryOf(vault, path))?.text ?? "";
}

// `base` is unused: it exists so the Tauri backend can 3-way merge against a
// concurrent peer edit (#99), and in a browser this tab is the only writer.
export async function writeNote(
  vault: string,
  path: string,
  content: string,
  _base = "",
): Promise<void> {
  await apply(vault, { put: [{ key: path, text: content }] });
}

export async function createNote(vault: string, path: string): Promise<string> {
  const taken = new Set((await entriesOf(vault)).map((e) => e.key));
  const free = freeKey(taken, path);
  await apply(vault, { put: [{ key: free, text: "" }] });
  return free;
}

export async function createFolder(vault: string, path: string): Promise<void> {
  await apply(vault, { put: [{ key: `${path.replace(/\/+$/, "")}/${KEEP}`, text: "" }] });
}

export async function renamePath(
  vault: string,
  from: string,
  to: string,
  isDir: boolean,
): Promise<void> {
  const entries = await entriesOf(vault);
  if (!isDir) {
    const entry = entries.find((e) => e.key === from);
    if (!entry) return;
    await apply(vault, {
      put: [{ key: to, text: entry.text, modified: entry.modified }],
      drop: [from],
    });
    return;
  }
  const fromPrefix = `${from.replace(/\/+$/, "")}/`;
  const toPrefix = `${to.replace(/\/+$/, "")}/`;
  const moved = entries.filter((e) => e.key.startsWith(fromPrefix));
  await apply(vault, {
    put: moved.map((e) => ({
      key: `${toPrefix}${e.key.slice(fromPrefix.length)}`,
      text: e.text,
      modified: e.modified,
    })),
    drop: moved.map((e) => e.key),
  });
}

export async function deletePath(vault: string, path: string, isDir: boolean): Promise<void> {
  if (!isDir) {
    await apply(vault, { drop: [path] });
    return;
  }
  const prefix = `${path.replace(/\/+$/, "")}/`;
  const keys = (await entriesOf(vault))
    .map((e) => e.key)
    .filter((k) => k === path || k.startsWith(prefix));
  await apply(vault, { drop: keys });
}

/// No peers and no file watcher, so there is nothing to arm — other tabs are
/// picked up through the BroadcastChannel in `onVaultChanged`.
export const watchVault = (_vault: string): Promise<void> => Promise.resolve();

/// Nothing to keep alive: a browser tab is not a sync hub.
export const setBackgroundSync = (_enabled: boolean): Promise<void> => Promise.resolve();

export function onVaultChanged(cb: (vaultId: string) => void): Promise<Unlisten> {
  if (typeof BroadcastChannel === "undefined") return Promise.resolve(() => {});
  const listener = new BroadcastChannel(CHANNEL);
  listener.onmessage = (e: MessageEvent<string>) => cb(e.data);
  return Promise.resolve(() => listener.close());
}

/// Background sync can't be changed from anywhere else here, so this never
/// fires.
export const onBackgroundSyncChanged = (_cb: (on: boolean) => void): Promise<Unlisten> =>
  Promise.resolve(() => {});

const MCP_OFF: McpStatus = {
  enabled: false,
  port: null,
  url: null,
  token: "",
  command: null,
};

/// The MCP server is an HTTP listener in the app process; a tab can't host one.
/// Reported off, as the mobile Rust build does.
export const mcpStatus = (): Promise<McpStatus> => Promise.resolve(MCP_OFF);
export const setMcpEnabled = (_enabled: boolean): Promise<McpStatus> => Promise.resolve(MCP_OFF);

export async function searchNotes(
  vault: string,
  query: string,
  max?: number,
): Promise<SearchHit[]> {
  const notes = (await entriesOf(vault)).map(({ key, text }) => ({ path: key, text }));
  return searchNotesIn(notes, query, Math.min(100, Math.max(1, max ?? 20)));
}

export async function listTags(vault: string): Promise<TagCount[]> {
  const notes = (await entriesOf(vault)).map(({ key, text }) => ({ path: key, text }));
  return tagCountsIn(notes);
}

export async function listNoteTypes(vault: string): Promise<NoteTypeEntry[]> {
  const notes = (await entriesOf(vault)).map(({ key, text }) => ({ path: key, text }));
  return noteTypesIn(notes);
}

/// Link previews are a backend feature precisely because CORS blocks the fetch
/// from a page (#62), and that applies in full here. Resolving null is the
/// documented "no preview" answer, so a link just renders as a link.
export const fetchLinkPreview = (_url: string): Promise<LinkPreview | null> =>
  Promise.resolve(null);

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

// ---- vault archives (#79) ----

export async function exportVault(vault: string): Promise<Uint8Array> {
  const encoder = new TextEncoder();
  const items: ZipEntry[] = [];
  // Alphabetical, so an archive of the same vault is byte-comparable across
  // exports rather than ordered by whatever was edited last.
  for (const entry of (await entriesOf(vault)).sort((a, b) => (a.key < b.key ? -1 : 1))) {
    if (entry.key === KEEP) continue; // a marker at the root names no folder
    if (entry.key.endsWith(`/${KEEP}`)) {
      items.push({ name: entry.key.slice(0, -KEEP.length), data: null });
      continue;
    }
    items.push({ name: entry.key, data: encoder.encode(entry.text) });
  }
  return writeZip(items);
}

export async function importVault(vault: string, data: Uint8Array): Promise<number> {
  // `fatal` so a binary file that slipped past the extension check is skipped
  // rather than imported as a note full of replacement characters — what
  // `import_vault` does with a non-UTF-8 entry.
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const taken = new Set((await entriesOf(vault)).map((e) => e.key));
  const put: { key: string; text: string }[] = [];
  let count = 0;

  for (const entry of await readZip(data)) {
    const name = entry.name.replace(/\\/g, "/").replace(/^\/+/, "");
    // Entry names become vault keys, so reject a crafted archive's path
    // traversal, empty segments and meta-namespace injection — the same
    // validation `import_vault` does.
    if (
      name
        .replace(/\/+$/, "")
        .split("/")
        .some((seg) => !seg || seg === ".." || seg.charCodeAt(0) === 0)
    ) {
      continue;
    }
    if (entry.data === null) {
      put.push({ key: `${name.replace(/\/+$/, "")}/${KEEP}`, text: "" });
      continue;
    }
    const lower = name.toLowerCase();
    if (!/\.(md|markdown|txt)$/.test(lower)) continue; // only text/markdown
    // .txt imports normalize to .md so they open as notes.
    const path = lower.endsWith(".txt") ? `${name.slice(0, -4)}.md` : name;
    let text: string;
    try {
      text = decoder.decode(entry.data);
    } catch {
      continue;
    }
    const free = freeKey(taken, path);
    taken.add(free);
    put.push({ key: free, text });
    count++;
  }

  await apply(vault, { put });
  return count;
}
