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

const NAME_KEY: &[u8] = b"\x00meta/name";
const MARKER: u8 = 0x01;
const KEEP: &str = ".keep";
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

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
    async fn node(&self) -> Result<&Node, String> {
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
    id: String,
    name: String,
    // True when the vault has no synced meta yet — i.e. it was joined from a
    // ticket but no peer has come online to sync its contents. The UI shows a
    // "waiting for a peer" state instead of a misleading generated vault (#4).
    pending: bool,
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

// Android only: a clone of the endpoint, so the JNI network-change hook (fired
// from Kotlin's ConnectivityManager callback) can notify iroh. Android doesn't
// surface network changes to native code, so iroh relies on us telling it.
#[cfg(target_os = "android")]
struct NetHandle {
    endpoint: Endpoint,
    docs: Docs,
    watched: std::sync::Arc<Mutex<HashSet<NamespaceId>>>,
    peers: std::sync::Arc<Mutex<PeerMap>>,
}
#[cfg(target_os = "android")]
static NET: std::sync::OnceLock<NetHandle> = std::sync::OnceLock::new();

// Re-probe the endpoint and re-dial every watched vault's peers. Shared by the
// network-change and app-resume hooks: both leave iroh with stale sockets/paths
// it can't detect on Android, and the recovery action is identical.
#[cfg(target_os = "android")]
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
    #[cfg(target_os = "android")]
    let _ = NET.set(NetHandle {
        endpoint: endpoint.clone(),
        docs: docs.clone(),
        watched: watched.clone(),
        peers: peers.clone(),
    });

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
    })
}

fn parse_id(id: &str) -> Result<NamespaceId> {
    NamespaceId::from_str(id).map_err(|e| anyhow!("bad vault id: {e}"))
}

async fn open(node: &Node, id: &str) -> Result<iroh_docs::api::Doc> {
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

/// Placeholder name for a vault whose real name hasn't synced yet.
fn fallback_name(id: &iroh_docs::NamespaceId) -> String {
    let s = id.to_string();
    format!("vault-{}", &s[..s.len().min(6)])
}

/// Build a `VaultInfo`, marking it `pending` (and using a placeholder name)
/// when the vault's meta name hasn't synced from a peer yet.
fn vault_info(id: iroh_docs::NamespaceId, meta_name: Option<String>) -> VaultInfo {
    VaultInfo {
        id: id.to_string(),
        pending: meta_name.is_none(),
        name: meta_name.unwrap_or_else(|| fallback_name(&id)),
    }
}

/// Collect all visible note keys (paths) in a vault.
async fn list_keys(doc: &iroh_docs::api::Doc) -> Result<Vec<String>> {
    let mut stream = Box::pin(doc.get_many(Query::single_latest_per_key()).await?);
    let mut keys = Vec::new();
    while let Some(entry) = stream.next().await {
        let entry = entry?;
        let key = entry.key();
        if key.first() == Some(&0x00) {
            continue; // reserved meta keys
        }
        if let Ok(s) = std::str::from_utf8(key) {
            keys.push(s.to_string());
        }
    }
    Ok(keys)
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
async fn free_key(doc: &iroh_docs::api::Doc, path: &str) -> Result<String> {
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

#[tauri::command]
pub async fn list_vaults(state: State<'_, VaultManager>) -> Result<Vec<VaultInfo>, String> {
    let node = state.node().await?;
    map_err(
        async {
            let mut stream = node.docs.list().await?;
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                let (id, _cap) = item?;
                let doc = node.docs.open(id).await?.ok_or_else(|| anyhow!("open failed"))?;
                let meta_name = vault_meta_name(node, &doc).await;
                out.push(vault_info(id, meta_name));
            }
            Ok(out)
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_vault(state: State<'_, VaultManager>, name: String) -> Result<VaultInfo, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = node.docs.create().await?;
            doc.set_bytes(node.author, NAME_KEY.to_vec(), encode(&name)).await?;
            // A vault we create has its name immediately, so it's never pending.
            Ok(vault_info(doc.id(), Some(name)))
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
            Ok(vault_info(doc.id(), meta_name))
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

#[tauri::command]
pub async fn list_tree(state: State<'_, VaultManager>, vault: String) -> Result<Vec<TreeNode>, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            let keys = list_keys(&doc).await?;
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
        .await,
    )
}

#[tauri::command]
pub async fn read_note(state: State<'_, VaultManager>, vault: String, path: String) -> Result<String, String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            Ok(read_key(node, &doc, path.as_bytes()).await?.unwrap_or_default())
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
                let content = read_key(node, &doc, key.as_bytes()).await?.unwrap_or_default();
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
                        doc.set_bytes(node.author, free.into_bytes(), encode(&c)).await?;
                        count += 1;
                    }
                }
            }
            Ok(count)
        }
        .await,
    )
}

#[tauri::command]
pub async fn write_note(
    state: State<'_, VaultManager>,
    vault: String,
    path: String,
    content: String,
) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            doc.set_bytes(node.author, path.clone().into_bytes(), encode(&content)).await?;
            Ok(())
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
            doc.set_bytes(node.author, free.clone().into_bytes(), encode("")).await?;
            Ok(free)
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_folder(state: State<'_, VaultManager>, vault: String, path: String) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            let key = format!("{}/{}", path.trim_end_matches('/'), KEEP);
            doc.set_bytes(node.author, key.into_bytes(), encode("")).await?;
            Ok(())
        }
        .await,
    )
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
            if is_dir {
                let from_prefix = format!("{}/", from.trim_end_matches('/'));
                let to_prefix = format!("{}/", to.trim_end_matches('/'));
                let keys = list_keys(&doc).await?;
                for key in keys.iter().filter(|k| k.starts_with(&from_prefix)) {
                    let new_key = format!("{}{}", to_prefix, &key[from_prefix.len()..]);
                    if let Some(content) = read_key(node, &doc, key.as_bytes()).await? {
                        doc.set_bytes(node.author, new_key.into_bytes(), encode(&content)).await?;
                    }
                }
                doc.del(node.author, from_prefix.into_bytes()).await?;
            } else {
                let content = read_key(node, &doc, from.as_bytes()).await?.unwrap_or_default();
                doc.set_bytes(node.author, to.into_bytes(), encode(&content)).await?;
                doc.del(node.author, from.into_bytes()).await?;
            }
            Ok(())
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
            let prefix = if is_dir {
                format!("{}/", path.trim_end_matches('/'))
            } else {
                path
            };
            doc.del(node.author, prefix.into_bytes()).await?;
            Ok(())
        }
        .await,
    )
}

/// Begin live-syncing a vault and emitting `vault-changed` on every mutation:
/// dial its known peers, subscribe to the doc, and spawn a task that re-emits
/// changes and learns new peers. Idempotent — a vault already armed is a no-op,
/// so it's safe to call from both `watch_vault` (the open vault) and
/// `set_live_sync` (every vault).
async fn arm_vault(app: &AppHandle, node: &Node, nsid: NamespaceId) -> Result<(), String> {
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
}
