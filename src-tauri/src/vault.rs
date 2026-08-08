//! P2P vault backend built on iroh-docs.
//!
//! Each vault is an iroh-docs document (a unique namespace). Notes live as
//! entries keyed by their path (`work/todo.md`); folders are implicit from key
//! prefixes, with an empty folder kept alive by a `<folder>/.keep` marker.
//!
//! iroh-docs treats an empty value as a deletion tombstone (excluded from
//! queries), so every stored value is prefixed with a single marker byte to
//! guarantee it is non-empty — this lets a user clear a note to "" without the
//! note vanishing. The marker is stripped on read.
//!
//! The iroh node is built lazily on the first command rather than in Tauri
//! `setup`: on Android, iroh's network monitor reads the Android JNI context
//! (via `ndk-context`), which tao only initializes once the event loop is
//! running — after `setup`. Deferring the build avoids that startup race.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint, EndpointAddr, EndpointId, SecretKey};
use iroh_blobs::{
    store::{fs::options::Options, fs::FsStore, GcConfig},
    BlobsProtocol,
};
use iroh_docs::{
    api::protocol::{AddrInfoOptions, ShareMode},
    engine::{LiveEvent, ProtectCallbackHandler},
    protocol::Docs,
    store::Query,
    AuthorId, DocTicket, NamespaceId,
};
use iroh_gossip::net::Gossip;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::OnceCell;
use yrs::updates::decoder::Decode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, TextRef, Transact, TransactionMut, Update};

const NAME_KEY: &[u8] = b"\x00meta/name";
const MARKER: u8 = 0x01;
// Note bodies are stored as a yrs (CRDT) document state, tagged 0x02, so
// concurrent edits on different devices merge instead of clobbering — plain
// last-writer-wins would silently drop one side (issue #99). Plain-text values
// (the vault meta name, `.keep` folder markers, and pre-CRDT notes) stay tagged
// 0x01 (MARKER) and are seeded into a CRDT on first edit. Each note doc holds a
// single text type rooted under TEXT_ROOT.
const TAG_YRS: u8 = 0x02;
const TEXT_ROOT: &str = "t";
pub(crate) const KEEP: &str = ".keep";
// How often to sweep orphaned content blobs (old note versions no longer
// referenced by any entry). Blobs referenced by current entries are protected.
const GC_INTERVAL: Duration = Duration::from_secs(600);
// Drop a cached peer we haven't seen sync in this long, so dead peers (old
// devices, closed emulators) don't accumulate and get re-dialed forever.
const PEER_TTL_SECS: u64 = 60 * 60 * 24 * 30; // 30 days

/// Per-vault known sync peers: EndpointId -> last-seen unix seconds. Persisted to
/// `peers.json`. The timestamp lets us prune peers we haven't synced with in a
/// long time (PEER_TTL_SECS) so dead devices don't pile up.
type PeerMap = BTreeMap<NamespaceId, BTreeMap<EndpointId, u64>>;

/// Per-user LOCAL vault display-name overrides: NamespaceId -> name. Persisted to
/// `vault-names.json`. Renaming a vault (#120) writes here only; it never touches
/// the synced `\x00meta/name`, so renames stay local to this device/user.
type NameMap = BTreeMap<NamespaceId, String>;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A mutation observed on an armed vault, broadcast to in-process listeners
/// (the MCP server's resource subscriptions — see `mcp.rs`). `path` is the note
/// key when the event carries one; `None` for events that only say "something in
/// this vault changed" (e.g. `ContentReady`, which carries a blob hash we can't
/// map back to a key), so listeners refresh the vault rather than one note.
#[derive(Clone, Debug)]
pub struct VaultChange {
    pub vault: NamespaceId,
    pub path: Option<String>,
}

// Bounded: a slow subscriber lags rather than growing the queue without limit.
// Lagged receivers get a `RecvError::Lagged` they treat as "refresh everything".
const CHANGE_CHANNEL_CAP: usize = 64;

/// The live iroh node (handles into the running protocols).
pub struct Node {
    blobs: FsStore,
    docs: Docs,
    author: AuthorId,
    _router: Router,
    watched: std::sync::Arc<Mutex<HashSet<NamespaceId>>>,
    dir: PathBuf,
    our_id: EndpointId,
    // Known sync peers per vault. We dial these by EndpointId (discovery resolves
    // the current relay + addresses), so connections survive address/network
    // changes and restarts — rather than relying on stale ticket addresses.
    peers: std::sync::Arc<Mutex<PeerMap>>,
    // Local per-user vault name overrides (#120). Never synced.
    names: std::sync::Arc<Mutex<NameMap>>,
    // In-process fanout of the same mutations that drive the `vault-changed`
    // Tauri event. The frontend uses the event; the MCP server needs the entry
    // key too (to name the resource that changed), which the event doesn't carry.
    changes: tokio::sync::broadcast::Sender<VaultChange>,
}

impl Node {
    /// Subscribe to vault mutations. Only *armed* vaults emit (see `arm_vault`).
    pub fn subscribe_changes(&self) -> tokio::sync::broadcast::Receiver<VaultChange> {
        self.changes.subscribe()
    }

    /// This device's author id — the identity every write is signed with.
    pub(crate) fn author(&self) -> AuthorId {
        self.author
    }

    /// The docs protocol handle. Only the MCP tests need it — everything else
    /// goes through `open`/`all_vaults`.
    #[cfg(test)]
    pub(crate) fn docs(&self) -> &Docs {
        &self.docs
    }

    /// Flush the blob store. Only the MCP seed helper needs this: a short-lived
    /// process that writes and exits leaves entries in redb whose content blobs
    /// were never written, so the notes read back as "not synced yet".
    #[cfg(test)]
    pub(crate) async fn flush_blobs(&self) {
        // FsStore derefs to the blobs Store, which owns the flush.
        let _ = self.blobs.shutdown().await;
    }
}

/// Managed Tauri state. Builds the node lazily on first use (see module docs).
pub struct VaultManager {
    node: OnceCell<Node>,
    dir: PathBuf,
}

impl VaultManager {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            node: OnceCell::new(),
            dir,
        }
    }

    /// Returns the node, building it on first call. A failed build is not
    /// cached, so the next command retries.
    pub(crate) async fn node(&self) -> Result<&Node, String> {
        self.node
            .get_or_try_init(|| init(self.dir.clone()))
            .await
            .map_err(|e| {
                let s = e.to_string();
                eprintln!("[vault] node init failed: {s}");
                s
            })
    }
}

#[derive(Serialize)]
pub struct VaultInfo {
    pub(crate) id: String,
    pub(crate) name: String,
    // True when the vault has no synced meta yet — i.e. it was joined from a
    // ticket but no peer has come online to sync its contents. The UI shows a
    // "waiting for a peer" state instead of a misleading generated vault (#4).
    pub(crate) pending: bool,
    // First 6 hex chars of the id; shown after the name to disambiguate vaults
    // that share a local display name (#120).
    pub(crate) hash: String,
}

#[derive(Serialize)]
pub struct TreeNode {
    name: String,
    path: String,
    is_dir: bool,
    children: Vec<TreeNode>,
}

fn encode(content: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(content.len() + 1);
    v.push(MARKER);
    v.extend_from_slice(content.as_bytes());
    v
}

fn decode(bytes: &[u8]) -> String {
    let body = match bytes.first() {
        Some(&MARKER) => &bytes[1..],
        _ => bytes,
    };
    String::from_utf8_lossy(body).into_owned()
}

// ---- note CRDT (yrs) ----

/// Deterministic yrs client id from `bytes` via FNV-1a, masked to 53 bits (yrs
/// requires Yjs-compatible 53-bit ids). Identical input → identical id on every
/// device, so e.g. a legacy note seeded independently on two peers produces
/// byte-identical ops that dedup on merge instead of duplicating the text.
fn client_id(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h & ((1u64 << 53) - 1)
}

/// A full-state v1 yrs update for a doc whose text is `s`, built with a
/// content-derived client id so the same text seeds identically on any device.
/// This is the unit applied when merging, and (tagged) the value we store.
fn seed_update(s: &str) -> Vec<u8> {
    let doc = Doc::with_client_id(client_id(format!("seed:{s}").as_bytes()));
    let text = doc.get_or_insert_text(TEXT_ROOT);
    let mut txn = doc.transact_mut();
    if !s.is_empty() {
        text.insert(&mut txn, 0, s);
    }
    txn.encode_state_as_update_v1(&StateVector::default())
}

/// Turn one stored note value into a yrs update ready to apply: a 0x02 value is
/// already an update; a legacy 0x01 (or untagged) plain-text value is seeded
/// from its text.
///
/// Disambiguate by UTF-8 validity, not by the leading byte: a raw yrs update for
/// a single-client doc begins with 0x01 — colliding with MARKER — so a
/// `bytes.first()` test alone would treat an untagged yrs update (stored by
/// builds predating the 0x02 tag) as plain text and `from_utf8_lossy` would bake
/// U+FFFD garbage into the doc. Only seed-from-text when the marker-stripped
/// bytes are valid UTF-8; otherwise hand the raw bytes to the yrs decoder.
fn value_to_update(bytes: &[u8]) -> Vec<u8> {
    if bytes.first() == Some(&TAG_YRS) {
        return bytes[1..].to_vec();
    }
    let body = match bytes.first() {
        Some(&MARKER) => &bytes[1..],
        _ => bytes,
    };
    match std::str::from_utf8(body) {
        Ok(s) => seed_update(s),
        Err(_) => bytes.to_vec(),
    }
}

/// Materialize a note doc's text.
fn doc_text(doc: &Doc) -> String {
    let text = doc.get_or_insert_text(TEXT_ROOT);
    let txn = doc.transact();
    text.get_string(&txn)
}

/// Encode a note doc's full state for storage, tagged 0x02.
pub(crate) fn encode_doc(doc: &Doc) -> Vec<u8> {
    let txn = doc.transact();
    let state = txn.encode_state_as_update_v1(&StateVector::default());
    let mut v = Vec::with_capacity(state.len() + 1);
    v.push(TAG_YRS);
    v.extend_from_slice(&state);
    v
}

/// A fresh note value (tagged 0x02) holding `s` — used when creating a note or
/// writing one to a new key (rename/import).
pub(crate) fn fresh_note(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.push(TAG_YRS);
    v.extend_from_slice(&seed_update(s));
    v
}

