//! The browser shell.
//!
//! Everything the desktop app's `init` does with a filesystem, this does with
//! OPFS: the iroh-docs replica on a redb `StorageBackend` over a synchronous
//! access handle, note content in a hash-keyed store beside it, and the two
//! 32-byte keys that make this device *this* device in the same place. What it
//! hands back is the same [`vellum_vault::vault::Node`] the desktop builds, so
//! every rule above it — the tree, search, tags, the yrs merge, export/import —
//! is the one code path, not a second implementation of the same behaviour.
//!
//! Worker only. OPFS sync access handles do not exist on the main thread, and
//! iroh's browser build is single-threaded, so CRDT merges and blob hashing
//! would jank the editor.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use iroh::{endpoint::presets, Endpoint, SecretKey};
use iroh_blobs::store::mem::MemStore;
use iroh_docs::{engine::DefaultAuthorStorage, engine::Engine, protocol::Docs, Author};
use iroh_gossip::net::Gossip;
use vellum_vault::vault::{Node, SideStore};
use wasm_bindgen::prelude::*;

mod bridge;
mod opfs;

/// Names for the two 32-byte keys that survive a reload. Without them every
/// reload is a new device to every peer, and every edit is by a new author.
const ENDPOINT_SECRET: &str = "endpoint-secret";
const AUTHOR_KEY: &str = "author";

thread_local! {
    static NODE: RefCell<Option<Rc<Node>>> = const { RefCell::new(None) };
    /// The durable content store, kept beside the node it belongs to.
    static STORE: RefCell<Option<Arc<opfs::VaultStore>>> = const { RefCell::new(None) };
}

pub(crate) fn node() -> Option<Rc<Node>> {
    NODE.with(|n| n.borrow().clone())
}

pub(crate) fn set_node(n: Node) {
    NODE.with(|slot| *slot.borrow_mut() = Some(Rc::new(n)));
}

