//! Linked folders (#219): mirror a vault folder to a plain directory on disk so
//! notes can be opened, grepped, and diffed from an ordinary editor, kept in
//! sync in both directions.
//!
//! # Where the mirror lives
//!
//! A link never points at a directory the caller picks. Instead each link gets
//! two paths:
//! - a **canonical** directory under the app data dir (`local/<link-id>/`) —
//!   the one actually watched and written to, covered for free by the app's
//!   existing backup/uninstall story (see the README's "where your notes
//!   live");
//! - a **friendly** symlink at `~/.vellum/local/<slug>` pointing at it, so
//!   there's a short, memorable path to hand to an editor (e.g. Zed's "add
//!   folder to project").
//!
//! Because the mirror is never inside a git work tree, `git checkout` /
//! `git switch` / `git worktree` never touch it — there is nothing to
//! reconcile with a branch switch, unlike a directory chosen inside a repo.
//!
//! # Sync model
//!
//! Vault \<-\> disk sync is one `reconcile` pass over the union of vault
//! entries and local files, driven by a per-path `base` cache (the content as
//! of the last successful reconciliation for that path):
//! - path known on neither side before, present on one side only → copy it to
//!   the other side.
//! - known on neither side, present (and differing) on both → local wins,
//!   using the vault's current text as the merge base (same semantics as the
//!   MCP server's `write_note`) — this is what lets a non-empty target
//!   directory union with the vault on first sync instead of requiring an
//!   empty one.
//! - previously synced, now missing on one side → the deletion propagates
//!   (soft-delete in the vault, or remove the file) rather than resurrecting
//!   it from the other side.
//! - previously synced, changed on exactly one side → copy that side's text
//!   to the other.
//! - previously synced, changed on **both** sides → a real three-way merge via
//!   `write_note_merged`, using the cached base — the only case that needs the
//!   actual last-synced text rather than just knowing something changed.
//!
//! The `base` cache itself is persisted to a hidden `.vellum-sync-state.json`
//! file inside `local_dir` after every successful pass, and reloaded on the
//! next `start_link` — without this, a restart would forget what was already
//! synced, treat every already-synced path as "never synced, differs on both
//! sides", and let the stale local copy silently clobber a vault edit made
//! while the app was closed.
//!
//! A local file that exists but can't be read this pass (a permission error,
//! a transient I/O glitch, or non-UTF-8 content) is left out of the diff
//! entirely rather than treated as absent — a read failure must never look
//! like a deletion.
//!
//! `reconcile` is re-run (debounced) whenever either side signals a change, so
//! there is exactly one sync engine for the initial merge, steady-state vault
//! changes, and steady-state disk changes — no separate "patch" path to keep
//! consistent with it.
//!
//! # Renames (v1)
//!
//! Not detected as such: a rename on either side is a delete + create pair to
//! `reconcile`, same as an unrelated file replacing another. Good enough for a
//! first version; a follow-up can special-case matching content hashes.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::vault::{self, Node, VaultManager};

// ============================ config ============================

#[derive(Clone, Serialize, Deserialize)]
pub struct LinkConfig {
    pub id: String,
    pub vault: String,
    /// Folder prefix within the vault; "" links the whole vault.
    pub folder: String,
    /// Canonical storage — what's actually watched and written to.
    pub local_dir: PathBuf,
    /// The `~/.vellum/local/<slug>` symlink pointing at `local_dir`, for
    /// display and for the user to add to an editor.
    pub friendly_path: PathBuf,
    pub enabled: bool,
}

fn links_path(dir: &Path) -> PathBuf {
    dir.join("links.json")
}

