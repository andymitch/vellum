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

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use futures_lite::StreamExt;
use iroh::{endpoint::presets, protocol::Router, Endpoint};
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

const NAME_KEY: &[u8] = b"\x00meta/name";
const MARKER: u8 = 0x01;
const KEEP: &str = ".keep";

/// Managed Tauri state holding the live iroh node.
pub struct AppState {
    blobs: FsStore,
    docs: Docs,
    author: AuthorId,
    _router: Router,
    watched: Mutex<HashSet<NamespaceId>>,
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
pub async fn init(dir: PathBuf) -> Result<AppState> {
    std::fs::create_dir_all(&dir)?;

    let endpoint = Endpoint::bind(presets::N0).await?;
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

    Ok(AppState {
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

async fn open(state: &AppState, id: &str) -> Result<iroh_docs::api::Doc> {
    let nsid = parse_id(id)?;
    state
        .docs
        .open(nsid)
        .await?
        .ok_or_else(|| anyhow!("vault not found"))
}

async fn read_key(state: &AppState, doc: &iroh_docs::api::Doc, key: &[u8]) -> Result<Option<String>> {
    let Some(entry) = doc.get_one(Query::key_exact(key)).await? else {
        return Ok(None);
    };
    let bytes = state.blobs.blobs().get_bytes(entry.content_hash()).await?;
    Ok(Some(decode(&bytes)))
}

async fn vault_name(state: &AppState, doc: &iroh_docs::api::Doc) -> String {
    match read_key(state, doc, NAME_KEY).await {
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
        // Skip reserved meta keys.
        if key.first() == Some(&0x00) {
            continue;
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
    // Folders first, alphabetical.
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
    r.map_err(|e| e.to_string())
}

// ============================ commands ============================

#[tauri::command]
pub async fn list_vaults(state: State<'_, AppState>) -> Result<Vec<VaultInfo>, String> {
    map_err(
        async {
            let mut stream = state.docs.list().await?;
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                let (id, _cap) = item?;
                let doc = state.docs.open(id).await?.ok_or_else(|| anyhow!("open failed"))?;
                let name = vault_name(&state, &doc).await;
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
pub async fn create_vault(state: State<'_, AppState>, name: String) -> Result<VaultInfo, String> {
    map_err(
        async {
            let doc = state.docs.create().await?;
            doc.set_bytes(state.author, NAME_KEY.to_vec(), encode(&name)).await?;
            Ok(VaultInfo {
                id: doc.id().to_string(),
                name,
            })
        }
        .await,
    )
}

#[tauri::command]
pub async fn join_vault(state: State<'_, AppState>, ticket: String) -> Result<VaultInfo, String> {
    map_err(
        async {
            let ticket = DocTicket::from_str(&ticket).map_err(|e| anyhow!("bad ticket: {e}"))?;
            let doc = state.docs.import(ticket).await?;
            let name = vault_name(&state, &doc).await;
            Ok(VaultInfo {
                id: doc.id().to_string(),
                name,
            })
        }
        .await,
    )
}

#[tauri::command]
pub async fn share_vault(state: State<'_, AppState>, vault: String) -> Result<String, String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
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
pub async fn list_tree(state: State<'_, AppState>, vault: String) -> Result<Vec<TreeNode>, String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            let keys = list_keys(&doc).await?;
            let mut root = Builder::default();
            for key in &keys {
                let segments: Vec<&str> = key.split('/').collect();
                if segments.last() == Some(&KEEP) {
                    // Folder marker: register the folder, not a file.
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
pub async fn read_note(state: State<'_, AppState>, vault: String, path: String) -> Result<String, String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            Ok(read_key(&state, &doc, path.as_bytes()).await?.unwrap_or_default())
        }
        .await,
    )
}

#[tauri::command]
pub async fn write_note(
    state: State<'_, AppState>,
    vault: String,
    path: String,
    content: String,
) -> Result<(), String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            doc.set_bytes(state.author, path.into_bytes(), encode(&content)).await?;
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_note(state: State<'_, AppState>, vault: String, path: String) -> Result<(), String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            doc.set_bytes(state.author, path.into_bytes(), encode("")).await?;
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn create_folder(state: State<'_, AppState>, vault: String, path: String) -> Result<(), String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            let key = format!("{}/{}", path.trim_end_matches('/'), KEEP);
            doc.set_bytes(state.author, key.into_bytes(), encode("")).await?;
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn rename_path(
    state: State<'_, AppState>,
    vault: String,
    from: String,
    to: String,
    is_dir: bool,
) -> Result<(), String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            if is_dir {
                // Move every entry under the folder prefix.
                let from_prefix = format!("{}/", from.trim_end_matches('/'));
                let to_prefix = format!("{}/", to.trim_end_matches('/'));
                let keys = list_keys(&doc).await?;
                // also include .keep markers (list_keys already includes them)
                for key in keys.iter().filter(|k| k.starts_with(&from_prefix)) {
                    let new_key = format!("{}{}", to_prefix, &key[from_prefix.len()..]);
                    if let Some(content) = read_key(&state, &doc, key.as_bytes()).await? {
                        doc.set_bytes(state.author, new_key.into_bytes(), encode(&content)).await?;
                    }
                }
                doc.del(state.author, from_prefix.into_bytes()).await?;
            } else {
                let content = read_key(&state, &doc, from.as_bytes()).await?.unwrap_or_default();
                doc.set_bytes(state.author, to.into_bytes(), encode(&content)).await?;
                doc.del(state.author, from.into_bytes()).await?;
            }
            Ok(())
        }
        .await,
    )
}

#[tauri::command]
pub async fn delete_path(
    state: State<'_, AppState>,
    vault: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    map_err(
        async {
            let doc = open(&state, &vault).await?;
            let prefix = if is_dir {
                format!("{}/", path.trim_end_matches('/'))
            } else {
                path
            };
            doc.del(state.author, prefix.into_bytes()).await?;
            Ok(())
        }
        .await,
    )
}

/// Start emitting `vault-changed` events when a vault's document mutates.
#[tauri::command]
pub async fn watch_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    vault: String,
) -> Result<(), String> {
    let nsid = parse_id(&vault).map_err(|e| e.to_string())?;
    {
        let mut watched = state.watched.lock().unwrap();
        if !watched.insert(nsid) {
            return Ok(()); // already watching
        }
    }
    let doc = open(&state, &vault).await.map_err(|e| e.to_string())?;
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

    #[tokio::test]
    async fn vault_roundtrip() {
        let dir = std::env::temp_dir().join(format!("notes-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = init(dir.clone()).await.expect("init node");

        // create vault + name
        let doc = state.docs.create().await.expect("create doc");
        doc.set_bytes(state.author, NAME_KEY.to_vec(), encode("My Vault"))
            .await
            .expect("set name");
        assert_eq!(vault_name(&state, &doc).await, "My Vault");

        // write + read a note (content goes through blobs)
        doc.set_bytes(state.author, b"a/b.md".to_vec(), encode("hello"))
            .await
            .expect("set note");
        let got = read_key(&state, &doc, b"a/b.md").await.expect("read").unwrap();
        assert_eq!(got, "hello");

        // empty content stays alive (marker byte, not a tombstone)
        doc.set_bytes(state.author, b"empty.md".to_vec(), encode(""))
            .await
            .expect("set empty");
        let keys = list_keys(&doc).await.expect("list");
        assert!(keys.contains(&"a/b.md".to_string()));
        assert!(keys.contains(&"empty.md".to_string()));
        assert!(!keys.iter().any(|k| k.starts_with('\u{0}')), "meta key leaked");

        // delete removes it
        doc.del(state.author, b"a/b.md".to_vec()).await.expect("del");
        let keys2 = list_keys(&doc).await.expect("list2");
        assert!(!keys2.contains(&"a/b.md".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
