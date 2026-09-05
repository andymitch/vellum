//! Browser-side vault node, driven from JS — the sync path that decides whether
//! a WASM backend for Vellum is viable at all (spike for #221/#222).
//!
//! `start` boots an iroh endpoint with an in-memory blob store and an in-memory
//! iroh-docs replica (redb's Vec<u8> backend), then either creates a vault or
//! joins one from a ticket. `write` and `dump` are the two operations the test
//! needs: a write must reach the other peer, and `dump` reads content back out
//! of the blob store — so a passing run proves the replica *and* the note
//! content synced, not merely that a connection opened.
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint, SecretKey};
use iroh_blobs::{store::mem::MemStore, BlobsProtocol};
use iroh_docs::{
    actor::SyncHandle,
    api::protocol::{AddrInfoOptions, ShareMode},
    engine::{DefaultAuthorStorage, Engine},
    protocol::Docs,
    store::Query,
    Author, AuthorId, DocTicket,
};
use iroh_gossip::net::Gossip;
use wasm_bindgen::prelude::*;

mod bridge;
mod opfs;

/// Names under which the two 32-byte keys that make this device *this* device
/// are remembered in the vault store.
const ENDPOINT_SECRET: &str = "endpoint-secret";
const AUTHOR_KEY: &str = "author";

struct Node {
    blobs: MemStore,
    author: AuthorId,
    doc: iroh_docs::api::Doc,
    /// Present when the vault is persistent: everything durable that isn't the
    /// replica — note content, and this device's identity.
    store: Option<opfs::VaultStore>,
    /// Present when the vault is persistent: lets `flush` force the replica's
    /// redb transaction to commit, rather than waiting out the actor's batch
    /// window. See `flush` for why a browser needs that and a desktop doesn't.
    sync: Option<SyncHandle>,
    _router: Router,
    _endpoint: Endpoint,
}

thread_local! {
    static NODE: RefCell<Option<Rc<Node>>> = const { RefCell::new(None) };
}

fn node() -> Result<Rc<Node>> {
    NODE.with(|n| n.borrow().clone()).ok_or_else(|| anyhow!("start() first"))
}

/// Where the replica lives. `Memory` is what the two sync tests use; `Opfs` is
/// the persistence route from PERSISTENCE.md — redb on an OPFS sync access
/// handle, which needs `Store::with_backend` from the local iroh-docs patch and
/// only works inside a worker.
pub enum Storage {
    Memory,
    Opfs { file: String },
}