fn load_links(dir: &Path) -> Vec<LinkConfig> {
    let Ok(s) = std::fs::read_to_string(links_path(dir)) else {
        return Vec::new();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

fn save_links(dir: &Path, links: &[LinkConfig]) {
    if let Ok(json) = serde_json::to_string_pretty(links) {
        let _ = std::fs::write(links_path(dir), json);
    }
}

/// An 8-byte random id, hex-encoded. Mirrors `mcp::new_token`'s approach —
/// good enough for a filesystem-safe unique id without a new dependency.
fn new_id() -> String {
    let buf: [u8; 8] = rand::random();
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

// ============================ naming ============================

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true; // suppress a leading dash
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "notes".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The last folder segment names a link best (`projects/hermes` -> `hermes`);
/// a whole-vault link (no folder) falls back to the vault's display name.
fn default_slug(vault_name: &str, folder: &str) -> String {
    let f = folder.trim().trim_matches('/');
    match f.rsplit('/').next().filter(|s| !s.is_empty()) {
        Some(last) => slugify(last),
        None => slugify(vault_name),
    }
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(unix)]
fn remove_symlink_dir(link: &Path) -> std::io::Result<()> {
    std::fs::remove_file(link)
}
#[cfg(windows)]
fn remove_symlink_dir(link: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(link)
}

/// Create `~/.vellum/local/<slug>` pointing at `target`, appending a short
/// suffix from `link_id` if `<slug>` is already taken by another link.
fn create_friendly_symlink(home: &Path, slug: &str, link_id: &str, target: &Path) -> Result<PathBuf> {
    let base_dir = home.join(".vellum").join("local");
    std::fs::create_dir_all(&base_dir)?;
    let mut candidate = base_dir.join(slug);
    if candidate.symlink_metadata().is_ok() {
        let suffix = &link_id[..link_id.len().min(6)];
        candidate = base_dir.join(format!("{slug}-{suffix}"));
    }
    symlink_dir(target, &candidate)?;
    Ok(candidate)
}

/// A folder prefix within the vault, normalized to either "" (whole vault) or
/// "segment/segment/". Rejects the same shapes `mcp::clean_path` does — this
/// becomes part of every note key the link touches.
fn clean_folder(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.contains('\0') || trimmed.contains('\\') {
        return Err("folder contains an illegal character (NUL or '\\')".into());
    }
    if trimmed.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err("folder contains an empty or relative ('.', '..') segment".into());
    }
    Ok(format!("{trimmed}/"))
}

// ============================ reconciliation ============================

fn write_local_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// `.vellum-sync-state.json` inside a link's `local_dir`: the per-path
/// last-synced content cache (`reconcile`'s `base`), persisted so a restart or
/// a link toggle doesn't forget what was already synced. Named so
/// `walk_local`'s dotfile skip hides it from the mirror's own listing.
fn base_cache_path(local_dir: &Path) -> PathBuf {
    local_dir.join(".vellum-sync-state.json")
}

fn load_base_cache(local_dir: &Path) -> HashMap<String, String> {
    let Ok(s) = std::fs::read_to_string(base_cache_path(local_dir)) else {
        return HashMap::new();
    };
    serde_json::from_str(&s).unwrap_or_default()
}

fn save_base_cache(local_dir: &Path, base: &HashMap<String, String>) {
    let Ok(json) = serde_json::to_string(base) else {
        return;
    };
    if let Err(e) = std::fs::write(base_cache_path(local_dir), json) {
        tracing::warn!("[link] could not persist sync state for {local_dir:?}: {e}");
    }
}

/// Every plain-text file under `dir`, keyed by its path relative to `dir` with
/// forward slashes, plus the set of paths that exist but couldn't be read
/// (permission error, transient I/O, or non-UTF-8 content) — kept distinct
/// from "absent" so `reconcile` doesn't mistake a read failure for a deletion
/// and propagate one. Dotfiles/dot-directories are skipped (the same policy
/// `vault::import_vault` uses), which also hides `base_cache_path`'s own file.
fn read_local_texts(dir: &Path) -> (HashMap<String, String>, HashSet<String>) {
    let mut texts = HashMap::new();
    let mut unreadable = HashSet::new();
    walk_local(dir, dir, &mut texts, &mut unreadable);
    (texts, unreadable)
}

fn walk_local(root: &Path, dir: &Path, out: &mut HashMap<String, String>, unreadable: &mut HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            walk_local(root, &path, out, unreadable);
        } else if file_type.is_file() {
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let Some(rel_str) = rel.to_str() else {
                continue; // non-UTF8 path — skip rather than guess
            };
            let key = rel_str.replace('\\', "/");
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    out.insert(key, text);
                }
                Err(e) => {
                    tracing::warn!(
                        "[link] could not read {path:?}: {e} — leaving it out of this pass rather than treating it as deleted"
                    );
                    unreadable.insert(key);
                }
            }
        }
    }
}

/// Write `content` as the vault's merge `content` against its current text as
/// `base`, then read back the merged result (which may differ from `content`
/// if a concurrent edit produced inline conflict markers).
async fn write_and_reread(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    vault_path: &str,
    base: &str,
    content: &str,
) -> Result<String> {
    vault::write_note_merged(node, doc, vault_path.as_bytes(), base, content).await?;
    Ok(vault::read_note_text(node, doc, vault_path.as_bytes())
        .await?
        .unwrap_or_default())
}