/// Apply the minimal single-region edit turning `old` into `new` onto a yrs text
/// whose current content is `old`. Offsets are byte offsets (OffsetKind::Bytes),
/// backed off to UTF-8 char boundaries so a codepoint is never split. Callers
/// must pass the doc's *current* text as `old`, so the offsets are always valid.
fn apply_text_diff(text: &TextRef, txn: &mut TransactionMut, old: &str, new: &str) {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());
    let max = ob.len().min(nb.len());
    let mut p = 0;
    while p < max && ob[p] == nb[p] {
        p += 1;
    }
    while p > 0 && (!old.is_char_boundary(p) || !new.is_char_boundary(p)) {
        p -= 1;
    }
    let mut s = 0;
    while s < (ob.len() - p).min(nb.len() - p) && ob[ob.len() - 1 - s] == nb[nb.len() - 1 - s] {
        s += 1;
    }
    while s > 0 && (!old.is_char_boundary(ob.len() - s) || !new.is_char_boundary(nb.len() - s)) {
        s -= 1;
    }
    let remove_len = ob.len() - p - s;
    if remove_len > 0 {
        text.remove_range(txn, p as u32, remove_len as u32);
    }
    let ins = &new[p..nb.len() - s];
    if !ins.is_empty() {
        text.insert(txn, p as u32, ins);
    }
}

/// Merge every author's stored state for a note key into one yrs doc. iroh-docs
/// keeps one entry per (key, author); reading only the newest (LWW) would drop a
/// peer's concurrent edit, so we apply them all — yrs merges them order-free.
/// `client_id` identifies edits we are about to make on the returned doc (use 0
/// for a read-only merge). Tombstones and not-yet-downloaded blobs are skipped.
/// Returns the doc and whether any content was applied.
pub(crate) async fn merged_note(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    key: &[u8],
    client_id: u64,
) -> Result<(Doc, bool)> {
    let ydoc = Doc::with_client_id(client_id);
    let mut found = false;
    let mut stream = Box::pin(doc.get_many(Query::key_exact(key)).await?);
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        if entry.content_len() == 0 {
            continue; // tombstone (deleted by some author)
        }
        // After a sync the entry can exist before its content blob has
        // downloaded; skip it (a ContentReady event re-fires the read).
        let Ok(bytes) = node.blobs.blobs().get_bytes(entry.content_hash()).await else {
            continue;
        };
        if let Ok(update) = Update::decode_v1(&value_to_update(&bytes)) {
            let mut txn = ydoc.transact_mut();
            if txn.apply_update(update).is_ok() {
                found = true;
            }
        }
    }
    Ok((ydoc, found))
}

/// Current merged text of a note, or `None` if the note is deleted or no entry's
/// content is available yet.
pub(crate) async fn read_note_text(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    key: &[u8],
) -> Result<Option<String>> {
    // Deletion (a tombstone with the newest timestamp) wins over older edits,
    // matching the pre-CRDT LWW behavior — don't resurrect a deleted note from a
    // stale per-author entry that a peer hasn't tombstoned yet.
    let live = Query::single_latest_per_key().key_exact(key);
    if !doc.get_one(live).await?.is_some_and(|e| e.content_len() > 0) {
        return Ok(None);
    }
    let (ydoc, found) = merged_note(node, doc, key, 0).await?;
    Ok(found.then(|| doc_text(&ydoc)))
}

/// Apply a whole-buffer note edit to its CRDT. The editor sends the text it
/// loaded (`base`) and the text now (`content`); we 3-way merge those against the
/// current merged state (`cur`) so a peer's concurrent edit is preserved rather
/// than overwritten (issue #99). True same-region conflicts come back as inline
/// `<<<<<<<`/`>>>>>>>` markers — surfaced, never silently dropped. The resulting
/// delta is applied relative to `cur`, so yrs offsets are always in bounds even
/// when a remote edit shifted them.
pub(crate) async fn write_note_merged(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    key: &[u8],
    base: &str,
    content: &str,
) -> Result<()> {
    let cid = client_id(node.author.to_string().as_bytes());
    let (ydoc, _) = merged_note(node, doc, key, cid).await?;
    let cur = doc_text(&ydoc);
    let merged = match diffy::merge(base, &cur, content) {
        Ok(m) | Err(m) => m,
    };
    if merged == cur {
        return Ok(()); // nothing new to store
    }
    {
        let text = ydoc.get_or_insert_text(TEXT_ROOT);
        let mut txn = ydoc.transact_mut();
        apply_text_diff(&text, &mut txn, &cur, &merged);
    }
    doc.set_bytes(node.author, key.to_vec(), encode_doc(&ydoc)).await?;
    Ok(())
}

fn peers_path(dir: &std::path::Path) -> PathBuf {
    dir.join("peers.json")
}

/// Load the per-vault peer map persisted on disk, pruning stale entries. Accepts
/// the legacy list format (`{nsid: [id,...]}`) and migrates it (last-seen = now).
fn load_peers(dir: &std::path::Path) -> PeerMap {
    let mut out = PeerMap::new();
    let Ok(s) = std::fs::read_to_string(peers_path(dir)) else {
        return out;
    };
    let now = now_secs();
    let cutoff = now.saturating_sub(PEER_TTL_SECS);
    if let Ok(raw) = serde_json::from_str::<BTreeMap<String, BTreeMap<String, u64>>>(&s) {
        for (k, ids) in raw {
            let Ok(nsid) = NamespaceId::from_str(&k) else { continue };
            let m: BTreeMap<EndpointId, u64> = ids
                .iter()
                .filter(|(_, &ts)| ts >= cutoff)
                .filter_map(|(i, &ts)| EndpointId::from_str(i).ok().map(|id| (id, ts)))
                .collect();
            if !m.is_empty() {
                out.insert(nsid, m);
            }
        }
    } else if let Ok(legacy) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(&s) {
        for (k, ids) in legacy {
            if let Ok(nsid) = NamespaceId::from_str(&k) {
                let m = ids
                    .iter()
                    .filter_map(|i| EndpointId::from_str(i).ok().map(|id| (id, now)))
                    .collect();
                out.insert(nsid, m);
            }
        }
    }
    out
}

fn save_peers(dir: &std::path::Path, map: &PeerMap) {
    let raw: BTreeMap<String, BTreeMap<String, u64>> = map
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                v.iter().map(|(id, ts)| (id.to_string(), *ts)).collect(),
            )
        })
        .collect();
    if let Ok(s) = serde_json::to_string(&raw) {
        let _ = std::fs::write(peers_path(dir), s);
    }
}

fn names_path(dir: &std::path::Path) -> PathBuf {
    dir.join("vault-names.json")
}

/// Load per-user local vault name overrides (#120). A missing file, bad JSON, or
/// blank entries are simply skipped — a vault with no override falls back to its
/// synced meta name (see `vault_info`).
fn load_names(dir: &std::path::Path) -> NameMap {
    let mut out = NameMap::new();
    let Ok(s) = std::fs::read_to_string(names_path(dir)) else {
        return out;
    };
    if let Ok(raw) = serde_json::from_str::<BTreeMap<String, String>>(&s) {
        for (k, name) in raw {
            let Ok(nsid) = NamespaceId::from_str(&k) else { continue };
            let name = name.trim().to_string();
            if !name.is_empty() {
                out.insert(nsid, name);
            }
        }
    }
    out
}

fn save_names(dir: &std::path::Path, map: &NameMap) {
    let raw: BTreeMap<String, String> =
        map.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    if let Ok(s) = serde_json::to_string(&raw) {
        let _ = std::fs::write(names_path(dir), s);
    }
}

/// Remember a vault's sync peer by EndpointId, stamping last-seen = now. Skips our
/// own id. Persists immediately for a newly-seen peer; refreshed timestamps for
/// known peers are flushed periodically (see the flush task in `init`).
fn remember_peer(
    dir: &std::path::Path,
    peers: &Mutex<PeerMap>,
    our_id: EndpointId,
    nsid: NamespaceId,
    id: EndpointId,
) {
    if id == our_id {
        return;
    }
    let mut m = peers.lock().unwrap();
    let is_new = m.entry(nsid).or_default().insert(id, now_secs()).is_none();
    if is_new {
        save_peers(dir, &m);
    }
}

/// Node-id-only addresses for a vault's known peers. Empty per-address info means
/// the endpoint resolves the live relay + direct addresses via discovery.
fn peer_addrs(peers: &Mutex<PeerMap>, nsid: NamespaceId) -> Vec<EndpointAddr> {
    peers
        .lock()
        .unwrap()
        .get(&nsid)
        .map(|s| s.keys().map(|id| EndpointAddr::new(*id)).collect())
        .unwrap_or_default()
}

/// Prune peers not seen within PEER_TTL_SECS and persist. Returns true if any
/// were dropped.
fn prune_peers(dir: &std::path::Path, peers: &Mutex<PeerMap>) {
    let cutoff = now_secs().saturating_sub(PEER_TTL_SECS);
    let mut m = peers.lock().unwrap();
    let before: usize = m.values().map(|v| v.len()).sum();
    for v in m.values_mut() {
        v.retain(|_, &mut ts| ts >= cutoff);
    }
    m.retain(|_, v| !v.is_empty());
    let after: usize = m.values().map(|v| v.len()).sum();
    // Always persist: refreshes on-disk last-seen for live peers so they don't
    // get pruned later just because their timestamp was only ever set on insert.
    if before != after || !m.is_empty() {
        save_peers(dir, &m);
    }
}

/// Write a credential file readable only by the owner (0o600 on Unix).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

// A clone of the live node bits needed to recover sync after the OS freezes our
// sockets without telling iroh. Used by the recovery hooks below:
//   - Android: the JNI network-change / app-resume callbacks (ConnectivityManager
//     + onResume) — Android doesn't surface these to native code.
//   - Desktop: the wake-from-suspend detector (macOS et al. freeze sockets +
//     relay paths during sleep, and a same-IP wake doesn't trip iroh's monitor).
#[cfg(any(target_os = "android", desktop))]
struct NetHandle {
    endpoint: Endpoint,
    docs: Docs,
    watched: std::sync::Arc<Mutex<HashSet<NamespaceId>>>,
    peers: std::sync::Arc<Mutex<PeerMap>>,
}
#[cfg(any(target_os = "android", desktop))]
static NET: std::sync::OnceLock<NetHandle> = std::sync::OnceLock::new();

// Re-probe the endpoint and re-dial every watched vault's peers. Shared by the
// network-change, app-resume, and wake-from-suspend hooks: all leave iroh with
// stale sockets/paths it can't detect, and the recovery action is identical.
// Crucially this re-calls start_sync directly, bypassing arm_vault's "already
// watched" guard — the vault stays watched across a freeze, so re-arming it that
// way would be a no-op and never re-dial.
#[cfg(any(target_os = "android", desktop))]
fn rearm_sync() {
    let Some(net) = NET.get() else { return };
    let endpoint = net.endpoint.clone();
    let docs = net.docs.clone();
    let peers = net.peers.clone();
    let nsids: Vec<NamespaceId> = net.watched.lock().unwrap().iter().copied().collect();
    tauri::async_runtime::spawn(async move {
        tracing::info!(vaults = nsids.len(), "re-arming sync");
        // Tell iroh to re-probe (Android doesn't surface this natively).
        // Discovery + relay then do the heavy lifting: re-dial each open vault's
        // peers by EndpointId and let iroh resolve the new transport (relay
        // after a wifi drop, direct addrs after holepunch).
        endpoint.network_change().await;
        for nsid in nsids {
            if let Ok(Some(doc)) = docs.open(nsid).await {
                let _ = doc.start_sync(peer_addrs(&peers, nsid)).await;
            }
        }
    });
}