async fn boot_with(storage: Storage, ticket: Option<String>) -> Result<String> {
    // Before anything else, because this device's identity lives in it and the
    // endpoint needs its secret key at bind time.
    let store = match &storage {
        Storage::Memory => None,
        Storage::Opfs { file } => Some(
            opfs::VaultStore::open(&format!("{file}.store"))
                .await
                .map_err(|e| anyhow!("opening the vault store on OPFS: {e:?}"))?,
        ),
    };
    // Reuse the endpoint secret if we have one. Minting a fresh one each boot
    // is what makes a reloaded tab look like a brand new device to every peer.
    let secret = match &store {
        Some(store) => match store.get_key(ENDPOINT_SECRET)? {
            Some(bytes) => SecretKey::from_bytes(&bytes),
            None => {
                let secret = SecretKey::generate();
                store.put_key(ENDPOINT_SECRET, &secret.to_bytes())?;
                secret
            }
        },
        None => SecretKey::generate(),
    };
    let endpoint = Endpoint::builder(presets::N0).secret_key(secret).bind().await?;
    // A browser has no direct addresses, so a ticket is only reachable once the
    // home relay is up. Minting one before that produces a ticket nobody can
    // dial — which is exactly what silently failed on the first run of this
    // spike: the join was accepted and then nothing ever synced, with no error.
    endpoint.online().await;
    let blobs = MemStore::new();
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let mut sync = None;
    let docs = match storage {
        Storage::Memory => {
            Docs::memory()
                .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
                .await?
        }
        // The builder only offers memory or a file path, so build the engine
        // directly. It already takes a `Store`, which is why the patch adds
        // nothing but a constructor.
        Storage::Opfs { ref file } => {
            let backend = opfs::OpfsBackend::open(file)
                .await
                .map_err(|e| anyhow!("opening {file} on OPFS: {e:?}"))?;
            let store = iroh_docs::store::fs::Store::with_backend(backend)?;
            let downloader = blobs.downloader(&endpoint);
            let engine = Engine::spawn(
                endpoint.clone(),
                gossip.clone(),
                store,
                (*blobs).clone(),
                downloader,
                // The only variant a browser can use: `Persistent` is a file
                // path, and there is no "use this author" variant to hand our
                // remembered key to. So this mints a throwaway author into the
                // (persisted) author table on every boot, which the identity
                // block below has to set aside and delete. Worth raising with
                // n0 alongside the storage-backend patch.
                DefaultAuthorStorage::Mem,
                None,
            )
            .await?;
            sync = Some(engine.sync.clone());
            Docs::new(engine)
        }
    };
    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();
    // Same story as the endpoint secret: without a remembered author key every
    // reload authors its edits as a different person.
    let author = match &store {
        Some(vault) => {
            let throwaway = docs.author_default().await?;
            let id = match vault.get_key(AUTHOR_KEY)? {
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
                    vault.put_key(AUTHOR_KEY, &author.to_bytes())?;
                    id
                }
            };
            // Ours becomes the default so the throwaway can go: deleting the
            // default is refused, and leaving it would grow the author table by
            // one key per launch, forever.
            docs.author_set_default(id).await?;
            docs.author_delete(throwaway).await?;
            id
        }
        None => docs.author_create().await?,
    };
    // With a persistent replica, a second run has to reopen the vault already in
    // the store instead of creating an empty one — that reopening is the whole
    // point of the persistence test.
    let existing = {
        let mut listed = Box::pin(docs.list().await?);
        let mut first = None;
        if let Some(entry) = listed.next().await {
            first = Some(entry?.0);
        }
        first
    };
    let doc = match (&ticket, existing) {
        (Some(t), _) => docs.import(t.parse::<DocTicket>()?).await?,
        (None, Some(namespace)) => docs
            .open(namespace)
            .await?
            .ok_or_else(|| anyhow!("replica {namespace} vanished between list and open"))?,
        (None, None) => docs.create().await?,
    };

    // Rehydrate exactly the content the reopened replica points at, then drop
    // whatever it no longer does — superseded versions of an edited note, or a
    // deleted note's content. Reading the hashes off the entries is what keeps
    // boot proportional to the vault rather than to its history.
    let mut restored = 0usize;
    let mut gc = (0usize, 0usize);
    if let Some(store) = &store {
        let mut live: Vec<Vec<u8>> = Vec::new();
        let mut entries = Box::pin(doc.get_many(Query::single_latest_per_key()).await?);
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let hash = entry.content_hash();
            live.push(hash.as_bytes().to_vec());
            if let Some(bytes) = store.get_blob(hash.as_bytes())? {
                blobs.add_bytes(bytes).await?;
                restored += 1;
            }
        }
        gc = store.retain_blobs(&live)?;
    }
    // Reported so the test can see the throwaway cleanup holding: a count that
    // climbs with each session means the author table is growing per launch.
    let authors = {
        let mut listed = Box::pin(docs.author_list().await?);
        let mut n = 0usize;
        while let Some(author) = listed.next().await {
            author?;
            n += 1;
        }
        n
    };
    let share = doc.share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses).await?;
    let id = endpoint.id().fmt_short();
    let addr = format!("{:?}", endpoint.addr());
    NODE.with(|n| {
        *n.borrow_mut() = Some(Rc::new(Node {
            blobs,
            author,
            doc,
            store,
            sync,
            _router: router,
            _endpoint: endpoint,
        }))
    });
    Ok(format!(
        "{{\"endpoint\":\"{id}\",\"addr\":{addr:?},\"ticket\":\"{share}\",\
         \"restored\":{restored},\"gcDropped\":{},\"blobsStored\":{},\"authors\":{authors}}}",
        gc.0, gc.1
    ))
}

