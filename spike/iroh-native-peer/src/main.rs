//! Native half of the browser<->native sync test: stand up a vault the way
//! vault.rs does, write a note, hand the share ticket out over loopback HTTP so
//! the browser page can fetch it, then wait for the browser's write to arrive.
//!
//! This is the pairing that matters for #221/#222 — a phone browser syncing with
//! the installed desktop app — and the direction a browser can actually do: it
//! dials, this side accepts.
use anyhow::Result;
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint, SecretKey};
use iroh_blobs::{store::mem::MemStore, BlobsProtocol};
use iroh_docs::{
    api::protocol::{AddrInfoOptions, ShareMode},
    protocol::Docs,
    store::Query,
};
use iroh_gossip::net::Gossip;
use std::io::{Read, Write};

#[tokio::main]
async fn main() -> Result<()> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(SecretKey::generate())
        .bind()
        .await?;
    // The browser peer can only reach us through a relay, so wait until we have
    // one before minting a ticket — the same trap that silently broke the first
    // browser-to-browser run.
    endpoint.online().await;
    let blobs = MemStore::new();
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let docs = Docs::memory()
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;
    let _router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(iroh_docs::ALPN, docs.clone())
        .spawn();

    let author = docs.author_create().await?;
    let doc = docs.create().await?;
    doc.set_bytes(author, b"from-native.md".to_vec(), b"written on the desktop".to_vec())
        .await?;
    let ticket = doc
        .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
        .await?
        .to_string();
    println!("HOST endpoint={}", endpoint.id().fmt_short());
    println!("HOST ticket={ticket}");

    // Minimal loopback server handing out the ticket, with CORS so the page
    // (served from another port) can read it.
    std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind("127.0.0.1:8899").expect("bind 8899");
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let res = format!(
                "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                ticket.len(),
                ticket
            );
            let _ = stream.write_all(res.as_bytes());
        }
    });

    // Wait for the browser's entry, content included: an entry without its blob
    // would mean the replica synced but the note itself didn't.
    for i in 0..120 {
        let mut entries = Box::pin(doc.get_many(Query::single_latest_per_key()).await?);
        let mut keys = Vec::new();
        let mut from_browser = None;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let key = String::from_utf8_lossy(entry.key()).to_string();
            if key == "from-browser.md" && entry.content_len() > 0 {
                if let Ok(bytes) = blobs.get_bytes(entry.content_hash()).await {
                    from_browser = Some(String::from_utf8_lossy(&bytes).to_string());
                }
            }
            keys.push(key);
        }
        if let Some(note) = from_browser {
            println!("HOST SUCCESS keys={keys:?} browser_wrote={note:?}");
            return Ok(());
        }
        if i % 10 == 0 {
            println!("HOST waiting… keys={keys:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!("HOST TIMEOUT: the browser's write never arrived");
    std::process::exit(1);
}