/// Fired from Kotlin's ConnectivityManager callback on a default-network change
/// (e.g. wifi <-> cellular handoff).
#[cfg(target_os = "android")]
pub fn notify_network_change() {
    rearm_sync();
}

/// Fired from MainActivity.onResume. Android freezes the process while
/// backgrounded; its UDP sockets and relay connections can go stale with no
/// native signal to iroh, so sync stays dead until a full restart. Re-arm on
/// foreground so it recovers without a kill+relaunch. (Issues #49, #5.)
#[cfg(target_os = "android")]
pub fn on_resume() {
    rearm_sync();
}

/// Build the persistent iroh node (endpoint + blobs + gossip + docs on a router).
pub async fn init(dir: PathBuf) -> Result<Node> {
    std::fs::create_dir_all(&dir)?;

    // Persist the endpoint secret key so the node id is stable across restarts.
    // Without this, iroh generates a fresh key each launch → a new node id →
    // persisted sync peers reference a dead id and never reconnect.
    let key_path = dir.join("endpoint-secret");
    let secret = match std::fs::read(&key_path) {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            SecretKey::from_bytes(&arr)
        }
        _ => {
            let sk = SecretKey::generate();
            write_private(&key_path, &sk.to_bytes())?;
            sk
        }
    };
    // N0 preset = pkarr publish + n0 DNS resolution + relays. Add mDNS so peers
    // on the same LAN discover each other directly, even when n0 DNS/relays are
    // unreachable (restrictive wifi, captive networks) — fixes same-network sync
    // when a peer can't publish to pkarr.
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .address_lookup(MdnsAddressLookup::builder())
        .bind()
        .await?;
    // Blob GC. Without it, every saved note version leaves an orphaned content
    // blob on disk forever. The protect handler lets iroh-docs report the blobs
    // still referenced by current entries across ALL replicas, so GC only sweeps
    // true orphans — vaults that aren't currently open stay protected too.
    let (protect_handler, protect_cb) = ProtectCallbackHandler::new();
    let blobs_root = dir.join("blobs");
    let mut blob_opts = Options::new(&blobs_root);
    blob_opts.gc = Some(GcConfig {
        interval: GC_INTERVAL,
        add_protected: Some(protect_cb),
    });
    let blobs = FsStore::load_with_opts(blobs_root.join("blobs.db"), blob_opts).await?;
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let docs = Docs::persistent(dir.clone())
        .protect_handler(protect_handler)
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;

    let watched = std::sync::Arc::new(Mutex::new(HashSet::new()));
    let our_id = endpoint.id();
    let peers = std::sync::Arc::new(Mutex::new(load_peers(&dir)));
    let names = std::sync::Arc::new(Mutex::new(load_names(&dir)));
    // Periodically refresh on-disk peer last-seen and prune dead peers, so an
    // actively-syncing peer keeps a fresh timestamp (avoids being pruned) while
    // long-gone devices age out of the cache.
    {
        let peers = peers.clone();
        let dir = dir.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                prune_peers(&dir, &peers);
            }
        });
    }
    #[cfg(any(target_os = "android", desktop))]
    let _ = NET.set(NetHandle {
        endpoint: endpoint.clone(),
        docs: docs.clone(),
        watched: watched.clone(),
        peers: peers.clone(),
    });
    // Desktop wake-from-suspend recovery. macOS (and others) freeze our UDP
    // sockets + relay paths during sleep; on wake the IP is often unchanged, so
    // iroh's own monitor doesn't fire and live sync stays dead until restart
    // (#107). Android gets the same recovery from MainActivity.onResume; desktop
    // has no such signal, so we infer a wake by watching wall-clock for a jump.
    // We compare SystemTime, not Instant: macOS's monotonic clock pauses during
    // sleep, so a sleeping timer wouldn't reveal the gap, but wall-clock does.
    #[cfg(desktop)]
    {
        tauri::async_runtime::spawn(async move {
            let tick = Duration::from_secs(10);
            // Tolerate normal scheduling slop + small NTP steps; only a real
            // suspend produces a multi-minute jump.
            let wake_threshold = tick + Duration::from_secs(20);
            loop {
                let before = std::time::SystemTime::now();
                tokio::time::sleep(tick).await;
                let elapsed = std::time::SystemTime::now()
                    .duration_since(before)
                    .unwrap_or(tick);
                if elapsed > wake_threshold {
                    tracing::info!(
                        gap_secs = elapsed.as_secs(),
                        "wake from suspend detected; re-arming sync"
                    );
                    rearm_sync();
                }
            }
        });
    }

    let router = Router::builder(endpoint)
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();

    let author = docs.author_default().await?;

    Ok(Node {
        blobs,
        docs,
        author,
        _router: router,
        watched,
        dir,
        our_id,
        peers,
        names,
        changes: tokio::sync::broadcast::channel(CHANGE_CHANNEL_CAP).0,
    })
}

pub(crate) fn parse_id(id: &str) -> Result<NamespaceId> {
    NamespaceId::from_str(id).map_err(|e| anyhow!("bad vault id: {e}"))
}

pub(crate) async fn open(node: &Node, id: &str) -> Result<iroh_docs::api::Doc> {
    let nsid = parse_id(id)?;
    node.docs
        .open(nsid)
        .await?
        .ok_or_else(|| anyhow!("vault not found"))
}

async fn read_key(node: &Node, doc: &iroh_docs::api::Doc, key: &[u8]) -> Result<Option<String>> {
    // single_latest_per_key: when a note has been edited on multiple devices it
    // has one entry per author; we want the newest across all authors. A plain
    // key_exact query sorts by author and returns an arbitrary (often stale) one,
    // which made a peer's edit invisible once we'd also edited the same note.
    let query = Query::single_latest_per_key().key_exact(key);
    let Some(entry) = doc.get_one(query).await? else {
        return Ok(None);
    };
    // After a sync the entry can exist before its content blob has downloaded;
    // get_bytes then errors. Treat that as "not available yet" — iroh emits a
    // ContentReady event (→ vault-changed) that triggers a refetch once it lands.
    match node.blobs.blobs().get_bytes(entry.content_hash()).await {
        Ok(bytes) => Ok(Some(decode(&bytes))),
        Err(_) => Ok(None),
    }
}

/// The vault's user-visible name from its synced meta, or `None` if the
/// `\x00meta/name` entry hasn't arrived yet (freshly joined, no peer synced).
async fn vault_meta_name(node: &Node, doc: &iroh_docs::api::Doc) -> Option<String> {
    match read_key(node, doc, NAME_KEY).await {
        Ok(Some(name)) if !name.is_empty() => Some(name),
        _ => None,
    }
}

/// First 6 hex chars of the NamespaceId, shown after the name to disambiguate
/// vaults sharing a local display name (#120). Idempotent, stable across devices.
fn short_hash(id: &iroh_docs::NamespaceId) -> String {
    let s = id.to_string();
    s[..s.len().min(6)].to_string()
}

/// Placeholder name for a vault whose real name hasn't synced yet.
fn fallback_name(id: &iroh_docs::NamespaceId) -> String {
    format!("vault-{}", short_hash(id))
}

/// Build a `VaultInfo`. Effective name = local override (#120) ?? synced meta
/// name ?? fallback. No explicit meta->local backfill on upgrade: with no
/// override, resolution falls through to the synced meta name, so existing users
/// keep their current name automatically. `pending` reflects ONLY whether the
/// synced meta has arrived — a local override does not clear it, because a
/// renamed-but-not-yet-synced vault is still waiting for peer content.
fn vault_info(
    id: iroh_docs::NamespaceId,
    meta_name: Option<String>,
    override_name: Option<String>,
) -> VaultInfo {
    VaultInfo {
        pending: meta_name.is_none(),
        name: override_name
            .or(meta_name)
            .unwrap_or_else(|| fallback_name(&id)),
        hash: short_hash(&id),
        id: id.to_string(),
    }
}

/// A visible entry in a vault: its path plus the metadata a caller can use
/// without reading (and CRDT-merging) the content itself.
pub(crate) struct NoteEntry {
    pub path: String,
    /// Entry timestamp, microseconds since the unix epoch (iroh-docs' unit).
    pub modified_us: u64,
}

/// Collect all visible entries in a vault, newest first.
///
/// Tombstoned keys are excluded by content length, not by trusting the query to
/// drop them. A delete only ever writes an empty record under the *deleting*
/// author (iroh-docs scopes removal to `author_prefix`), so when a note was
/// written on another device the key still has that peer's live record
/// alongside our newer tombstone. `single_latest_per_key` hands back the newest
/// of the two — our tombstone — and without this check the key would keep
/// showing up as a live note. Same rule `key_exists` already applies.
pub(crate) async fn list_entries(doc: &iroh_docs::api::Doc) -> Result<Vec<NoteEntry>> {
    let mut stream = Box::pin(doc.get_many(Query::single_latest_per_key()).await?);
    let mut out = Vec::new();
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        if entry.content_len() == 0 {
            continue; // deleted (tombstone wins over any older live record)
        }
        let key = entry.key();
        if key.first() == Some(&0x00) {
            continue; // reserved meta keys
        }
        if let Ok(s) = std::str::from_utf8(key) {
            out.push(NoteEntry {
                path: s.to_string(),
                modified_us: entry.timestamp(),
            });
        }
    }
    out.sort_by(|a, b| b.modified_us.cmp(&a.modified_us));
    Ok(out)
}

/// Collect all visible note keys (paths) in a vault.
pub(crate) async fn list_keys(doc: &iroh_docs::api::Doc) -> Result<Vec<String>> {
    Ok(list_entries(doc).await?.into_iter().map(|e| e.path).collect())
}

// ============================ search + tags ============================

/// Deleted notes are moved here rather than tombstoned, so they must be kept out
/// of listings, search results and tag counts. (The MCP `delete_note` tool is
/// what puts them there; a real delete would propagate to every synced device.)
pub(crate) const TRASH: &str = ".trash";

/// Notes are scanned one at a time, and each scan is a blob read plus a CRDT
/// merge — so cap the work rather than stalling on a huge vault.
pub(crate) const SEARCH_SCAN_LIMIT: usize = 2000;

