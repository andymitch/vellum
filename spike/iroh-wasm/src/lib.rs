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
    protocol::Docs,
    store::Query,
    AuthorId, DocTicket,
};
use iroh_gossip::net::Gossip;
use wasm_bindgen::prelude::*;

struct Node {
    blobs: MemStore,
    author: AuthorId,
    doc: iroh_docs::api::Doc,
    _router: Router,
    _endpoint: Endpoint,
}

thread_local! {
    static NODE: RefCell<Option<Rc<Node>>> = const { RefCell::new(None) };
}

fn node() -> Result<Rc<Node>> {
    NODE.with(|n| n.borrow().clone()).ok_or_else(|| anyhow!("start() first"))
}

async fn boot(ticket: Option<String>) -> Result<String> {
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
    let docs = Docs::memory()
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;
    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();
    let author = docs.author_create().await?;
    let doc = match &ticket {
        Some(t) => docs.import(t.parse::<DocTicket>()?).await?,
        None => docs.create().await?,
    };
    let share = doc.share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses).await?;
    let id = endpoint.id().fmt_short();
    let addr = format!("{:?}", endpoint.addr());
    NODE.with(|n| {
        *n.borrow_mut() = Some(Rc::new(Node {
            blobs,
            author,
            doc,
            _router: router,
            _endpoint: endpoint,
        }))
    });
    Ok(format!("{{\"endpoint\":\"{id}\",\"addr\":{addr:?},\"ticket\":\"{share}\"}}"))
}

#[wasm_bindgen]
pub async fn start(ticket: Option<String>) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    boot(ticket).await.map_err(err)
}

#[wasm_bindgen]
pub async fn write(key: String, value: String) -> Result<(), JsValue> {
    let n = node().map_err(err)?;
    n.doc
        .set_bytes(n.author, key.into_bytes(), value.into_bytes())
        .await
        .map_err(|e| err(anyhow!("{e:?}")))?;
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