/// Push local content to the vault, using the vault's own current text as the
/// merge base — i.e. "local wins" for whatever the vault doesn't already
/// agree on. Used when there's no better (previously-synced) base to merge
/// against.
async fn push_to_vault(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    vault_path: &str,
    content: &str,
) -> Result<String> {
    let cur = vault::read_note_text(node, doc, vault_path.as_bytes())
        .await?
        .unwrap_or_default();
    write_and_reread(node, doc, vault_path, &cur, content).await
}

/// One reconciliation pass: union the vault's notes under `folder` with the
/// files under `local_dir`, resolve every path per the module doc, and update
/// `base` (the per-path last-synced content cache) as it goes.
///
/// This single function is the whole sync engine — see the module doc for why
/// there's no separate "apply one disk event" / "apply one vault event" path.
pub(crate) async fn reconcile(
    node: &Node,
    doc: &iroh_docs::api::Doc,
    folder: &str,
    local_dir: &Path,
    base: &mut HashMap<String, String>,
) -> Result<()> {
    std::fs::create_dir_all(local_dir)?;

    let mut vault_texts: HashMap<String, String> = HashMap::new();
    for entry in vault::list_entries(doc).await? {
        if vault::is_hidden_path(&entry.path) {
            continue;
        }
        let Some(rel) = entry.path.strip_prefix(folder) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        if let Some(text) = vault::read_note_text(node, doc, entry.path.as_bytes()).await? {
            vault_texts.insert(rel.to_string(), text);
        }
    }
    let (local_texts, local_unreadable) = read_local_texts(local_dir);

    let mut all: BTreeSet<String> = base.keys().cloned().collect();
    all.extend(vault_texts.keys().cloned());
    all.extend(local_texts.keys().cloned());
    all.extend(local_unreadable.iter().cloned());

    for rel in all {
        if local_unreadable.contains(&rel) {
            // Present on disk but unreadable right now (permission error,
            // transient I/O, or non-UTF-8 content) — must not look like a
            // deletion. Leave it for a later pass once it reads cleanly.
            continue;
        }
        let vault_path = format!("{folder}{rel}");
        let local_path = local_dir.join(&rel);
        let v = vault_texts.get(&rel);
        let l = local_texts.get(&rel);
        let known = base.get(&rel).cloned();

        match (known, v, l) {
            // Converged already (including the very first pass over a
            // pre-populated mirror) — just record the shared text.
            (_, Some(v), Some(l)) if v == l => {
                base.insert(rel, v.clone());
            }
            // Never synced, vault-only -> materialize it locally.
            (None, Some(v), None) => {
                write_local_file(&local_path, v)?;
                base.insert(rel, v.clone());
            }
            // Never synced, local-only -> push it up.
            (None, None, Some(l)) => {
                let merged = push_to_vault(node, doc, &vault_path, l).await?;
                write_local_file(&local_path, &merged)?;
                base.insert(rel, merged);
            }
            // Never synced, present (and differing) on both -> local wins,
            // merged against the vault's current text (answers the "non-empty
            // target directory" question from #219: union, don't require empty).
            (None, Some(v), Some(l)) => {
                let merged = write_and_reread(node, doc, &vault_path, v, l).await?;
                write_local_file(&local_path, &merged)?;
                base.insert(rel, merged);
            }
            // Previously synced, local file is gone -> the user deleted it;
            // soft-delete the note rather than resurrecting the file.
            (Some(_), Some(_), None) => {
                vault::trash_note(node, doc, &vault_path).await?;
                base.remove(&rel);
            }
            // Previously synced, vault note is gone (deleted or trashed) ->
            // remove the local file rather than pushing it back.
            (Some(_), None, Some(_)) => {
                let _ = std::fs::remove_file(&local_path);
                base.remove(&rel);
            }
            // Both sides already agree it's gone.
            (Some(_), None, None) => {
                base.remove(&rel);
            }
            // Nothing on either side and never synced — nothing to do (can
            // only arise if a path is briefly present in the union set).
            (None, None, None) => {}
            // Previously synced, present (and differing) on both.
            (Some(h), Some(v), Some(l)) => {
                if h == *v {
                    // Vault unchanged since last sync -> local edit, push it.
                    let merged = push_to_vault(node, doc, &vault_path, l).await?;
                    write_local_file(&local_path, &merged)?;
                    base.insert(rel, merged);
                } else if h == *l {
                    // Local unchanged since last sync -> vault edit, pull it.
                    write_local_file(&local_path, v)?;
                    base.insert(rel, v.clone());
                } else {
                    // Both changed since last sync -> a real three-way merge,
                    // with the last-synced text as the base (the one case that
                    // needs it rather than just a hash).
                    let merged = write_and_reread(node, doc, &vault_path, &h, l).await?;
                    write_local_file(&local_path, &merged)?;
                    base.insert(rel, merged);
                }
            }
        }
    }
    Ok(())
}