/// Keys that aren't user-visible notes: folder markers and anything trashed.
pub(crate) fn is_hidden_path(path: &str) -> bool {
    path.ends_with(KEEP)
        || path.ends_with('/')
        || path == TRASH
        || path.starts_with(&format!("{TRASH}/"))
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    /// Matching lines as `{line_number}: {text}`, at most 3 per note.
    pub lines: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TagCount {
    pub tag: String,
    /// How many notes carry the tag (not how many times it occurs).
    pub count: usize,
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

/// Pull inline `#tag`s out of a note body, in order of appearance, de-duplicated
/// case-insensitively.
///
/// Tags live in the Markdown itself (no sidecar storage), so they sync through
/// the same CRDT as the text and survive export/import. The rules exist to keep
/// them from colliding with ordinary Markdown:
///
/// - `#` must start the line or follow whitespace, so `example.com/#anchor` and
///   `C#` inside a word are not tags.
/// - The next character must be alphanumeric, which is exactly what separates a
///   tag from an ATX heading — `# Heading` has a space and is not a tag.
/// - `_`, `-` and `/` continue a tag (so `#in/progress`, `#q3-goals` work), but
///   trailing ones are trimmed so `#work.` and `#work/` both yield `work`.
pub(crate) fn extract_tags(text: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].1 != '#' || !(i == 0 || chars[i - 1].1.is_whitespace()) {
            i += 1;
            continue;
        }
        let body = i + 1;
        let mut j = body;
        while j < chars.len() && is_tag_char(chars[j].1) {
            j += 1;
        }
        // `j > body` rules out a bare "#"; the alphanumeric test rules out
        // "# Heading" and oddities like "#-".
        if j > body && chars[body].1.is_alphanumeric() {
            let end = chars.get(j).map_or(text.len(), |(p, _)| *p);
            let tag = text[chars[body].0..end].trim_end_matches(['-', '/', '_']);
            if !tag.is_empty() && !out.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                out.push(tag.to_string());
            }
        }
        i = j.max(body);
    }
    out
}

/// Case-insensitive substring search across a vault's notes.
///
/// Deliberately not a regex and not indexed: every note has to be merged out of
/// the CRDT to be read at all, so the scan dominates either way, and a personal
/// vault is small. Shared by the in-app search and the MCP `search_notes` tool
/// so the two can't drift.
pub(crate) async fn search(
    node: &Node,
    vault: &str,
    query: &str,
    path_contains: Option<&str>,
    max: usize,
) -> Result<Vec<SearchHit>> {
    let doc = open(node, vault).await?;
    let needle = query.to_lowercase();
    let filter = path_contains.map(str::to_lowercase);
    let mut hits = Vec::new();
    let mut scanned = 0usize;
    for entry in list_entries(&doc).await? {
        if hits.len() >= max || scanned >= SEARCH_SCAN_LIMIT {
            break;
        }
        if is_hidden_path(&entry.path) {
            continue;
        }
        if filter.as_ref().is_some_and(|p| !entry.path.to_lowercase().contains(p)) {
            continue;
        }
        scanned += 1;
        // Not synced yet — the entry exists but its content blob hasn't landed.
        let Some(text) = read_note_text(node, &doc, entry.path.as_bytes()).await? else {
            continue;
        };
        let lines: Vec<String> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&needle))
            .take(3)
            .map(|(i, l)| format!("{}: {}", i + 1, l.trim()))
            .collect();
        if !lines.is_empty() {
            hits.push(SearchHit {
                path: entry.path,
                lines,
            });
        }
    }
    Ok(hits)
}

/// Every tag in a vault with the number of notes carrying it, most-used first
/// (ties broken alphabetically so the order is stable).
pub(crate) async fn tags_in_vault(node: &Node, vault: &str) -> Result<Vec<TagCount>> {
    let doc = open(node, vault).await?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut scanned = 0usize;
    for entry in list_entries(&doc).await? {
        if scanned >= SEARCH_SCAN_LIMIT {
            break;
        }
        if is_hidden_path(&entry.path) {
            continue;
        }
        scanned += 1;
        let Some(text) = read_note_text(node, &doc, entry.path.as_bytes()).await? else {
            continue;
        };
        for tag in extract_tags(&text) {
            *counts.entry(tag).or_default() += 1;
        }
    }
    let mut out: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, count)| TagCount { tag, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
    Ok(out)
}

async fn key_exists(doc: &iroh_docs::api::Doc, key: &str) -> Result<bool> {
    // single_latest_per_key drops tombstones, and content_len 0 catches any
    // residual empty record — so a *deleted* note's key counts as free. A raw
    // key_exact (no single_latest_per_key) would still see the tombstone and
    // make free_key append " 1" to a recreated name (#48 ghost file).
    let query = Query::single_latest_per_key().key_exact(key.as_bytes());
    Ok(doc.get_one(query).await?.is_some_and(|e| e.content_len() > 0))
}

/// Find a free key by inserting/incrementing a numeric suffix before the
/// extension: `Untitled.md` → `Untitled 1.md` → `Untitled 2.md` …
pub(crate) async fn free_key(doc: &iroh_docs::api::Doc, path: &str) -> Result<String> {
    if !key_exists(doc, path).await? {
        return Ok(path.to_string());
    }
    let (stem, ext) = match path.rfind('.') {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, ""),
    };
    let mut n = 1;
    loop {
        let cand = format!("{stem} {n}{ext}");
        if !key_exists(doc, &cand).await? {
            return Ok(cand);
        }
        n += 1;
    }
}

// ---- intermediate tree node used while building ----
#[derive(Default)]
struct Builder {
    dirs: BTreeMap<String, Builder>,
    files: BTreeMap<String, String>, // name -> full path
}

fn insert_path(root: &mut Builder, segments: &[&str], full: &str, is_file: bool) {
    match segments {
        [] => {}
        [last] => {
            if is_file {
                root.files.insert((*last).to_string(), full.to_string());
            } else {
                root.dirs.entry((*last).to_string()).or_default();
            }
        }
        [head, rest @ ..] => {
            let child = root.dirs.entry((*head).to_string()).or_default();
            insert_path(child, rest, full, is_file);
        }
    }
}

fn to_nodes(b: &Builder, prefix: &str) -> Vec<TreeNode> {
    let mut out = Vec::new();
    for (name, child) in &b.dirs {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        out.push(TreeNode {
            name: name.clone(),
            path: path.clone(),
            is_dir: true,
            children: to_nodes(child, &path),
        });
    }
    for (name, full) in &b.files {
        out.push(TreeNode {
            name: name.clone(),
            path: full.clone(),
            is_dir: false,
            children: Vec::new(),
        });
    }
    out
}

fn map_err<T>(r: Result<T>) -> Result<T, String> {
    r.map_err(|e| {
        let s = e.to_string();
        eprintln!("[vault] command error: {s}");
        s
    })
}

// ============================ commands ============================

/// Every vault this device holds, with its effective display name. Shared by the
/// `list_vaults` command and the MCP server.
pub(crate) async fn all_vaults(node: &Node) -> Result<Vec<VaultInfo>> {
    let mut stream = node.docs.list().await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let (id, _cap) = item?;
        let doc = node.docs.open(id).await?.ok_or_else(|| anyhow!("open failed"))?;
        let meta_name = vault_meta_name(node, &doc).await;
        let override_name = node.names.lock().unwrap().get(&id).cloned();
        out.push(vault_info(id, meta_name, override_name));
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_vaults(state: State<'_, VaultManager>) -> Result<Vec<VaultInfo>, String> {
    let node = state.node().await?;
    map_err(all_vaults(node).await)
}

#[tauri::command]
pub async fn create_vault(state: State<'_, VaultManager>, name: String) -> Result<VaultInfo, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = node.docs.create().await?;
            doc.set_bytes(node.author, NAME_KEY.to_vec(), encode(&name)).await?;
            // A vault we create has its name immediately, so it's never pending.
            // No local override yet — the synced-meta name is the default (#120).
            Ok(vault_info(doc.id(), Some(name), None))
        }
        .await,
    )
}

#[tauri::command]
pub async fn join_vault(state: State<'_, VaultManager>, ticket: String) -> Result<VaultInfo, String> {
    let node = state.node().await?;
    map_err(
        async {
            let ticket = DocTicket::from_str(&ticket).map_err(|e| anyhow!("bad ticket: {e}"))?;
            let DocTicket { capability, nodes } = ticket;
            let nsid = capability.id();
            // Remember the ticket's peers by EndpointId so we can re-dial them
            // via discovery later, even after their addresses change.
            for n in &nodes {
                remember_peer(&node.dir, &node.peers, node.our_id, nsid, n.id);
            }
            // Re-pair safe: if we already have this namespace, open it; otherwise
            // import it. open() errors ("Replica not found") for a namespace we
            // don't have, so treat any non-Some result as "needs import".
            let doc = match node.docs.open(nsid).await {
                Ok(Some(doc)) => doc,
                _ => node.docs.import_namespace(capability).await?,
            };
            // Bootstrap with the ticket's full addresses (relay) for a robust
            // first connect; subsequent syncs re-dial by EndpointId via discovery.
            doc.start_sync(nodes).await?;
            // start_sync is non-blocking: with no peer online the meta name
            // isn't here yet, so this comes back pending and the UI shows a
            // "waiting for a peer" state rather than a generated vault (#4).
            let meta_name = vault_meta_name(node, &doc).await;
            let override_name = node.names.lock().unwrap().get(&doc.id()).cloned();
            Ok(vault_info(doc.id(), meta_name, override_name))
        }
        .await,
    )
}

#[tauri::command]
pub async fn share_vault(state: State<'_, VaultManager>, vault: String) -> Result<String, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            // Write capability → flat, equal ownership across all synced devices.
            // Relay (node id + relay URL) keeps the ticket short enough for a
            // low-density QR while staying resilient: the relay guarantees the
            // first connection, then holepunching upgrades to direct addresses
            // and iroh-docs persists the discovered sync peers for next time.
            let ticket = doc
                .share(ShareMode::Write, AddrInfoOptions::Relay)
                .await?;
            Ok(ticket.to_string())
        }
        .await,
    )
}