/// Start syncing every vault, and forward the node's change channel to the page.
///
/// The desktop bridges the same channel to a Tauri event. Subscribing *before*
/// arming matters: `arm` publishes an initial nudge per vault, and a broadcast
/// channel drops anything sent with no receiver — the desktop path documents
/// and obeys the same order.
///
/// `spawn_local` rather than a `Send` spawn, because this holds an `Rc<Node>`;
/// a browser wasm build is single-threaded, which is what makes that sound.
pub(crate) async fn start_syncing() -> Result<()> {
    let Some(node) = node() else {
        return Ok(());
    };
    let mut changes = node.subscribe_changes();
    let synced = node.clone();
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            match changes.recv().await {
                Ok(change) => {
                    // Content that arrives from a peer lands in MemStore, which
                    // dies with the tab. Copying it here is what makes a synced
                    // note readable after a reload rather than reading as empty
                    // — and an empty read is worse than a missing one, because
                    // the next autosave would write that emptiness back.
                    let vault = change.vault.to_string();
                    if let Err(e) = persist_content(&synced, &vault).await {
                        tracing::warn!(?e, "could not persist synced content");
                    }
                    bridge::publish_change(&vault);
                }
                // A dropped event still means something changed, and the page
                // re-reads on any event.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    vellum_vault::vault::arm_all(&node).await
}

/// Copy any note content the replica references but the durable store lacks.
///
/// Blobs live in `MemStore`, which does not survive a reload, so the bytes have
/// to be written somewhere durable or a reopened vault lists notes it cannot
/// read. This is the honest-but-blunt version: it walks the vault's live entries
/// after each mutation and stores whatever is missing, which is O(notes) per
/// save. The right fix is a real `iroh-blobs` store over OPFS, which is a large
/// piece of work — that crate has no store trait, so it means implementing the
/// whole actor protocol (partial blobs, bao trees, range requests, tags).
pub(crate) async fn persist_content(node: &Node, vault: &str) -> Result<()> {
    let Some(store) = STORE.with(|s| s.borrow().clone()) else {
        return Ok(());
    };
    let doc = vellum_vault::vault::open(node, vault).await?;
    for hash in vellum_vault::vault::live_content_hashes(&doc).await? {
        if store.has_blob(hash.as_bytes())? {
            continue;
        }
        if let Ok(bytes) = node.blobs().get_bytes(hash).await {
            store.put_blob(hash.as_bytes(), &bytes)?;
        }
    }
    Ok(())
}

/// The browser's [`SideStore`]: the peer cache and local vault names, in the
/// `meta` table of the store that sits beside the replica. The desktop writes
/// these as two JSON files; neither syncs, so each shell keeps its own.
struct OpfsSideStore {
    store: Arc<opfs::VaultStore>,
}

// Sound for the same reason the redb backend is: wasm32-unknown-unknown in a
// browser is single-threaded, so there is no second thread to observe this.
unsafe impl Send for OpfsSideStore {}
unsafe impl Sync for OpfsSideStore {}

impl SideStore for OpfsSideStore {
    fn read(&self, name: &str) -> Option<String> {
        self.store.get_text(name).ok().flatten()
    }

    fn write(&self, name: &str, contents: &str) {
        let _ = self.store.put_text(name, contents);
    }
}

/// Build the node for `file`, a vault database in the origin's private file
/// system. The desktop equivalent is `vault::init`.
pub async fn boot(file: &str) -> Result<Node> {
    let store = Arc::new(
        opfs::VaultStore::open(&format!("{file}.store"))
            .await
            .map_err(|e| anyhow!("opening the vault store on OPFS: {e:?}"))?,
    );

    // Reuse this device's identity, exactly as the desktop reuses
    // `endpoint-secret` from its data dir: a fresh key each launch means
    // persisted peers reference a node id that no longer exists.
    let secret = match store.get_key(ENDPOINT_SECRET)? {
        Some(bytes) => SecretKey::from_bytes(&bytes),
        None => {
            let secret = SecretKey::generate();
            store.put_key(ENDPOINT_SECRET, &secret.to_bytes())?;
            secret
        }
    };
    // No mDNS: a browser cannot do it, so discovery is relay-only.
    let endpoint = Endpoint::builder(presets::N0).secret_key(secret).bind().await?;
    // A browser endpoint has no direct addresses, so a share ticket minted
    // before the home relay is up is undialable — the join is accepted and then
    // nothing ever syncs, with no error anywhere.
    //
    // Bounded, because offline it never resolves: this app is local-first, and
    // waiting forever for a relay would mean a plane or a tunnel hangs `boot`
    // and every command behind it, leaving the sidebar insisting there are no
    // vaults. Timing out costs only a ticket minted before the relay is up,
    // which is recoverable — the vault opens, edits are local, and sync starts
    // when the network does.
    if n0_future::time::timeout(std::time::Duration::from_secs(10), endpoint.online())
        .await
        .is_err()
    {
        tracing::warn!("no relay yet; opening the vault offline");
    }

    let blobs = MemStore::new();
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let backend = opfs::OpfsBackend::open(file)
        .await
        .map_err(|e| anyhow!("opening {file} on OPFS: {e:?}"))?;
    let engine = Engine::spawn(
        endpoint.clone(),
        gossip.clone(),
        iroh_docs::store::fs::Store::with_backend(backend)?,
        (*blobs).clone(),
        blobs.downloader(&endpoint),
        DefaultAuthorStorage::Mem,
        None,
    )
    .await?;
    let docs = Docs::new(engine);

    // Keep authorship stable across reloads. `DefaultAuthorStorage` offers only
    // `Mem` and `Persistent(PathBuf)` — no way to hand it an author we already
    // have — so it mints a throwaway on every boot and writes it to the
    // (persisted) author table. Adopt our remembered key as the default, then
    // delete the throwaway, or the table grows by one key per launch forever.
    let throwaway = docs.author_default().await?;
    let ours = match store.get_key(AUTHOR_KEY)? {
        Some(bytes) => {
            let author = Author::from_bytes(&bytes);
            let id = author.id();
            docs.author_import(author).await?;
            id
        }
        None => {
            let id = docs.author_create().await?;
            let author = docs
                .author_export(id)
                .await?
                .ok_or_else(|| anyhow!("author {id} vanished right after creating it"))?;
            store.put_key(AUTHOR_KEY, &author.to_bytes())?;
            id
        }
    };
    docs.author_set_default(ours).await?;
    if throwaway != ours {
        // Deleting the default is refused, which is why the swap comes first.
        docs.author_delete(throwaway).await?;
    }

    let side: Arc<dyn SideStore> = Arc::new(OpfsSideStore { store: store.clone() });
    STORE.with(|s| *s.borrow_mut() = Some(store.clone()));
    let node = Node::assemble(
        endpoint,
        (*blobs).clone(),
        Box::new(blobs),
        docs,
        gossip,
        side,
    )
    .await?;

    // Reload note content before anything reads it, so entries resolve rather
    // than reading as pending. Content is addressed by hash, so re-adding the
    // bytes reproduces exactly the hashes the entries already reference. Then
    // drop whatever no live entry points at — a superseded version of an edited
    // note, or a deleted note's body — which is what keeps boot proportional to
    // the vault rather than to its whole history.
    let mut live: Vec<Vec<u8>> = Vec::new();
    for id in vaults(&node).await? {
        let doc = vellum_vault::vault::open(&node, &id).await?;
        for hash in vellum_vault::vault::live_content_hashes(&doc).await? {
            live.push(hash.as_bytes().to_vec());
            if let Some(bytes) = store.get_blob(hash.as_bytes())? {
                node.blobs().add_bytes(bytes).await?;
            }
        }
    }
    store.retain_blobs(&live)?;

    Ok(node)
}


/// Every vault id in the replica store.
async fn vaults(node: &Node) -> Result<Vec<String>> {
    use futures_lite::StreamExt;
    let mut stream = node.docs().list().await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok((id, _cap)) = item {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

#[wasm_bindgen(start)]
pub fn setup() {
    console_error_panic_hook::set_once();
}