// ============================ manager ============================

/// A large burst of simultaneous filesystem events (e.g. an accidental bulk
/// delete on the mirror directory) is treated as suspicious rather than synced
/// blindly: crossing this suspends sync in **both** directions for the link
/// (see `spawn_disk_watcher`'s `suspended` flag) until it's toggled off and
/// on, rather than just skipping the one pass that tripped it — `reconcile`
/// always diffs full state, so resuming on the next small edit would still
/// apply the very burst this guard exists to hold back.
const LARGE_BURST_THRESHOLD: u32 = 20;

struct LinkHandle {
    _watcher: RecommendedWatcher,
    cancel: CancellationToken,
}

/// Managed Tauri state: the app data dir (for `links.json` and canonical
/// storage) plus the live handles for every currently-running link.
pub struct LinkManager {
    dir: PathBuf,
    handles: Mutex<HashMap<String, LinkHandle>>,
}

impl LinkManager {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            handles: Mutex::new(HashMap::new()),
        }
    }
}

/// What the Settings UI shows for one link.
#[derive(Serialize, Clone)]
pub struct LinkInfo {
    pub id: String,
    pub vault: String,
    pub vault_name: String,
    /// Folder prefix without the trailing slash ("" for a whole-vault link).
    pub folder: String,
    /// The friendly `~/.vellum/local/<slug>` path — what to add to an editor.
    pub path: String,
    pub enabled: bool,
}

async fn vault_name_of(node: &Node, vault: &str) -> String {
    vault::all_vaults(node)
        .await
        .ok()
        .and_then(|vs| vs.into_iter().find(|v| v.id == vault).map(|v| v.name))
        .unwrap_or_else(|| vault.to_string())
}

async fn info_of(node: &Node, cfg: &LinkConfig) -> LinkInfo {
    LinkInfo {
        id: cfg.id.clone(),
        vault: cfg.vault.clone(),
        vault_name: vault_name_of(node, &cfg.vault).await,
        folder: cfg.folder.trim_end_matches('/').to_string(),
        path: cfg.friendly_path.to_string_lossy().into_owned(),
        enabled: cfg.enabled,
    }
}

async fn drain_quiet(rx: &mut tokio::sync::broadcast::Receiver<vault::VaultChange>, quiet: Duration) {
    loop {
        if tokio::time::timeout(quiet, rx.recv()).await.is_err() {
            return; // quiet period elapsed
        }
    }
}

/// Vault -> disk: reconcile on every mutation of this link's vault (debounced),
/// so an edit made elsewhere (the editor, a peer device) lands in the mirror.
fn spawn_vault_watcher(
    app: AppHandle,
    cfg: LinkConfig,
    base: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    cancel: CancellationToken,
    suspended: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        let mgr = app.state::<VaultManager>();
        let Ok(node) = mgr.node().await else { return };
        let Ok(nsid) = vault::parse_id(&cfg.vault) else { return };
        let mut rx = node.subscribe_changes();
        loop {
            let relevant = tokio::select! {
                _ = cancel.cancelled() => return,
                ev = rx.recv() => match ev {
                    Ok(change) => change.vault == nsid,
                    Err(RecvError::Lagged(_)) => true,
                    Err(RecvError::Closed) => return,
                },
            };
            if !relevant {
                continue;
            }
            // A suspicious disk-side burst suspends both directions until the
            // link is toggled off and on (see spawn_disk_watcher).
            if suspended.load(Ordering::Relaxed) {
                continue;
            }
            drain_quiet(&mut rx, Duration::from_millis(500)).await;
            let Ok(doc) = vault::open(node, &cfg.vault).await else { continue };
            let mut b = base.lock().await;
            match reconcile(node, &doc, &cfg.folder, &cfg.local_dir, &mut b).await {
                Ok(()) => save_base_cache(&cfg.local_dir, &b),
                Err(e) => tracing::warn!("[link] vault->disk reconcile failed for {}: {e}", cfg.id),
            }
        }
    });
}