/// Stop syncing a vault and drop its local replica. The namespace can be
/// rejoined later from a ticket (re-pair safe via join_vault). Other peers keep
/// their copy — this only forgets it on this device.
#[tauri::command]
pub async fn forget_vault(state: State<'_, VaultManager>, vault: String) -> Result<(), String> {
    let node = state.node().await?;
    let nsid = parse_id(&vault).map_err(|e| e.to_string())?;
    node.watched.lock().unwrap().remove(&nsid);
    // Forget this vault's cached peers too, so we don't keep dialing them.
    {
        let mut m = node.peers.lock().unwrap();
        if m.remove(&nsid).is_some() {
            save_peers(&node.dir, &m);
        }
    }
    // Drop this vault's local name override too (#120), so a later rejoin starts
    // from the synced-meta default rather than a stale local name.
    {
        let mut m = node.names.lock().unwrap();
        if m.remove(&nsid).is_some() {
            save_names(&node.dir, &m);
        }
    }
    map_err(
        async {
            // leave() ends live sync.
            if let Ok(Some(doc)) = node.docs.open(nsid).await {
                let _ = doc.leave().await;
            }
            // drop_doc requires the replica's open-handle count to be 0, but
            // iroh-docs' `Doc` has no Drop-close, so every open() we ever made
            // for this namespace leaked a handle. Each drop_doc attempt releases
            // one handle (via its internal close) even when it then fails with
            // "replica is not closed", so retry until the count hits 0 and the
            // remove succeeds. Bounded so a genuine error can't spin forever.
            for _ in 0..256 {
                match node.docs.drop_doc(nsid).await {
                    Ok(()) => return Ok(()),
                    Err(e) if e.to_string().contains("not closed") => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            Err(anyhow!("could not close vault replica to drop it"))
        }
        .await,
    )
}

/// Set this device's LOCAL display name for a vault (#120). Writes only
/// `vault-names.json`; never touches the synced meta, so peers keep their own
/// names. An empty/whitespace name clears the override (falls back to the synced
/// meta name / hash).
#[tauri::command]
pub async fn rename_vault(
    app: AppHandle,
    state: State<'_, VaultManager>,
    vault: String,
    name: String,
) -> Result<(), String> {
    let node = state.node().await?;
    let nsid = parse_id(&vault).map_err(|e| e.to_string())?;
    let name = name.trim().to_string();
    {
        let mut m = node.names.lock().unwrap();
        if name.is_empty() {
            m.remove(&nsid);
        } else {
            m.insert(nsid, name);
        }
        save_names(&node.dir, &m);
    }
    // Refresh the UI. The FE also re-reads after the invoke resolves, so this is
    // a belt-and-suspenders refresh for the active vault.
    let _ = app.emit("vault-changed", vault);
    Ok(())
}

/// Build the vault's folder tree from its keys. Shared by the `list_tree`
/// command and the MCP server's tree resource.
pub(crate) async fn build_tree(doc: &iroh_docs::api::Doc) -> Result<Vec<TreeNode>> {
    let keys = list_keys(doc).await?;
    let mut root = Builder::default();
    for key in &keys {
        let segments: Vec<&str> = key.split('/').collect();
        if segments.last() == Some(&KEEP) {
            let dir_segments = &segments[..segments.len() - 1];
            if !dir_segments.is_empty() {
                insert_path(&mut root, dir_segments, "", false);
            }
        } else {
            insert_path(&mut root, &segments, key, true);
        }
    }
    Ok(to_nodes(&root, ""))
}

#[tauri::command]
pub async fn list_tree(state: State<'_, VaultManager>, vault: String) -> Result<Vec<TreeNode>, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            build_tree(&doc).await
        }
        .await,
    )
}

/// Search a vault's notes (#15). Case-insensitive substring match; see `search`.
#[tauri::command]
pub async fn search_notes(
    state: State<'_, VaultManager>,
    vault: String,
    query: String,
    max: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let node = state.node().await?;
    // An empty query would match every line of every note — return nothing
    // rather than the whole vault while the user is still typing.
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    map_err(search(node, &vault, &query, None, max.unwrap_or(20).clamp(1, 100)).await)
}

/// Every inline `#tag` in a vault, with how many notes use it (#15).
#[tauri::command]
pub async fn list_tags(
    state: State<'_, VaultManager>,
    vault: String,
) -> Result<Vec<TagCount>, String> {
    let node = state.node().await?;
    map_err(tags_in_vault(node, &vault).await)
}

#[tauri::command]
pub async fn read_note(state: State<'_, VaultManager>, vault: String, path: String) -> Result<String, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            Ok(read_note_text(node, &doc, path.as_bytes()).await?.unwrap_or_default())
        }
        .await,
    )
}

/// Export every note in the vault as a zip of `.md` files mirroring the folder
/// tree (#79). Empty folders are preserved as zip directory entries; the meta
/// name entry is skipped. Returns the zip bytes for the frontend to save.
#[tauri::command]
pub async fn export_vault(state: State<'_, VaultManager>, vault: String) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            let keys = list_keys(&doc).await?;
            // Read all contents first — the zip writer isn't Send, so it must not
            // be held across an await. (name, Some(content)) = file; None = dir.
            let mut items: Vec<(String, Option<String>)> = Vec::new();
            for key in &keys {
                if key.as_bytes().first() == Some(&0) {
                    continue; // \x00meta/* — internal, not a note
                }
                if key == KEEP || key.ends_with(&format!("/{KEEP}")) {
                    let dir = &key[..key.len() - KEEP.len()]; // keeps trailing '/'
                    if !dir.is_empty() {
                        items.push((dir.to_string(), None));
                    }
                    continue;
                }
                let content = read_note_text(node, &doc, key.as_bytes()).await?.unwrap_or_default();
                items.push((key.clone(), Some(content)));
            }
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
            for (name, content) in items {
                match content {
                    None => zip.add_directory(&name, opts)?,
                    Some(c) => {
                        zip.start_file(&name, opts)?;
                        zip.write_all(c.as_bytes())?;
                    }
                }
            }
            Ok(zip.finish()?.into_inner())
        }
        .await,
    )
}

/// Import a zip of `.md` files into the vault (#79), recreating the folder tree.
/// Name collisions are de-duplicated against existing notes (free_key). Empty
/// directory entries recreate empty folders. Returns the number of notes added.
#[tauri::command]
pub async fn import_vault(
    state: State<'_, VaultManager>,
    vault: String,
    data: Vec<u8>,
) -> Result<usize, String> {
    use std::io::Read;
    let node = state.node().await?;
    map_err(
        async {
            // Drain the archive into memory first — ZipArchive isn't Send, so it
            // must be fully read (and dropped) before any await.
            let items: Vec<(String, Option<String>)> = {
                let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))?;
                let mut v = Vec::new();
                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i)?;
                    let name = entry.name().replace('\\', "/");
                    let name = name.trim_start_matches('/').to_string();
                    // Entry names become iroh-docs keys, so reject a crafted
                    // archive's path traversal ('..'), empty segments, and
                    // meta-namespace injection (a \x00-prefixed segment).
                    if name
                        .trim_end_matches('/')
                        .split('/')
                        .any(|c| c.is_empty() || c == ".." || c.as_bytes().first() == Some(&0))
                    {
                        continue;
                    }
                    if entry.is_dir() {
                        v.push((name, None));
                        continue;
                    }
                    let lower = name.to_lowercase();
                    if !(lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")) {
                        continue; // only text/markdown
                    }
                    let mut s = String::new();
                    if entry.read_to_string(&mut s).is_err() {
                        continue; // non-utf8 — skip rather than fail the whole import
                    }
                    v.push((name, Some(s)));
                }
                v
            };
            let doc = open(node, &vault).await?;
            let mut count = 0usize;
            for (name, content) in items {
                match content {
                    None => {
                        let key = format!("{}{}", name.trim_end_matches('/'), format!("/{KEEP}"));
                        doc.set_bytes(node.author, key.into_bytes(), encode("")).await?;
                    }
                    Some(c) => {
                        // .txt imports normalize to .md so they open as notes.
                        let path = if name.to_lowercase().ends_with(".txt") {
                            format!("{}.md", &name[..name.len() - 4])
                        } else {
                            name
                        };
                        let free = free_key(&doc, &path).await?;
                        doc.set_bytes(node.author, free.into_bytes(), fresh_note(&c)).await?;
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        .await,
    )
}

/// Save a note. `base` is the text the editor last loaded; it lets the backend
/// 3-way merge the user's buffer (`content`) against any concurrent peer edit
/// instead of overwriting it (see `write_note_merged`, issue #99).
#[tauri::command]
pub async fn write_note(
    state: State<'_, VaultManager>,
    vault: String,
    path: String,
    base: String,
    content: String,
) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            write_note_merged(node, &doc, path.as_bytes(), &base, &content).await
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_note(state: State<'_, VaultManager>, vault: String, path: String) -> Result<String, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            let free = free_key(&doc, &path).await?;
            doc.set_bytes(node.author, free.clone().into_bytes(), fresh_note("")).await?;
            Ok(free)
        }
        .await,
    )
}

/// Create a folder by writing its `.keep` marker (folders are implicit from key
/// prefixes, so an empty one needs a marker entry to exist at all).
pub(crate) async fn create_folder_key(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    path: &str,
) -> Result<()> {
    let key = format!("{}/{}", path.trim_end_matches('/'), KEEP);
    doc.set_bytes(node.author, key.into_bytes(), encode("")).await?;
    Ok(())
}

#[tauri::command]
pub async fn create_folder(state: State<'_, VaultManager>, vault: String, path: String) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            create_folder_key(node, &doc, &path).await
        }
        .await,
    )
}

/// Remove every entry under `prefix`, whichever device wrote it.
///
/// Order matters, and getting it wrong is what made folder renames leave a
/// duplicate behind. `del` writes ONE empty record at `(our author, prefix)`,
/// and iroh-docs scopes removal to `author_prefix` — so it clears *our* records
/// under the prefix and nothing else. Two consequences:
///
/// 1. A prefix tombstone never shadows `old/a.md` written by another device, so
///    peer-authored notes need an exact-key tombstone each (newer than the
///    peer's record, so the key reads as deleted).
/// 2. Those per-key tombstones are themselves *our* records under the prefix —
///    so a prefix `del` afterwards deletes them again. Worse, removing our
///    tombstone RESURRECTS the peer's record underneath it, which is how a
///    folder came back after being deleted. Sweep the prefix FIRST.
///
/// So: sweep, then re-read the listing and tombstone whatever still reads as
/// live. The listing must be taken *after* the sweep — a pre-sweep listing
/// misses exactly the keys the sweep just resurrected.
pub(crate) async fn clear_prefix(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    prefix: &str,
) -> Result<()> {
    // Clears our own entries under the folder, plus the folder marker key
    // itself (an empty folder whose only entry is "work/"). `del` takes a
    // prefix, so there is no way to drop the marker without this side effect.
    doc.del(node.author, prefix.as_bytes().to_vec()).await?;
    // Anything still live under the prefix was written by another device (or
    // was just uncovered by the sweep); tombstone it by exact key.
    for key in list_keys(doc).await?.iter().filter(|k| k.starts_with(prefix)) {
        doc.del(node.author, key.clone().into_bytes()).await?;
    }
    Ok(())
}

