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
use iroh_docs::{engine::DefaultAuthorStorage, engine::Engine, protocol::Docs};
use iroh_gossip::net::Gossip;
use vellum_vault::vault::{Node, SideStore};
use wasm_bindgen::prelude::*;

mod opfs;

/// Names for the two 32-byte keys that survive a reload. Without them every
/// reload is a new device to every peer, and every edit is by a new author.
const ENDPOINT_SECRET: &str = "endpoint-secret";
const AUTHOR_KEY: &str = "author";

thread_local! {
    static NODE: RefCell<Option<Rc<Node>>> = const { RefCell::new(None) };
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
    endpoint.online().await;

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

    let side: Arc<dyn SideStore> = Arc::new(OpfsSideStore { store: store.clone() });
    let node = Node::assemble(
        endpoint,
        (*blobs).clone(),
        Box::new(blobs),
        docs,
        gossip,
        side,
    )
    .await?;
    Ok(node)
}

#[wasm_bindgen(start)]
pub fn setup() {
    console_error_panic_hook::set_once();
}