/// Disk -> vault: reconcile whenever the mirror directory changes (debounced),
/// guarded against an oversized burst (see `LARGE_BURST_THRESHOLD`).
fn spawn_disk_watcher(
    app: AppHandle,
    cfg: LinkConfig,
    base: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    cancel: CancellationToken,
    suspended: Arc<AtomicBool>,
) -> notify::Result<RecommendedWatcher> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        // Access events (a plain read) carry nothing to sync; everything else
        // (create/modify/remove/rename) is worth a reconcile pass.
        if matches!(res, Ok(ev) if !matches!(ev.kind, EventKind::Access(_))) {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(&cfg.local_dir, RecursiveMode::Recursive)?;

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                got = rx.recv() => {
                    if got.is_none() { return; }
                }
            }
            // Once a burst has tripped the guard below, every later event —
            // however small — is ignored too: reconcile always diffs full
            // state, so without this a single unrelated edit would apply the
            // very mass-change this guard exists to hold back. Stays this way
            // until the link is toggled off and on (a fresh `start_link`
            // rebuilds this flag as `false`).
            if suspended.load(Ordering::Relaxed) {
                continue;
            }
            // Debounce, counting the burst so a mass-change can be treated as
            // suspicious rather than synced blindly.
            let mut count: u32 = 1;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(300)) => break,
                    more = rx.recv() => {
                        if more.is_none() { return; }
                        count += 1;
                    }
                }
            }
            if count > LARGE_BURST_THRESHOLD {
                suspended.store(true, Ordering::Relaxed);
                tracing::warn!(
                    "[link] {count} file events on {:?} in one burst — suspending sync for this \
                     link (both directions); toggle it off and on to reconcile once you've \
                     confirmed it's intentional",
                    cfg.local_dir
                );
                continue;
            }
            let mgr = app.state::<VaultManager>();
            let Ok(node) = mgr.node().await else { continue };
            let Ok(doc) = vault::open(node, &cfg.vault).await else { continue };
            let mut b = base.lock().await;
            match reconcile(node, &doc, &cfg.folder, &cfg.local_dir, &mut b).await {
                Ok(()) => save_base_cache(&cfg.local_dir, &b),
                Err(e) => tracing::warn!("[link] disk->vault reconcile failed for {}: {e}", cfg.id),
            }
        }
    });
    Ok(watcher)
}

/// Run the initial reconciliation and start both watchers for `cfg`.
async fn start_link(app: &AppHandle, cfg: LinkConfig) -> Result<()> {
    let mgr = app.state::<VaultManager>();
    let node = mgr.node().await.map_err(|e| anyhow!(e))?;
    let nsid = vault::parse_id(&cfg.vault)?;
    vault::arm_vault(app, node, nsid).await.map_err(|e| anyhow!(e))?;
    let doc = vault::open(node, &cfg.vault).await?;

    // Reload rather than start empty — an empty base would make reconcile
    // treat every already-synced path as "never synced", see the module doc.
    let base = Arc::new(tokio::sync::Mutex::new(load_base_cache(&cfg.local_dir)));
    {
        let mut b = base.lock().await;
        reconcile(node, &doc, &cfg.folder, &cfg.local_dir, &mut b).await?;
        save_base_cache(&cfg.local_dir, &b);
    }

    let cancel = CancellationToken::new();
    // Tripped by a suspicious disk-side burst; suspends sync in both
    // directions for this link until it's toggled off and on again (a fresh
    // `start_link` call, which rebuilds this as `false`).
    let suspended = Arc::new(AtomicBool::new(false));
    let watcher = spawn_disk_watcher(app.clone(), cfg.clone(), base.clone(), cancel.clone(), suspended.clone())?;
    spawn_vault_watcher(app.clone(), cfg.clone(), base, cancel.clone(), suspended);

    let link_mgr = app.state::<LinkManager>();
    link_mgr.handles.lock().unwrap().insert(
        cfg.id.clone(),
        LinkHandle {
            _watcher: watcher,
            cancel,
        },
    );
    Ok(())
}

/// Stop a link's watchers, if running. Does not touch its files or config.
fn stop_link(app: &AppHandle, id: &str) {
    let link_mgr = app.state::<LinkManager>();
    let removed = link_mgr.handles.lock().unwrap().remove(id);
    if let Some(handle) = removed {
        handle.cancel.cancel();
    }
}