/// In-memory vault: what the two sync tests use.
#[wasm_bindgen]
pub async fn start(ticket: Option<String>) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    boot_with(Storage::Memory, ticket).await.map_err(err)
}

/// Vault whose replica lives in `file` on OPFS. Worker only.
#[wasm_bindgen]
pub async fn start_persistent(file: String, ticket: Option<String>) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    boot_with(Storage::Opfs { file }, ticket).await.map_err(err)
}

#[wasm_bindgen]
pub async fn write(key: String, value: String) -> Result<(), JsValue> {
    let n = node().map_err(err)?;
    let bytes = value.into_bytes();
    // Content first, then the entry that points at it: the reverse order can
    // leave an entry whose content is missing until a peer supplies it, while
    // this way the worst case is an unreferenced blob, which the GC on next
    // boot sweeps up. The hash is BLAKE3 of the bytes, the same hash iroh-blobs
    // will derive, so the key matches what the entry ends up referencing.
    if let Some(store) = &n.store {
        let hash = iroh_blobs::Hash::new(&bytes);
        store.put_blob(hash.as_bytes(), &bytes).map_err(err)?;
    }
    n.doc
        .set_bytes(n.author, key.into_bytes(), bytes)
        .await
        .map_err(|e| err(anyhow!("{e:?}")))?;
    Ok(())
}

/// Commit the replica's pending redb transaction now.
///
/// The docs actor batches entries into one transaction and commits after 500 ms
/// of idle (`MAX_COMMIT_DELAY`), or on a graceful shutdown. On desktop that is
/// free: the process closes the engine on its way out. A browser tab has no such
/// guarantee — it can be closed, backgrounded and discarded, or killed by the OS
/// with no chance to run async cleanup — so a write that returned successfully
/// can still be sitting in an uncommitted transaction when the tab disappears.
/// This spike lost an edit to exactly that.
///
/// So durability in the browser has to be asked for at the save boundary rather
/// than inherited from a clean shutdown. The content store needs no equivalent:
/// `BlobStore::put` commits per write already.
#[wasm_bindgen]
pub async fn flush() -> Result<(), JsValue> {
    let n = node().map_err(err)?;
    if let Some(sync) = &n.sync {
        sync.flush_store().await.map_err(|e| err(anyhow!("{e:?}")))?;
    }
    Ok(())
}

/// How many blobs the durable store holds — lets the test see dedup and GC.
#[wasm_bindgen]
pub fn blobs_stored() -> Result<usize, JsValue> {
    let n = node().map_err(err)?;
    match &n.store {
        Some(store) => store.blobs_len().map_err(err),
        None => Ok(0),
    }
}

/// Every entry with its content, read back through the blob store.
#[wasm_bindgen]
pub async fn dump() -> Result<String, JsValue> {
    let n = node().map_err(err)?;
    let mut out = Vec::new();
    let mut entries = Box::pin(
        n.doc
            .get_many(Query::single_latest_per_key())
            .await
            .map_err(|e| err(anyhow!("{e:?}")))?,
    );
    while let Some(entry) = entries.next().await {
        let entry = entry.map_err(|e| err(anyhow!("{e:?}")))?;
        let key = String::from_utf8_lossy(entry.key()).to_string();
        let value = match n.blobs.get_bytes(entry.content_hash()).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            // Entry synced but its content blob hasn't landed yet.
            Err(_) => "<pending>".to_string(),
        };
        out.push(format!("{{\"key\":{key:?},\"value\":{value:?}}}"));
    }
    Ok(format!("[{}]", out.join(",")))
}

fn err(e: anyhow::Error) -> JsValue {
    JsValue::from_str(&format!("{e:?}"))
}