/// Move a note or folder to a new key. Copies the merged CRDT state to the new
/// key (preserving edit history), then tombstones the old one. A note whose
/// content blob hasn't synced yet reads as empty (`merged_note` `found == false`);
/// copying it would write a blank note and the tombstone would lose the original,
/// so we abort instead and let the caller retry once sync catches up.
pub(crate) async fn rename_key(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    from: &str,
    to: &str,
    is_dir: bool,
) -> Result<()> {
    if is_dir {
        let from_prefix = format!("{}/", from.trim_end_matches('/'));
        let to_prefix = format!("{}/", to.trim_end_matches('/'));
        let keys = list_keys(doc).await?;
        // Stage every child first; bail before mutating if any isn't ready.
        let mut staged = Vec::new();
        for key in keys.iter().filter(|k| k.starts_with(&from_prefix)) {
            let new_key = format!("{}{}", to_prefix, &key[from_prefix.len()..]);
            let (ydoc, found) = merged_note(node, doc, key.as_bytes(), 0).await?;
            if !found {
                return Err(anyhow!("folder contents still syncing; try again"));
            }
            staged.push((new_key, encode_doc(&ydoc)));
        }
        for (new_key, val) in staged {
            doc.set_bytes(node.author, new_key.into_bytes(), val).await?;
        }
        clear_prefix(node, doc, &from_prefix).await?;
    } else {
        let (ydoc, found) = merged_note(node, doc, from.as_bytes(), 0).await?;
        if !found {
            return Err(anyhow!("note content still syncing; try again"));
        }
        doc.set_bytes(node.author, to.as_bytes().to_vec(), encode_doc(&ydoc)).await?;
        doc.del(node.author, from.as_bytes().to_vec()).await?;
    }
    Ok(())
}

/// Tombstone a note, or every entry under a folder.
pub(crate) async fn delete_key(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    path: &str,
    is_dir: bool,
) -> Result<()> {
    if is_dir {
        // Recursively remove every entry under the folder, including notes
        // written by other devices — see clear_prefix for why the order matters.
        let prefix = format!("{}/", path.trim_end_matches('/'));
        clear_prefix(node, doc, &prefix).await?;
    } else {
        doc.del(node.author, path.as_bytes().to_vec()).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn rename_path(
    state: State<'_, VaultManager>,
    vault: String,
    from: String,
    to: String,
    is_dir: bool,
) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            rename_key(node, &doc, &from, &to, is_dir).await
        }
        .await,
    )
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, VaultManager>,
    vault: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            delete_key(node, &doc, &path, is_dir).await
        }
        .await,
    )
}

/// Begin live-syncing a vault and emitting `vault-changed` on every mutation:
/// dial its known peers, subscribe to the doc, and spawn a task that re-emits
/// changes and learns new peers. Idempotent — a vault already armed is a no-op,
/// so it's safe to call from both `watch_vault` (the open vault) and
/// `set_live_sync` (every vault).
pub(crate) async fn arm_vault(
    app: &AppHandle,
    node: &Node,
    nsid: NamespaceId,
) -> Result<(), String> {
    {
        let mut watched = node.watched.lock().unwrap();
        if !watched.insert(nsid) {
            return Ok(()); // already armed
        }
    }
    let doc = node
        .docs
        .open(nsid)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "vault not found".to_string())?;
    // Resume live sync by actively dialing the vault's known peers by EndpointId.
    // (start_sync with an empty peer list only listens — it doesn't re-initiate,
    // which is why sync didn't recover after a restart.) Discovery resolves each
    // peer's current relay + addresses, so this works across network changes.
    let _ = doc.start_sync(peer_addrs(&node.peers, nsid)).await;
    let mut stream = doc.subscribe().await.map_err(|e| e.to_string())?;
    let vault_id = nsid.to_string();
    let peers = node.peers.clone();
    let dir = node.dir.clone();
    let our_id = node.our_id;
    let changes = node.changes.clone();
    let app = app.clone();
    // Initial nudge: on join the vault name (and other meta) can finish syncing
    // in the gap between join reading it and this subscription starting, so that
    // ContentReady event is missed. Emit once now so the UI re-reads whatever
    // landed in that window (e.g. the real name replacing "vault-xxxx").
    let _ = app.emit("vault-changed", &vault_id);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = stream.next().await {
            let Ok(ev) = event else { continue };
            // Learn peers from live sync so re-dial works in both directions —
            // crucially, the sharer discovers the joiner's EndpointId this way.
            match &ev {
                LiveEvent::NeighborUp(id) => remember_peer(&dir, &peers, our_id, nsid, *id),
                LiveEvent::InsertRemote { from, .. } => {
                    remember_peer(&dir, &peers, our_id, nsid, *from)
                }
                LiveEvent::SyncFinished(ev) => {
                    remember_peer(&dir, &peers, our_id, nsid, ev.peer)
                }
                _ => {}
            }
            // The insert events carry the entry, so in-process listeners can be
            // told exactly which note changed. Everything else (ContentReady,
            // sync/neighbor events) only identifies the vault.
            let path = match &ev {
                LiveEvent::InsertLocal { entry } | LiveEvent::InsertRemote { entry, .. } => {
                    std::str::from_utf8(entry.key()).ok().map(str::to_string)
                }
                _ => None,
            };
            // Errs only when nobody is subscribed — the common case.
            let _ = changes.send(VaultChange { vault: nsid, path });
            let _ = app.emit("vault-changed", &vault_id);
        }
    });
    Ok(())
}

/// Start emitting `vault-changed` events when a vault's document mutates.
#[tauri::command]
pub async fn watch_vault(
    app: AppHandle,
    state: State<'_, VaultManager>,
    vault: String,
) -> Result<(), String> {
    let node = state.node().await?;
    let nsid = parse_id(&vault).map_err(|e| e.to_string())?;
    arm_vault(&app, node, nsid).await
}