// ============================ commands ============================
// Thin async functions, not `#[tauri::command]` themselves — lib.rs supplies
// the `#[cfg(desktop)]` / `#[cfg(not(desktop))]` command wrappers, mirroring
// how mcp.rs's `status`/`set_enabled` are wired up.

pub async fn list_links(app: &AppHandle) -> Result<Vec<LinkInfo>, String> {
    let mgr = app.state::<VaultManager>();
    let node = mgr.node().await?;
    let link_mgr = app.state::<LinkManager>();
    let mut out = Vec::new();
    for cfg in load_links(&link_mgr.dir) {
        out.push(info_of(node, &cfg).await);
    }
    Ok(out)
}

pub async fn add_link(app: &AppHandle, vault: String, folder: String) -> Result<LinkInfo, String> {
    let folder = clean_folder(&folder)?;
    let mgr = app.state::<VaultManager>();
    let node = mgr.node().await?;
    // Validate the vault id and grab its name for slug derivation up front,
    // so a bad id fails before anything is created on disk.
    let vault_name = vault_name_of(node, &vault).await;
    vault::parse_id(&vault).map_err(|e| e.to_string())?;

    let id = new_id();
    let link_mgr = app.state::<LinkManager>();
    let local_dir = link_mgr.dir.join("local").join(&id);
    std::fs::create_dir_all(&local_dir).map_err(|e| e.to_string())?;
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let slug = default_slug(&vault_name, &folder);
    let friendly_path = create_friendly_symlink(&home, &slug, &id, &local_dir).map_err(|e| e.to_string())?;

    let cfg = LinkConfig {
        id,
        vault,
        folder,
        local_dir,
        friendly_path,
        enabled: true,
    };
    let mut cfgs = load_links(&link_mgr.dir);
    cfgs.push(cfg.clone());
    save_links(&link_mgr.dir, &cfgs);

    if let Err(e) = start_link(app, cfg.clone()).await {
        // Don't leave a persisted link with nothing actually running — the
        // frontend has no way to learn about that until a restart otherwise.
        let mut cfgs = load_links(&link_mgr.dir);
        cfgs.retain(|c| c.id != cfg.id);
        save_links(&link_mgr.dir, &cfgs);
        let _ = std::fs::remove_dir_all(&cfg.local_dir);
        let _ = remove_symlink_dir(&cfg.friendly_path);
        return Err(e.to_string());
    }
    Ok(info_of(node, &cfg).await)
}

pub async fn remove_link(app: &AppHandle, id: String) -> Result<(), String> {
    stop_link(app, &id);
    let link_mgr = app.state::<LinkManager>();
    let mut cfgs = load_links(&link_mgr.dir);
    let Some(pos) = cfgs.iter().position(|c| c.id == id) else {
        return Err("link not found".into());
    };
    let cfg = cfgs.remove(pos);
    save_links(&link_mgr.dir, &cfgs);
    // Best-effort cleanup: the vault remains the source of truth, so nothing
    // of value is lost by removing the mirror.
    let _ = std::fs::remove_dir_all(&cfg.local_dir);
    let _ = remove_symlink_dir(&cfg.friendly_path);
    Ok(())
}

pub async fn set_link_enabled(app: &AppHandle, id: String, enabled: bool) -> Result<LinkInfo, String> {
    let link_mgr = app.state::<LinkManager>();
    let mut cfgs = load_links(&link_mgr.dir);
    let Some(cfg) = cfgs.iter_mut().find(|c| c.id == id) else {
        return Err("link not found".into());
    };
    cfg.enabled = enabled;
    let cfg = cfg.clone();
    save_links(&link_mgr.dir, &cfgs);

    if enabled {
        if let Err(e) = start_link(app, cfg.clone()).await {
            // Roll back the persisted flag: a relaunch shouldn't silently keep
            // retrying (and failing) a link the caller was just told about
            // via this error.
            let mut cfgs = load_links(&link_mgr.dir);
            if let Some(c) = cfgs.iter_mut().find(|c| c.id == id) {
                c.enabled = false;
            }
            save_links(&link_mgr.dir, &cfgs);
            return Err(e.to_string());
        }
    } else {
        stop_link(app, &id);
    }
    let mgr = app.state::<VaultManager>();
    let node = mgr.node().await?;
    Ok(info_of(node, &cfg).await)
}

