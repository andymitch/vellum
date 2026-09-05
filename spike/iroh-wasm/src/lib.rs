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
    api::protocol::{AddrInfoOptions, ShareMode},
    engine::{DefaultAuthorStorage, Engine},
    protocol::Docs,
    store::Query,
    AuthorId, DocTicket,
};
use iroh_gossip::net::Gossip;
use wasm_bindgen::prelude::*;

mod opfs;

struct Node {
    blobs: MemStore,
    author: AuthorId,
    doc: iroh_docs::api::Doc,
    /// Present when the vault is persistent: note content is appended here so a
    /// reopened vault can read notes, not just list them.
    blob_log: Option<opfs::BlobLog>,
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
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .bind()
        .await?;
    // A browser has no direct addresses, so a ticket is only reachable once the
    // home relay is up. Minting one before that produces a ticket nobody can
    // dial — which is exactly what silently failed on the first run of this
    // spike: the join was accepted and then nothing ever synced, with no error.
    endpoint.online().await;
    let blobs = MemStore::new();
    let gossip = Gossip::builder().spawn(endpoint.clone());
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
                // The author key is 32 bytes; persisting it is a separate step
                // (see PERSISTENCE.md) and not what this test measures.
                DefaultAuthorStorage::Mem,
                None,
            )
            .await?;
            Docs::new(engine)
        }
    };
    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();
    // Reload note content before touching the replica, so entries read as
    // present rather than pending. Content-addressed, so re-adding the bytes
    // reproduces the hashes the entries already reference.
    let blob_log = match &storage {
        Storage::Memory => None,
        Storage::Opfs { file } => {
            let log = opfs::BlobLog::open(&format!("{file}.blobs"))
                .await
                .map_err(|e| anyhow!("opening the blob log on OPFS: {e:?}"))?;
            for bytes in log.read_all()? {
                blobs.add_bytes(bytes).await?;
            }
            Some(log)
        }
    };

    let author = docs.author_create().await?;
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
    let share = doc.share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses).await?;
    let id = endpoint.id().fmt_short();
    let addr = format!("{:?}", endpoint.addr());
    NODE.with(|n| {
        *n.borrow_mut() = Some(Rc::new(Node {
            blobs,
            author,
            doc,
            blob_log,
            _router: router,
            _endpoint: endpoint,
        }))
    });
    Ok(format!("{{\"endpoint\":\"{id}\",\"addr\":{addr:?},\"ticket\":\"{share}\"}}"))
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
    n.doc
        .set_bytes(n.author, key.into_bytes(), bytes.clone())
        .await
        .map_err(|e| err(anyhow!("{e:?}")))?;
    // After the entry, so a crash in between leaves an unreadable entry rather
    // than content nothing points at. (A real port would tie the two together.)
    if let Some(log) = &n.blob_log {
        log.append(&bytes).map_err(|e| err(anyhow!("{e}")))?;
    }
    Ok(())
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
