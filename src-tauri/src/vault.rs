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

use anyhow::{anyhow, Result};
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint, SecretKey};
use iroh_blobs::{store::fs::FsStore, BlobsProtocol};
use iroh_docs::{
    api::protocol::{AddrInfoOptions, ShareMode},
    protocol::Docs,
    store::Query,
    AuthorId, DocTicket, NamespaceId,
};
use iroh_gossip::net::Gossip;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::OnceCell;

const NAME_KEY: &[u8] = b"\x00meta/name";
const MARKER: u8 = 0x01;
const KEEP: &str = ".keep";

/// The live iroh node (handles into the running protocols).
pub struct Node {
    blobs: FsStore,
    docs: Docs,
    author: AuthorId,
    _router: Router,
    watched: Mutex<HashSet<NamespaceId>>,
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
            std::fs::write(&key_path, sk.to_bytes())?;
            sk
        }
    };
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    let blobs = FsStore::load(dir.join("blobs")).await?;
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let docs = Docs::persistent(dir.clone())
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;

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
        watched: Mutex::new(HashSet::new()),
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
    let Some(entry) = doc.get_one(Query::key_exact(key)).await? else {
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

async fn vault_name(node: &Node, doc: &iroh_docs::api::Doc) -> String {
    match read_key(node, doc, NAME_KEY).await {
        Ok(Some(name)) if !name.is_empty() => name,
        _ => {
            let s = doc.id().to_string();
            format!("vault-{}", &s[..s.len().min(6)])
        }
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
                let name = vault_name(node, &doc).await;
                out.push(VaultInfo {
                    id: id.to_string(),
                    name,
                });
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
            Ok(VaultInfo {
                id: doc.id().to_string(),
                name,
            })
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
            let doc = node.docs.import(ticket).await?;
            let name = vault_name(node, &doc).await;
            Ok(VaultInfo {
                id: doc.id().to_string(),
                name,
            })
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
            let ticket = doc
                .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
                .await?;
            Ok(ticket.to_string())
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
            doc.set_bytes(node.author, path.into_bytes(), encode(&content)).await?;
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_note(state: State<'_, VaultManager>, vault: String, path: String) -> Result<(), String> {
    let node = state.node().await?;
    map_err(
        async {
            let doc = open(node, &vault).await?;
            doc.set_bytes(node.author, path.into_bytes(), encode("")).await?;
            Ok(())
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

/// Start emitting `vault-changed` events when a vault's document mutates.
#[tauri::command]
pub async fn watch_vault(
    app: AppHandle,
    state: State<'_, VaultManager>,
    vault: String,
) -> Result<(), String> {
    let node = state.node().await?;
    let nsid = parse_id(&vault).map_err(|e| e.to_string())?;
    {
        let mut watched = node.watched.lock().unwrap();
        if !watched.insert(nsid) {
            return Ok(()); // already watching
        }
    }
    let doc = open(node, &vault).await.map_err(|e| e.to_string())?;
    // Resume live sync. We only call start_sync on join, so after an app
    // restart a previously-joined vault wouldn't sync. iroh-docs persists per-
    // namespace sync peers, so start_sync with no explicit peers reconnects to
    // the peers we synced with before.
    let _ = doc.start_sync(Vec::new()).await;
    let mut stream = doc.subscribe().await.map_err(|e| e.to_string())?;
    let vault_id = vault.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = stream.next().await {
            if event.is_ok() {
                let _ = app.emit("vault-changed", &vault_id);
            }
        }
    });
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
        let dir = std::env::temp_dir().join("notes-mac-relay");
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("node");
        let doc = node.docs.create().await.expect("create");
        doc.set_bytes(node.author, NAME_KEY.to_vec(), encode("MacRelay")).await.unwrap();
        doc.set_bytes(node.author, b"mac-note.md".to_vec(), encode("from mac")).await.unwrap();
        let ticket = doc
            .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
            .await
            .unwrap();
        println!("TICKET={ticket}");
        println!("VAULT={}", doc.id());
        let start = Instant::now();
        loop {
            if let Ok(Some(v)) = read_key(&node, &doc, b"phone-note.md").await {
                println!("GOT_PHONE={v}");
                break;
            }
            if start.elapsed() > Duration::from_secs(180) {
                println!("TIMEOUT_NO_PHONE_NOTE");
                break;
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
        assert_eq!(vault_name(&node, &doc).await, "My Vault");

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
}