/// Resume every enabled link on launch, mirroring `mcp::start_if_enabled`.
pub fn start_enabled_links(app: &AppHandle) {
    let link_mgr = app.state::<LinkManager>();
    let cfgs: Vec<LinkConfig> = load_links(&link_mgr.dir).into_iter().filter(|c| c.enabled).collect();
    for cfg in cfgs {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = start_link(&app, cfg.clone()).await {
                tracing::error!("[link] could not start {} on launch: {e}", cfg.id);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::init;

    async fn fixture(name: &str) -> (Node, iroh_docs::api::Doc, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("vellum-link-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let node = init(base.join("data")).await.expect("node");
        let doc = node.docs().create().await.expect("create vault");
        let local_dir = base.join("local");
        std::fs::create_dir_all(&local_dir).unwrap();
        (node, doc, local_dir, base)
    }

    #[test]
    fn slugs_derive_from_folder_or_vault_name() {
        assert_eq!(default_slug("Hermes Notes", "projects/hermes"), "hermes");
        assert_eq!(default_slug("Hermes Notes", ""), "hermes-notes");
        assert_eq!(slugify("Q3 Goals!!"), "q3-goals");
        assert_eq!(slugify("---"), "notes");
    }

    #[test]
    fn folder_prefix_normalizes_and_rejects() {
        assert_eq!(clean_folder("projects/hermes").unwrap(), "projects/hermes/");
        assert_eq!(clean_folder("/projects/hermes/").unwrap(), "projects/hermes/");
        assert_eq!(clean_folder("").unwrap(), "");
        assert!(clean_folder("../secrets").is_err());
        assert!(clean_folder("a//b").is_err());
    }

    #[tokio::test]
    async fn reconcile_unions_vault_only_and_local_only_files() {
        let (node, doc, local_dir, base_dir) = fixture("union").await;
        doc.set_bytes(node.author(), b"a.md".to_vec(), crate::vault::fresh_note("from vault"))
            .await
            .unwrap();
        std::fs::write(local_dir.join("b.md"), "from disk").unwrap();

        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        assert_eq!(std::fs::read_to_string(local_dir.join("a.md")).unwrap(), "from vault");
        assert_eq!(
            vault::read_note_text(&node, &doc, b"b.md").await.unwrap().as_deref(),
            Some("from disk")
        );
        assert_eq!(base.get("a.md").map(String::as_str), Some("from vault"));
        assert_eq!(base.get("b.md").map(String::as_str), Some("from disk"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn reconcile_propagates_one_sided_edits_after_first_sync() {
        let (node, doc, local_dir, base_dir) = fixture("edits").await;
        doc.set_bytes(node.author(), b"note.md".to_vec(), crate::vault::fresh_note("v1"))
            .await
            .unwrap();
        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        assert_eq!(base.get("note.md").map(String::as_str), Some("v1"));

        // Local edit, vault unchanged -> pushes up.
        std::fs::write(local_dir.join("note.md"), "v2-local").unwrap();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        assert_eq!(
            vault::read_note_text(&node, &doc, b"note.md").await.unwrap().as_deref(),
            Some("v2-local")
        );

        // Vault edit, local unchanged -> pulls down.
        vault::write_note_merged(&node, &doc, b"note.md", "v2-local", "v3-vault")
            .await
            .unwrap();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        assert_eq!(std::fs::read_to_string(local_dir.join("note.md")).unwrap(), "v3-vault");

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn reconcile_merges_concurrent_edits_on_both_sides() {
        let (node, doc, local_dir, base_dir) = fixture("merge").await;
        doc.set_bytes(node.author(), b"note.md".to_vec(), crate::vault::fresh_note("L1\nL2\n"))
            .await
            .unwrap();
        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        // Vault changes (e.g. the editor, or a peer)...
        vault::write_note_merged(&node, &doc, b"note.md", "L1\nL2\n", "L1\nL2\nfrom-vault\n")
            .await
            .unwrap();
        // ...and, independently, the local mirror changes too.
        std::fs::write(local_dir.join("note.md"), "L1-local\nL2\n").unwrap();

        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        let merged = std::fs::read_to_string(local_dir.join("note.md")).unwrap();
        assert!(merged.contains("L1-local"), "local edit lost: {merged:?}");
        assert!(merged.contains("from-vault"), "vault edit lost: {merged:?}");

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn reconcile_propagates_deletes_without_resurrecting() {
        let (node, doc, local_dir, base_dir) = fixture("delete").await;
        doc.set_bytes(node.author(), b"a.md".to_vec(), crate::vault::fresh_note("keep"))
            .await
            .unwrap();
        doc.set_bytes(node.author(), b"b.md".to_vec(), crate::vault::fresh_note("keep"))
            .await
            .unwrap();
        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        // Deleting the local file soft-deletes the vault note...
        std::fs::remove_file(local_dir.join("a.md")).unwrap();
        // ...and deleting the vault note (soft-delete, same as a real delete
        // from the reconciliation's point of view) removes the local file.
        vault::trash_note(&node, &doc, "b.md").await.unwrap();

        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        assert!(
            vault::read_note_text(&node, &doc, b"a.md").await.unwrap().is_none(),
            "locally-deleted note should be gone from the vault"
        );
        assert!(
            !local_dir.join("b.md").exists(),
            "vault-deleted note should be removed locally, not resurrected"
        );
        assert!(!base.contains_key("a.md"));
        assert!(!base.contains_key("b.md"));

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[tokio::test]
    async fn reconcile_scopes_to_folder_prefix() {
        let (node, doc, local_dir, base_dir) = fixture("folder").await;
        doc.set_bytes(node.author(), b"projects/hermes/a.md".to_vec(), crate::vault::fresh_note("in scope"))
            .await
            .unwrap();
        doc.set_bytes(node.author(), b"other.md".to_vec(), crate::vault::fresh_note("out of scope"))
            .await
            .unwrap();

        let mut base = HashMap::new();
        reconcile(&node, &doc, "projects/hermes/", &local_dir, &mut base).await.unwrap();

        assert_eq!(std::fs::read_to_string(local_dir.join("a.md")).unwrap(), "in scope");
        assert!(!local_dir.join("other.md").exists());

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    /// The fix for a data-loss bug found in review: without persisting `base`
    /// across a restart, reconcile would forget a path was already synced and
    /// treat the vault's peer-made edit as "never synced, differs on both
    /// sides" — letting the stale local mirror silently win and clobber it.
    #[tokio::test]
    async fn base_cache_survives_a_restart_and_prevents_clobbering_a_peer_edit() {
        let (node, doc, local_dir, base_dir) = fixture("restart").await;
        doc.set_bytes(node.author(), b"note.md".to_vec(), crate::vault::fresh_note("v1"))
            .await
            .unwrap();

        // First launch: reconcile, then persist `base` — what `start_link` now
        // does after every successful pass.
        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        save_base_cache(&local_dir, &base);
        drop(base);

        // While the app is "closed", a peer edits the note directly in the vault.
        vault::write_note_merged(&node, &doc, b"note.md", "v1", "v2-from-peer")
            .await
            .unwrap();

        // "Restart": rebuild `base` from disk instead of starting empty, exactly
        // as `start_link` does.
        let mut base = load_base_cache(&local_dir);
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(local_dir.join("note.md")).unwrap(),
            "v2-from-peer",
            "a restart with a persisted base must pull the peer's edit, not clobber it with \
             the stale local mirror"
        );

        let _ = std::fs::remove_dir_all(&base_dir);
    }

    /// A second bug found in review: a read failure (permission error,
    /// transient I/O, non-UTF-8) must not look like the file was deleted.
    #[tokio::test]
    async fn an_unreadable_local_file_is_not_treated_as_a_deletion() {
        let (node, doc, local_dir, base_dir) = fixture("unreadable").await;
        doc.set_bytes(node.author(), b"note.md".to_vec(), crate::vault::fresh_note("keep"))
            .await
            .unwrap();
        let mut base = HashMap::new();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();
        assert_eq!(base.get("note.md").map(String::as_str), Some("keep"));

        // Non-UTF-8 bytes fail `read_to_string` the same way a permission
        // error or a transient I/O glitch would.
        std::fs::write(local_dir.join("note.md"), [0xff, 0xfe, 0xfd]).unwrap();
        reconcile(&node, &doc, "", &local_dir, &mut base).await.unwrap();

        assert_eq!(
            base.get("note.md").map(String::as_str),
            Some("keep"),
            "an unreadable file must not be treated as deleted"
        );
        assert!(
            vault::read_note_text(&node, &doc, b"note.md").await.unwrap().is_some(),
            "the vault note must survive an unreadable (not deleted) local file"
        );

        let _ = std::fs::remove_dir_all(&base_dir);
    }
}