/// Background "live sync" hub mode: arm *every* vault (not just the currently-
/// open one) so this device carries all vaults for peers that are only
/// intermittently online. An always-on device (typically a desktop) thus becomes
/// the rendezvous through which intermittent peers converge without ever
/// overlapping each other.
///
/// Called by the `set_background_sync` command (see lib.rs) when the user enables
/// Background sync, and once on launch if it was left on. This is only the iroh
/// side — keeping the process alive in the background is the platform layer's job
/// (Android foreground service / desktop tray + autostart), driven by the same
/// toggle. Sync stops naturally when the process exits, so disabling needs no
/// teardown here: already-armed vaults keep running harmlessly until exit, and
/// nothing dials out once the process is gone.
pub async fn arm_all_vaults(app: &AppHandle, mgr: &VaultManager) -> Result<(), String> {
    let node = mgr.node().await?;
    let mut stream = node.docs.list().await.map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok((id, _cap)) = item {
            ids.push(id);
        }
    }
    for id in ids {
        let _ = arm_vault(app, node, id).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Poll a key until it appears (content synced + downloaded) or timeout.
    async fn await_key(
        node: &Node,
        doc: &iroh_docs::api::Doc,
        key: &[u8],
        timeout: Duration,
    ) -> Option<String> {
        let start = Instant::now();
        loop {
            if let Ok(Some(v)) = read_key(node, doc, key).await {
                return Some(v);
            }
            if start.elapsed() > timeout {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Two real iroh nodes: A shares a vault (write ticket), B joins, and edits
    /// propagate both directions (flat ownership). Mirrors share_vault/join_vault
    /// by round-tripping the ticket through a string.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn p2p_sync_roundtrip() {
        let base = std::env::temp_dir().join(format!("notes-p2p-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = init(base.join("a")).await.expect("node A");
        let b = init(base.join("b")).await.expect("node B");

        // A creates a vault + note, shares with write capability.
        let doc_a = a.docs.create().await.expect("create");
        doc_a
            .set_bytes(a.author, b"shared/hello.md".to_vec(), encode("from A"))
            .await
            .expect("A write");
        let ticket = doc_a
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .expect("share");

        // B joins from the (stringified) ticket.
        let ticket = DocTicket::from_str(&ticket.to_string()).expect("parse ticket");
        let doc_b = b.docs.import(ticket).await.expect("import");

        // A's note reaches B.
        let got = await_key(&b, &doc_b, b"shared/hello.md", Duration::from_secs(30)).await;
        assert_eq!(got.as_deref(), Some("from A"), "B did not receive A's note");

        // Flat ownership: B writes, A receives.
        doc_b
            .set_bytes(b.author, b"shared/reply.md".to_vec(), encode("from B"))
            .await
            .expect("B write");
        let got2 = await_key(&a, &doc_a, b"shared/reply.md", Duration::from_secs(30)).await;
        assert_eq!(got2.as_deref(), Some("from B"), "A did not receive B's note");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Concurrent edits to the SAME note on two peers must both survive — the
    /// whole-note last-writer-wins clobber was issue #99. A creates a note, B
    /// joins, then each appends a different line *before* the other's edit syncs;
    /// after convergence both lines are present on both sides.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_note_edits_merge() {
        let base_dir = std::env::temp_dir().join(format!("notes-merge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_dir);
        let a = init(base_dir.join("a")).await.expect("node A");
        let b = init(base_dir.join("b")).await.expect("node B");

        let doc_a = a.docs.create().await.expect("create");
        let key = b"shared.md";
        // Seed the note via the same path the app uses (CRDT-tagged value).
        doc_a.set_bytes(a.author, key.to_vec(), fresh_note("L1\n")).await.expect("seed");
        let ticket = doc_a
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .expect("share");
        let doc_b = b
            .docs
            .import(DocTicket::from_str(&ticket.to_string()).expect("ticket"))
            .await
            .expect("import");

        // B must receive the seed before editing, else there's no shared base.
        let mut seeded = None;
        for _ in 0..60 {
            if let Ok(Some(t)) = read_note_text(&b, &doc_b, key).await {
                seeded = Some(t);
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(seeded.as_deref(), Some("L1\n"), "B did not receive the seed note");

        // Each appends a distinct line from the same base, concurrently.
        write_note_merged(&a, &doc_a, key, "L1\n", "L1\nfrom-A\n").await.expect("A edit");
        write_note_merged(&b, &doc_b, key, "L1\n", "L1\nfrom-B\n").await.expect("B edit");

        // Both edits converge on both peers.
        for (node, doc, who) in [(&a, &doc_a, "A"), (&b, &doc_b, "B")] {
            let mut ok = false;
            for _ in 0..60 {
                if let Ok(Some(t)) = read_note_text(node, doc, key).await {
                    if t.contains("from-A") && t.contains("from-B") {
                        ok = true;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            assert!(ok, "{who} is missing one side's edit (clobber regression)");
        }

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    /// Legacy plain-text notes (tagged 0x01, pre-CRDT) still read, and merging two
    /// devices that independently seed the *same* legacy text does not duplicate
    /// it (deterministic content-derived seed client id).
    #[tokio::test]
    async fn legacy_text_reads_and_seeds_dedup() {
        let dir = std::env::temp_dir().join(format!("notes-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init");
        let doc = node.docs.create().await.expect("create");

        // A pre-CRDT note: plain text value (0x01).
        doc.set_bytes(node.author, b"old.md".to_vec(), encode("legacy body")).await.expect("set");
        assert_eq!(
            read_note_text(&node, &doc, b"old.md").await.expect("read").as_deref(),
            Some("legacy body"),
        );
        // Two independent seeds of identical text merge to one copy, not two.
        let ydoc = Doc::with_client_id(0);
        {
            let mut txn = ydoc.transact_mut();
            txn.apply_update(Update::decode_v1(&seed_update("hello")).unwrap()).unwrap();
            txn.apply_update(Update::decode_v1(&seed_update("hello")).unwrap()).unwrap();
        }
        assert_eq!(doc_text(&ydoc), "hello", "identical seeds duplicated text");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A raw (untagged) yrs update must be decoded as an update, not lossily
    /// decoded as text. A 1-client update begins with 0x01 (== MARKER), so the
    /// old prefix-only check ran `from_utf8_lossy` over the binary and baked
    /// U+FFFD into the note (#116). value_to_update now disambiguates by UTF-8
    /// validity and round-trips the update intact.
    #[test]
    fn untagged_yrs_update_decodes_without_mojibake() {
        let raw = seed_update("hello — world"); // em-dash: multibyte, exercises lossy path
        assert_eq!(raw.first(), Some(&MARKER), "precondition: 1-client update starts 0x01");

        let ydoc = Doc::with_client_id(0);
        {
            let mut txn = ydoc.transact_mut();
            let update = Update::decode_v1(&value_to_update(&raw)).expect("decode");
            txn.apply_update(update).expect("apply");
        }
        let text = doc_text(&ydoc);
        assert!(!text.contains('\u{FFFD}'), "lossy decode baked in replacement chars: {text:?}");
        assert_eq!(text, "hello — world");
    }

    /// Legacy 0x01 plain-text values still seed from their text (the common,
    /// valid-UTF-8 case must not regress into the raw-update branch).
    #[test]
    fn legacy_marker_text_still_seeds() {
        let v = encode("plain legacy — body");
        let ydoc = Doc::with_client_id(0);
        {
            let mut txn = ydoc.transact_mut();
            let update = Update::decode_v1(&value_to_update(&v)).expect("decode");
            txn.apply_update(update).expect("apply");
        }
        assert_eq!(doc_text(&ydoc), "plain legacy — body");
    }

    /// A note's content survives a rename (CRDT state copied to the new key),
    /// and the old key is gone. Guards the rename-copy path added for #99.
    #[tokio::test]
    async fn rename_preserves_note_content() {
        let dir = std::env::temp_dir().join(format!("notes-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init");
        let doc = node.docs.create().await.expect("create");

        doc.set_bytes(node.author, b"a.md".to_vec(), fresh_note("hello world")).await.expect("seed");
        // Mirror rename_path's single-file copy+tombstone.
        let (ydoc, found) = merged_note(&node, &doc, b"a.md", 0).await.expect("merge");
        assert!(found);
        doc.set_bytes(node.author, b"b.md".to_vec(), encode_doc(&ydoc)).await.expect("copy");
        doc.del(node.author, b"a.md".to_vec()).await.expect("del");

        assert_eq!(
            read_note_text(&node, &doc, b"b.md").await.expect("read").as_deref(),
            Some("hello world"),
        );
        assert_eq!(read_note_text(&node, &doc, b"a.md").await.expect("read old").as_deref(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The data path behind the `vault-changed` Tauri event: subscribing to a
    /// doc yields a LiveEvent when it mutates (watch_vault forwards these to
    /// `app.emit`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscribe_emits_on_change() {
        use futures_lite::StreamExt;
        let dir = std::env::temp_dir().join(format!("notes-sub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("node");
        let doc = node.docs.create().await.expect("create");

        let mut events = doc.subscribe().await.expect("subscribe");
        doc.set_bytes(node.author, b"x.md".to_vec(), encode("hi"))
            .await
            .expect("write");

        let got = tokio::time::timeout(Duration::from_secs(10), events.next()).await;
        assert!(
            matches!(got, Ok(Some(Ok(_)))),
            "expected a LiveEvent after a local change, got {got:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Headless relay peer for manual cross-network testing. Run with:
    ///   cargo test mac_relay_peer -- --ignored --nocapture
    /// Prints a TICKET; a phone (on cellular) joins it → forces the iroh relay
    /// path. Waits up to 3 min for the phone to write `phone-note.md`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore]
    async fn mac_relay_peer() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
        // Persistent dir → stable id + reuse the same vault across restarts, so
        // a restart doesn't orphan a phone that already joined.
        let dir = std::env::temp_dir().join("notes-mac-relay");
        let node = init(dir.clone()).await.expect("node");
        let doc = {
            let mut list = Box::pin(node.docs.list().await.expect("list"));
            let mut existing = None;
            while let Some(item) = list.next().await {
                if let Ok((id, _)) = item {
                    existing = node.docs.open(id).await.ok().flatten();
                    break;
                }
            }
            match existing {
                Some(d) => d,
                None => node.docs.create().await.expect("create"),
            }
        };
        doc.start_sync(Vec::new()).await.ok();
        doc.set_bytes(node.author, NAME_KEY.to_vec(), encode("MacRelay")).await.unwrap();
        doc.set_bytes(node.author, b"mac-note.md".to_vec(), encode("from mac")).await.unwrap();
        let ticket = doc
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .unwrap();
        println!("TICKET={ticket}");
        println!("VAULT={}", doc.id());
        // Long-running: report phone-note.md whenever it changes, so a network
        // handoff on the phone can be observed (does sync survive the switch?).
        let start = Instant::now();
        let mut last = String::new();
        while start.elapsed() < Duration::from_secs(900) {
            if let Ok(Some(v)) = read_key(&node, &doc, b"phone-note.md").await {
                if v != last {
                    println!("GOT_PHONE@{}s={v}", start.elapsed().as_secs());
                    last = v;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    #[tokio::test]
    async fn vault_roundtrip() {
        let dir = std::env::temp_dir().join(format!("notes-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");

        let doc = node.docs.create().await.expect("create doc");
        doc.set_bytes(node.author, NAME_KEY.to_vec(), encode("My Vault"))
            .await
            .expect("set name");
        assert_eq!(vault_meta_name(&node, &doc).await, Some("My Vault".to_string()));

        doc.set_bytes(node.author, b"a/b.md".to_vec(), encode("hello"))
            .await
            .expect("set note");
        let got = read_key(&node, &doc, b"a/b.md").await.expect("read").unwrap();
        assert_eq!(got, "hello");

        doc.set_bytes(node.author, b"empty.md".to_vec(), encode(""))
            .await
            .expect("set empty");
        let keys = list_keys(&doc).await.expect("list");
        assert!(keys.contains(&"a/b.md".to_string()));
        assert!(keys.contains(&"empty.md".to_string()));
        assert!(!keys.iter().any(|k| k.starts_with('\u{0}')), "meta key leaked");

        doc.del(node.author, b"a/b.md".to_vec()).await.expect("del");
        let keys2 = list_keys(&doc).await.expect("list2");
        assert!(!keys2.contains(&"a/b.md".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Deleting a folder recursively tombstones every entry under it — nested
    // notes and subfolder markers — while leaving sibling notes untouched (#121).
    #[tokio::test]
    async fn delete_folder_is_recursive() {
        let dir = std::env::temp_dir().join(format!("notes-rmdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");

        for k in ["proj/.keep", "proj/a.md", "proj/sub/b.md", "proj.md", "other.md"] {
            doc.set_bytes(node.author, k.as_bytes().to_vec(), encode("x"))
                .await
                .expect("set");
        }

        // Mirror delete_path(is_dir=true): tombstone every key under the prefix.
        let prefix = "proj/".to_string();
        let keys = list_keys(&doc).await.expect("list");
        for key in keys.iter().filter(|k| k.starts_with(&prefix)) {
            doc.del(node.author, key.clone().into_bytes()).await.expect("del");
        }
        doc.del(node.author, prefix.into_bytes()).await.expect("del prefix");

        let after = list_keys(&doc).await.expect("list2");
        assert!(!after.iter().any(|k| k.starts_with("proj/")), "folder contents survived: {after:?}");
        // A note named like the folder ("proj.md") and an unrelated sibling stay.
        assert!(after.contains(&"proj.md".to_string()), "prefix over-matched proj.md");
        assert!(after.contains(&"other.md".to_string()), "sibling deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tag extraction (#15) has to coexist with ordinary Markdown: the only
    /// thing separating `#tag` from an ATX heading is the space.
    #[test]
    fn extract_tags_handles_markdown_lookalikes() {
        assert_eq!(extract_tags("Buy milk #errands #urgent"), ["errands", "urgent"]);
        // A heading is not a tag, at any level.
        assert!(extract_tags("# Heading\n## Sub").is_empty());
        // Nor is a URL fragment, or a '#' mid-word.
        assert!(extract_tags("see https://example.com/#anchor").is_empty());
        assert!(extract_tags("wrote it in C#").is_empty());
        // Nested and hyphenated tags survive intact.
        assert_eq!(extract_tags("#in/progress #q3-goals"), ["in/progress", "q3-goals"]);
        // Trailing punctuation and separators are trimmed off.
        assert_eq!(extract_tags("done #work. next #home, then #a/"), ["work", "home", "a"]);
        // Start of line counts as a boundary; repeats collapse (per note).
        assert_eq!(extract_tags("#top\nmore #top #TOP"), ["top"]);
        // A bare '#' or '# ' yields nothing.
        assert!(extract_tags("# \n#\nnot # a tag").is_empty());
        // Non-ASCII must not panic or split a codepoint.
        assert_eq!(extract_tags("café #café #日本語"), ["café", "日本語"]);
    }

    /// Search reports 1-based line numbers, ignores case, honours the cap, and
    /// never surfaces trashed notes.
    #[tokio::test]
    async fn search_finds_matches_and_skips_trash() {
        let dir = std::env::temp_dir().join(format!("notes-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");
        let vault = doc.id().to_string();

        for (k, v) in [
            ("one.md", "alpha\nbeta\ngamma\n"),
            ("two.md", "nothing here\n"),
            ("notes/three.md", "BETA rising\n"),
            (".trash/old.md", "beta in the bin\n"),
        ] {
            doc.set_bytes(node.author, k.as_bytes().to_vec(), fresh_note(v))
                .await
                .expect("set");
        }

        let hits = search(&node, &vault, "beta", None, 20).await.expect("search");
        assert_eq!(hits.len(), 2, "case-insensitive across both live notes: {hits:?}",);
        let one = hits.iter().find(|h| h.path == "one.md").expect("one.md");
        assert_eq!(one.lines, ["2: beta"], "1-based line numbers");
        assert!(
            !hits.iter().any(|h| h.path.starts_with(".trash/")),
            "trashed notes must not surface in search"
        );

        // path_contains narrows to a folder, and max caps the result set.
        let scoped = search(&node, &vault, "beta", Some("notes/"), 20).await.expect("scoped");
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].path, "notes/three.md");
        assert_eq!(search(&node, &vault, "beta", None, 1).await.expect("cap").len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Tag counts are per-note (not per-occurrence), most-used first, and skip
    /// the trash.
    #[tokio::test]
    async fn tags_in_vault_counts_notes() {
        let dir = std::env::temp_dir().join(format!("notes-tags-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");
        let vault = doc.id().to_string();

        for (k, v) in [
            ("a.md", "# Heading\ntask #work #urgent\nmore #work\n"),
            ("b.md", "another #work item\n"),
            ("c.md", "untagged\n"),
            (".trash/d.md", "#work #ghost\n"),
        ] {
            doc.set_bytes(node.author, k.as_bytes().to_vec(), fresh_note(v))
                .await
                .expect("set");
        }

        let tags = tags_in_vault(&node, &vault).await.expect("tags");
        let pairs: Vec<(&str, usize)> = tags.iter().map(|t| (t.tag.as_str(), t.count)).collect();
        assert_eq!(
            pairs,
            [("work", 2), ("urgent", 1)],
            "counted per note, most-used first, trash excluded"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Renaming a folder must MOVE it, not copy it: the new prefix gets the
    /// contents and the old prefix disappears entirely. Reported symptom is a
    /// duplicate — both folders present, the new one holding the notes.
    #[tokio::test]
    async fn rename_folder_moves_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("notes-mvdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");

        for k in ["old/.keep", "old/a.md", "old/sub/b.md"] {
            doc.set_bytes(node.author, k.as_bytes().to_vec(), fresh_note("x"))
                .await
                .expect("set");
        }

        rename_key(&node, &doc, "old", "new", true).await.expect("rename");

        let after = list_keys(&doc).await.expect("list");
        assert!(
            after.contains(&"new/a.md".to_string()) && after.contains(&"new/sub/b.md".to_string()),
            "contents did not arrive at the new name: {after:?}"
        );
        assert!(
            !after.iter().any(|k| k.starts_with("old/")),
            "old folder survived the rename (duplicate): {after:?}"
        );
    }

    /// The real-world case: the folder holds a note written by ANOTHER device
    /// (a second author, as any synced peer produces). `del(author, prefix)`
    /// writes a single empty entry at `(our author, "old/")` — it does not
    /// tombstone `old/a.md` under the peer's author — so the peer's note stays
    /// live under the old prefix and the folder appears duplicated.
    #[tokio::test]
    async fn rename_folder_moves_peer_authored_notes_too() {
        let dir = std::env::temp_dir().join(format!("notes-mvdir2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");
        // A second author stands in for a peer device writing into the vault.
        let peer = node.docs.author_create().await.expect("peer author");

        doc.set_bytes(node.author, b"old/.keep".to_vec(), encode(""))
            .await
            .expect("keep");
        doc.set_bytes(peer, b"old/a.md".to_vec(), fresh_note("from the phone"))
            .await
            .expect("peer note");

        rename_key(&node, &doc, "old", "new", true).await.expect("rename");

        let after = list_keys(&doc).await.expect("list");
        assert!(
            after.contains(&"new/a.md".to_string()),
            "peer's note did not arrive at the new name: {after:?}"
        );
        assert!(
            !after.iter().any(|k| k.starts_with("old/")),
            "old folder survived — duplicate left behind: {after:?}"
        );
    }

    /// And the stale folder left behind must at least be deletable — the second
    /// half of the report ("deleting the old folder does nothing"). Uses a peer
    /// author, since that is the case that leaves a folder behind at all.
    #[tokio::test]
    async fn delete_folder_after_rename_removes_it() {
        let dir = std::env::temp_dir().join(format!("notes-mvrm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");
        let peer = node.docs.author_create().await.expect("peer author");

        doc.set_bytes(node.author, b"old/.keep".to_vec(), encode("")).await.expect("keep");
        doc.set_bytes(peer, b"old/a.md".to_vec(), fresh_note("x")).await.expect("peer note");

        rename_key(&node, &doc, "old", "new", true).await.expect("rename");
        delete_key(&node, &doc, "old", true).await.expect("delete");

        let after = list_keys(&doc).await.expect("list");
        assert!(
            !after.iter().any(|k| k.starts_with("old/")),
            "old folder survived an explicit delete: {after:?}"
        );
        assert!(after.contains(&"new/a.md".to_string()), "delete took the renamed copy too: {after:?}");
    }

    // A deleted note's name is free again: recreating it reuses the name rather
    // than appending " 1" against the tombstone (#48).
    #[tokio::test]
    async fn deleted_key_is_free() {
        let dir = std::env::temp_dir().join(format!("notes-ghost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("init node");
        let doc = node.docs.create().await.expect("create doc");

        doc.set_bytes(node.author, b"Backlog.md".to_vec(), encode("x"))
            .await
            .expect("set");
        assert!(key_exists(&doc, "Backlog.md").await.expect("exists"));

        doc.del(node.author, b"Backlog.md".to_vec()).await.expect("del");
        assert!(!key_exists(&doc, "Backlog.md").await.expect("exists2"));
        // The recreated note reuses the original name, no " 1" suffix.
        assert_eq!(free_key(&doc, "Backlog.md").await.expect("free"), "Backlog.md");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // The real ghost-file scenario is multi-author: A's own live entry for a key
    // coexists with a peer's newer tombstone. A raw key_exact (FlatQuery) can
    // surface A's stale live entry, so the name would collide; single_latest_per_key
    // picks the newest (the tombstone) and treats the name as free (#48).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deleted_key_free_across_peers() {
        let base = std::env::temp_dir().join(format!("notes-ghost-p2p-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = init(base.join("a")).await.expect("node A");
        let b = init(base.join("b")).await.expect("node B");

        let doc_a = a.docs.create().await.expect("create");
        doc_a
            .set_bytes(a.author, b"Backlog.md".to_vec(), encode("hi"))
            .await
            .expect("A write");
        let ticket = doc_a
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .expect("share");
        let doc_b = b
            .docs
            .import(DocTicket::from_str(&ticket.to_string()).expect("parse ticket"))
            .await
            .expect("import");

        // B receives A's note, then deletes it — B's tombstone is now newer than
        // A's still-present live entry.
        let got = await_key(&b, &doc_b, b"Backlog.md", Duration::from_secs(30)).await;
        assert_eq!(got.as_deref(), Some("hi"), "B did not receive A's note");
        doc_b.del(b.author, b"Backlog.md".to_vec()).await.expect("B del");

        // Once A sees the deletion, the name must read as free despite A's own
        // stale live entry for it.
        let mut freed = false;
        for _ in 0..60 {
            if !key_exists(&doc_a, "Backlog.md").await.expect("exists") {
                freed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(freed, "A never saw the deletion propagate");
        assert_eq!(free_key(&doc_a, "Backlog.md").await.expect("free"), "Backlog.md");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// #120: renaming a vault sets a LOCAL name only. Node A creates + names a
    /// vault, renames it locally, and name resolution reflects the new name on A
    /// — while Node B joining the SAME vault does NOT see A's rename (it sees the
    /// synced meta default). Also asserts the override round-trips through disk.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn rename_vault_is_local_only() {
        let base = std::env::temp_dir().join(format!("notes-rename-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let a = init(base.join("a")).await.expect("node A");

        // A creates a vault with a synced-meta default name.
        let doc_a = a.docs.create().await.expect("create");
        let nsid = doc_a.id();
        doc_a
            .set_bytes(a.author, NAME_KEY.to_vec(), encode("Shared Default"))
            .await
            .expect("meta");

        // Local rename: writes vault-names.json only.
        {
            let mut m = a.names.lock().unwrap();
            m.insert(nsid, "A's Custom Name".to_string());
            save_names(&a.dir, &m);
        }

        // Effective name on A = local override; hash is the 6-hex disambiguator.
        let meta = vault_meta_name(&a, &doc_a).await;
        let ov = a.names.lock().unwrap().get(&nsid).cloned();
        let info = vault_info(nsid, meta, ov);
        assert_eq!(info.name, "A's Custom Name");
        assert_eq!(info.hash.len(), 6);
        assert!(!info.pending);

        // Persistence round-trip: reload names from disk.
        let reloaded = load_names(&a.dir);
        assert_eq!(reloaded.get(&nsid).map(String::as_str), Some("A's Custom Name"));

        // B joins the shared vault; B must NOT see A's rename — only synced meta.
        let b = init(base.join("b")).await.expect("node B");
        let ticket = doc_a
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .expect("share");
        let doc_b = b
            .docs
            .import(DocTicket::from_str(&ticket.to_string()).expect("ticket"))
            .await
            .expect("import");
        let got = await_key(&b, &doc_b, NAME_KEY, Duration::from_secs(30)).await;
        assert_eq!(
            got.as_deref(),
            Some("Shared Default"),
            "synced meta default should reach B"
        );
        let b_ov = b.names.lock().unwrap().get(&doc_b.id()).cloned();
        assert_eq!(b_ov, None, "B must not have A's local override");
        let b_info = vault_info(doc_b.id(), got, b_ov);
        assert_eq!(
            b_info.name, "Shared Default",
            "B sees the meta default, not A's rename"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// #120 unit checks: short_hash is idempotent + fixed length, and load_names
    /// skips blank/whitespace overrides.
    #[tokio::test]
    async fn short_hash_and_load_names_edge_cases() {
        let base = std::env::temp_dir()
            .join(format!("notes-names-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("mkdir");

        // short_hash: deterministic, first 6 hex chars.
        let a = init(base.join("node")).await.expect("node");
        let id = a.docs.create().await.expect("create").id();
        assert_eq!(short_hash(&id), short_hash(&id));
        assert_eq!(short_hash(&id).len(), 6);
        assert_eq!(short_hash(&id), id.to_string()[..6]);

        // load_names skips blank entries but keeps trimmed real ones.
        let good = id.to_string();
        let json = format!("{{\"{good}\":\"  Trimmed Me  \",\"bad-id\":\"x\"}}");
        std::fs::write(names_path(&base), json).expect("write");
        let names = load_names(&base);
        assert_eq!(names.get(&id).map(String::as_str), Some("Trimmed Me"));
        assert_eq!(names.len(), 1, "unparseable id dropped");

        // A blank override is dropped entirely.
        let blank = format!("{{\"{good}\":\"   \"}}");
        std::fs::write(names_path(&base), blank).expect("write");
        assert!(load_names(&base).is_empty(), "blank override skipped");

        let _ = std::fs::remove_dir_all(&base);
    }
}
