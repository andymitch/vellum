//! Local MCP server, so agents (Claude Code, Claude Desktop, any MCP client) can
//! read and write notes in a vault.
//!
//! # Why this lives inside the app
//!
//! The iroh-docs store is single-writer — the same reason `tauri-plugin-single-
//! instance` exists (a second process would contend for it). So the MCP server
//! cannot be a standalone binary opening the same data directory; it runs in
//! this process, sharing the live `VaultManager` / iroh `Node`. That in turn
//! forces an HTTP transport rather than stdio (stdio would need a child
//! process), so we listen on loopback and require a bearer token.
//!
//! Because it shares the node, an agent's write goes through exactly the same
//! path as the editor's: `write_note_merged` → yrs CRDT → iroh sync. The open
//! editor rebases from the resulting `vault-changed` event, and peers converge
//! as usual. Nothing here re-implements storage.
//!
//! # Writes never clobber
//!
//! Every mutating tool reads the note's current merged text and passes it as the
//! merge base, so a peer's concurrent edit is preserved (issue #99). The agent
//! never supplies a base and so cannot get it wrong — the read and the write
//! happen here, in one process, against the live CRDT.
//!
//! # Platform
//!
//! Nothing in this module is desktop-specific; only its `mod` declaration and
//! startup wiring in `lib.rs` are gated. A phone can't usefully host it today
//! (the Claude mobile app's connectors dial out from Anthropic's servers, so a
//! loopback listener on the same device is unreachable, and Android freezes the
//! process when backgrounded) — but that's policy, not a code constraint.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    extract::State as AxumState,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    Router,
};
use rmcp::{
    handler::server::{
        router::prompt::PromptRouter,
        tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    schemars::{self, JsonSchema},
    service::{Peer, RequestContext, RoleServer, SubscriptionContext},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, tower::StreamableHttpServerConfig,
        StreamableHttpService,
    },
    ErrorData, ServerHandler,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::vault::{self, Node, VaultChange, VaultManager, KEEP, TRASH};

/// URI scheme for this server's resources.
const SCHEME: &str = "vellum://";

// ============================ config ============================

/// Persisted server settings, in `mcp.json` beside `peers.json` in the app data
/// dir. The token survives toggling the server off and on so a client's saved
/// connection keeps working; the port is remembered for the same reason.
#[derive(Serialize, Deserialize, Clone)]
pub struct McpConfig {
    pub enabled: bool,
    /// Last port we successfully bound; 0 until the first start.
    pub port: u16,
    pub token: String,
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join("mcp.json")
}

/// A 32-byte random token, hex-encoded.
fn new_token() -> String {
    let buf: [u8; 32] = rand::random();
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Load the config, generating (and persisting) a token on first use.
fn load_config(dir: &Path) -> McpConfig {
    if let Ok(s) = std::fs::read_to_string(config_path(dir)) {
        if let Ok(cfg) = serde_json::from_str::<McpConfig>(&s) {
            if !cfg.token.is_empty() {
                return cfg;
            }
        }
    }
    let cfg = McpConfig {
        enabled: false,
        port: 0,
        token: new_token(),
    };
    save_config(dir, &cfg);
    cfg
}

fn save_config(dir: &Path, cfg: &McpConfig) {
    let Ok(json) = serde_json::to_string_pretty(cfg) else {
        return;
    };
    if let Err(e) = std::fs::write(config_path(dir), json) {
        tracing::warn!("[mcp] could not persist mcp.json: {e}");
    }
    // The token is a credential — keep it off other users' prying eyes. Best
    // effort: a failure here doesn't stop the server (the file is inside the
    // app's own data dir).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(config_path(dir), std::fs::Permissions::from_mode(0o600));
    }
}

// ============================ paths ============================

/// Normalize and validate a vault-relative path.
///
/// Leading slashes are stripped rather than rejected — agents produce them
/// routinely and the result is unambiguous (every tool echoes back the path it
/// actually used). Genuinely unsafe or meaningless shapes are refused: `..`
/// traversal, NUL, backslashes (which would read as a literal key character,
/// not a separator), empty segments, and a trailing slash.
fn clean_path(raw: &str) -> Result<String, ErrorData> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Err(bad_request("path is empty"));
    }
    if trimmed.ends_with('/') {
        return Err(bad_request("path must not end with '/'"));
    }
    if trimmed.contains('\0') || trimmed.contains('\\') {
        return Err(bad_request("path contains an illegal character (NUL or '\\')"));
    }
    if trimmed.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err(bad_request("path contains an empty or relative ('.', '..') segment"));
    }
    Ok(trimmed.to_string())
}

/// A note path — as `clean_path`, plus the `.md` extension the app assumes
/// (mirroring `createAndOpenNote` in the frontend).
fn clean_note_path(raw: &str) -> Result<String, ErrorData> {
    let p = clean_path(raw)?;
    Ok(if p.ends_with(".md") { p } else { format!("{p}.md") })
}

fn bad_request(msg: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(msg.into(), None)
}

fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// iroh-docs stamps entries in microseconds; agents are better served by the
/// unix-milliseconds convention they already know.
fn ms(micros: u64) -> u64 {
    micros / 1000
}

// ============================ ops ============================
//
// The vault operations, as free functions over `&Node`. They are deliberately
// independent of Tauri and of rmcp so they can be tested against a real node
// (see the tests at the bottom) without standing up an app or an HTTP server.

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultSummary {
    /// Vault id — pass this as `vault` to every other tool.
    pub id: String,
    pub name: String,
    /// First 6 characters of the id, shown in the app to disambiguate vaults
    /// that share a display name.
    pub hash: String,
    /// True when the vault was joined from a ticket but no peer has yet synced
    /// its contents — reads will come back empty until one does.
    pub pending: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NoteSummary {
    pub path: String,
    /// Last modification time, unix milliseconds.
    pub modified_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchHit {
    pub path: String,
    /// The matching lines, as `{line_number}: {text}`, at most 3 per note.
    pub lines: Vec<String>,
}

async fn op_list_vaults(node: &Node) -> Result<Vec<VaultSummary>> {
    Ok(vault::all_vaults(node)
        .await?
        .into_iter()
        .map(|v| VaultSummary {
            id: v.id,
            name: v.name,
            hash: v.hash,
            pending: v.pending,
        })
        .collect())
}

async fn op_list_notes(
    node: &Node,
    vault: &str,
    folder: Option<&str>,
    modified_since_ms: Option<u64>,
) -> Result<Vec<NoteSummary>> {
    let doc = vault::open(node, vault).await?;
    // An explicit folder under .trash is how a caller opts into seeing trashed
    // notes; otherwise they stay hidden.
    let want_trash = folder.is_some_and(|f| f.trim_start_matches('/').starts_with(TRASH));
    let prefix = folder.map(|f| format!("{}/", f.trim_start_matches('/').trim_end_matches('/')));
    Ok(vault::list_entries(&doc)
        .await?
        .into_iter()
        .filter(|e| want_trash || !vault::is_hidden_path(&e.path))
        .filter(|e| !e.path.ends_with(KEEP) && !e.path.ends_with('/'))
        .filter(|e| prefix.as_ref().is_none_or(|p| e.path.starts_with(p)))
        .filter(|e| modified_since_ms.is_none_or(|since| ms(e.modified_us) >= since))
        .map(|e| NoteSummary {
            path: e.path,
            modified_ms: ms(e.modified_us),
        })
        .collect())
}

/// Read a note's merged text. `None` means no note (or its content hasn't
/// downloaded yet) — callers turn that into a tool error.
async fn op_read_note(node: &Node, vault: &str, path: &str) -> Result<Option<String>> {
    let doc = vault::open(node, vault).await?;
    vault::read_note_text(node, &doc, path.as_bytes()).await
}

/// Thin wrapper over the shared `vault::search` — the in-app search (#15) uses
/// the same implementation, so the two can't drift apart.
async fn op_search_notes(
    node: &Node,
    vault: &str,
    query: &str,
    path_contains: Option<&str>,
    max: usize,
) -> Result<Vec<SearchHit>> {
    Ok(vault::search(node, vault, query, path_contains, max)
        .await?
        .into_iter()
        .map(|h| SearchHit {
            path: h.path,
            lines: h.lines,
        })
        .collect())
}

/// Create a note, de-duplicating the filename against existing siblings the way
/// the app does. Returns the path actually used.
async fn op_create_note(node: &Node, vault: &str, path: &str, content: &str) -> Result<String> {
    let doc = vault::open(node, vault).await?;
    let free = vault::free_key(&doc, path).await?;
    doc.set_bytes(node.author(), free.clone().into_bytes(), vault::fresh_note(content))
        .await?;
    Ok(free)
}

/// Replace a note's whole body, creating it if absent. The current merged text
/// is the merge base, so a peer's concurrent edit elsewhere in the note
/// survives.
async fn op_write_note(node: &Node, vault: &str, path: &str, content: &str) -> Result<()> {
    let doc = vault::open(node, vault).await?;
    let cur = vault::read_note_text(node, &doc, path.as_bytes())
        .await?
        .unwrap_or_default();
    vault::write_note_merged(node, &doc, path.as_bytes(), &cur, content).await
}

/// Replace `old` with `new` in an existing note. Errors rather than guessing
/// when `old` is missing, or when it appears more than once and the caller
/// didn't ask for `replace_all`.
async fn op_edit_note(
    node: &Node,
    vault: &str,
    path: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<usize> {
    if old.is_empty() {
        return Err(anyhow!("old_string must not be empty"));
    }
    let doc = vault::open(node, vault).await?;
    let cur = vault::read_note_text(node, &doc, path.as_bytes())
        .await?
        .ok_or_else(|| anyhow!("note not found (or its content hasn't synced yet): {path}"))?;
    let count = cur.matches(old).count();
    if count == 0 {
        return Err(anyhow!("old_string not found in {path}"));
    }
    if count > 1 && !replace_all {
        return Err(anyhow!(
            "old_string appears {count} times in {path}; pass replace_all or include more context"
        ));
    }
    let next = if replace_all {
        cur.replace(old, new)
    } else {
        cur.replacen(old, new, 1)
    };
    vault::write_note_merged(node, &doc, path.as_bytes(), &cur, &next).await?;
    Ok(if replace_all { count } else { 1 })
}

/// Append to an existing note. Inserts a newline first when the note doesn't
/// already end with one, so appended entries don't run together.
async fn op_append_note(node: &Node, vault: &str, path: &str, text: &str) -> Result<()> {
    let doc = vault::open(node, vault).await?;
    let cur = vault::read_note_text(node, &doc, path.as_bytes())
        .await?
        .ok_or_else(|| anyhow!("note not found (or its content hasn't synced yet): {path}"))?;
    let sep = if cur.is_empty() || cur.ends_with('\n') { "" } else { "\n" };
    let next = format!("{cur}{sep}{text}");
    vault::write_note_merged(node, &doc, path.as_bytes(), &cur, &next).await
}

async fn op_move_note(node: &Node, vault: &str, from: &str, to: &str) -> Result<()> {
    let doc = vault::open(node, vault).await?;
    if vault::read_note_text(node, &doc, to.as_bytes()).await?.is_some() {
        return Err(anyhow!("destination already exists: {to}"));
    }
    vault::rename_key(node, &doc, from, to, false).await
}

/// Soft-delete: move the note under `.trash/` instead of tombstoning it. A real
/// delete propagates to every synced device, so an agent gets the reversible
/// version. Returns the trash path.
async fn op_delete_note(node: &Node, vault: &str, path: &str) -> Result<String> {
    let doc = vault::open(node, vault).await?;
    let dest = vault::free_key(&doc, &format!("{TRASH}/{path}")).await?;
    vault::rename_key(node, &doc, path, &dest, false).await?;
    Ok(dest)
}

async fn op_create_folder(node: &Node, vault: &str, path: &str) -> Result<()> {
    let doc = vault::open(node, vault).await?;
    vault::create_folder_key(node, &doc, path).await
}

// ============================ resources ============================

/// The resource URIs this server serves, parsed from a `vellum://…` string.
enum ResourceUri {
    /// `vellum://vaults`
    Vaults,
    /// `vellum://{vault}/tree`
    Tree(String),
    /// `vellum://{vault}/notes/{path}`
    Note(String, String),
}

impl ResourceUri {
    fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix(SCHEME)?;
        if rest == "vaults" {
            return Some(Self::Vaults);
        }
        let (vault, tail) = rest.split_once('/')?;
        if tail == "tree" {
            return Some(Self::Tree(vault.to_string()));
        }
        let path = tail.strip_prefix("notes/")?;
        (!path.is_empty()).then(|| Self::Note(vault.to_string(), path.to_string()))
    }
}

fn note_uri(vault: &str, path: &str) -> String {
    format!("{SCHEME}{vault}/notes/{path}")
}

fn tree_uri(vault: &str) -> String {
    format!("{SCHEME}{vault}/tree")
}

// ============================ tool arguments ============================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNotesArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Restrict to this folder, e.g. `journal`. Omit for the whole vault.
    /// Trashed notes are hidden unless this names a folder under `.trash`.
    pub folder: Option<String>,
    /// Only notes modified at or after this time (unix milliseconds).
    pub modified_since_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Note path relative to the vault root, e.g. `journal/2026-08-08.md`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Text to find. Case-insensitive substring match — not a regex. A query of
    /// the form `#tag` instead matches that tag exactly: `#work` finds notes
    /// carrying `#work` and not `#workout`, and notes carrying a plain-text
    /// query as a tag are returned first.
    pub query: String,
    /// Only search notes whose path contains this (case-insensitive).
    pub path_contains: Option<String>,
    /// Maximum notes to return. Default 20.
    pub max: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Desired path. If a note already exists there, a numeric suffix is added
    /// and the path actually used is returned.
    pub path: String,
    /// Initial body. Defaults to empty.
    pub content: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    pub path: String,
    /// Exact text to replace. Must appear in the note.
    pub old_string: String,
    /// Replacement text.
    pub new_string: String,
    /// Replace every occurrence. Without this, `old_string` must be unique.
    pub replace_all: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppendNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    pub path: String,
    /// Text to append. A newline is inserted first if the note doesn't end with one.
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    pub path: String,
    /// The note's new full body. Created if it doesn't exist.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VaultArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FolderArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Folder path, e.g. `projects/vellum`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TriageArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Note to watch, e.g. `Backlog.md`. Omit to be told which one to use.
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DailyNoteArgs {
    /// Vault id, from `list_vaults`.
    pub vault: String,
    /// Date as `YYYY-MM-DD`. Defaults to today.
    pub date: Option<String>,
}

// ============================ the server ============================

/// One MCP session. Cloned per connection by the service factory; the app
/// handle is the only real state, plus this session's legacy subscriptions.
#[derive(Clone)]
pub struct Vellum {
    app: AppHandle,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
    /// URIs this session subscribed to via the legacy `resources/subscribe`.
    /// The 2026-07-28 `subscriptions/listen` path carries its own filter and
    /// doesn't use this.
    subscribed: Arc<Mutex<HashSet<String>>>,
    /// Set once the legacy notification pump is running for this session.
    pumping: Arc<Mutex<bool>>,
}

impl Vellum {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            subscribed: Arc::new(Mutex::new(HashSet::new())),
            pumping: Arc::new(Mutex::new(false)),
        }
    }

    /// Live-sync the vault we're about to touch, so an agent-only session (no
    /// window open, background sync off) still converges with peers.
    async fn arm(&self, node: &Node, vault: &str) -> Result<(), ErrorData> {
        let nsid = vault::parse_id(vault).map_err(|e| bad_request(e.to_string()))?;
        vault::arm_vault(&self.app, node, nsid).await.map_err(internal)
    }
}

#[tool_router]
impl Vellum {
    #[tool(description = "List the vaults on this device. Every other tool takes one of these ids as `vault`.")]
    async fn list_vaults(&self) -> Result<Json<Vec<VaultSummary>>, ErrorData> {
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        op_list_vaults(node).await.map(Json).map_err(internal)
    }

    #[tool(
        description = "List note paths in a vault, newest first, with last-modified times. Folders are implicit in the paths."
    )]
    async fn list_notes(
        &self,
        Parameters(args): Parameters<ListNotesArgs>,
    ) -> Result<Json<Vec<NoteSummary>>, ErrorData> {
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_list_notes(node, &args.vault, args.folder.as_deref(), args.modified_since_ms)
            .await
            .map(Json)
            .map_err(internal)
    }

    #[tool(description = "Read a note's Markdown body.")]
    async fn read_note(&self, Parameters(args): Parameters<NoteArgs>) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_read_note(node, &args.vault, &path)
            .await
            .map_err(internal)?
            .ok_or_else(|| bad_request(format!("note not found (or its content hasn't synced yet): {path}")))
    }

    #[tool(
        description = "Search a vault's notes for text. Case-insensitive substring match (not a regex). Returns matching notes with a few matching lines each."
    )]
    async fn search_notes(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<Json<Vec<SearchHit>>, ErrorData> {
        if args.query.trim().is_empty() {
            return Err(bad_request("query is empty"));
        }
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_search_notes(
            node,
            &args.vault,
            &args.query,
            args.path_contains.as_deref(),
            args.max.unwrap_or(20).clamp(1, 100),
        )
        .await
        .map(Json)
        .map_err(internal)
    }

    #[tool(
        description = "Create a new note. If the path is taken, a numeric suffix is added — the path actually used is returned."
    )]
    async fn create_note(
        &self,
        Parameters(args): Parameters<CreateNoteArgs>,
    ) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_create_note(node, &args.vault, &path, args.content.as_deref().unwrap_or(""))
            .await
            .map_err(internal)
    }

    #[tool(
        description = "Replace an exact string in a note. Preferred over write_note for edits — it only touches the region you name, so a concurrent edit from another device survives."
    )]
    async fn edit_note(
        &self,
        Parameters(args): Parameters<EditNoteArgs>,
    ) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        let n = op_edit_note(
            node,
            &args.vault,
            &path,
            &args.old_string,
            &args.new_string,
            args.replace_all.unwrap_or(false),
        )
        .await
        .map_err(|e| bad_request(e.to_string()))?;
        Ok(format!("replaced {n} occurrence(s) in {path}"))
    }

    #[tool(description = "Append text to the end of a note.")]
    async fn append_note(
        &self,
        Parameters(args): Parameters<AppendNoteArgs>,
    ) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_append_note(node, &args.vault, &path, &args.text)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        Ok(format!("appended to {path}"))
    }

    #[tool(
        description = "Replace a note's entire body, creating it if absent. Prefer edit_note or append_note when you only mean to change part of it."
    )]
    async fn write_note(
        &self,
        Parameters(args): Parameters<WriteNoteArgs>,
    ) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_write_note(node, &args.vault, &path, &args.content)
            .await
            .map_err(internal)?;
        Ok(format!("wrote {path}"))
    }

    #[tool(description = "Move or rename a note, preserving its edit history.")]
    async fn move_note(
        &self,
        Parameters(args): Parameters<MoveNoteArgs>,
    ) -> Result<String, ErrorData> {
        let from = clean_note_path(&args.from)?;
        let to = clean_note_path(&args.to)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_move_note(node, &args.vault, &from, &to)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        Ok(format!("moved {from} to {to}"))
    }

    #[tool(
        description = "Delete a note by moving it to the vault's .trash folder. Reversible with move_note."
    )]
    async fn delete_note(&self, Parameters(args): Parameters<NoteArgs>) -> Result<String, ErrorData> {
        let path = clean_note_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        let dest = op_delete_note(node, &args.vault, &path)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        Ok(format!("moved {path} to {dest}"))
    }

    #[tool(
        description = "Create an empty folder. Not needed before creating a note in a new folder — note paths create their folders implicitly."
    )]
    async fn create_folder(
        &self,
        Parameters(args): Parameters<FolderArgs>,
    ) -> Result<String, ErrorData> {
        let path = clean_path(&args.path)?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        self.arm(node, &args.vault).await?;
        op_create_folder(node, &args.vault, &path)
            .await
            .map_err(internal)?;
        Ok(format!("created folder {path}"))
    }
}

#[prompt_router]
impl Vellum {
    /// Add today's entry to the daily journal note.
    #[prompt(name = "daily_note")]
    async fn daily_note_prompt(
        &self,
        Parameters(args): Parameters<DailyNoteArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let date = args.date.unwrap_or_else(|| "today's date".to_string());
        Ok(vec![PromptMessage::new_text(
            Role::User,
            format!(
                "In Vellum vault `{}`, open the journal note for {date} at `journal/{date}.md`. \
                 Use list_notes to check whether it exists: create_note it if not, then append_note \
                 my entry. Ask me what to record before writing anything.",
                args.vault
            ),
        )])
    }

    /// Watch a note and act on requests written into it.
    #[prompt(name = "triage")]
    async fn triage_prompt(
        &self,
        Parameters(args): Parameters<TriageArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        let note = args
            .note
            .unwrap_or_else(|| "the note the user names".to_string());
        Ok(vec![PromptMessage::new_text(
            Role::User,
            format!(
                // A raw literal starting at column 0: the previous version used
                // line continuations that did not survive, so every client was
                // served the source indentation as runs of spaces mid-sentence.
                r#"You are on triage duty for Vellum vault `{vault}`, watching `{note}`.

That note is a work queue: unchecked `- [ ]` items are requests for you, and checked ones are already handled. Subscribe to the note's resource so you are told when it changes.

Judge for yourself when to act. A change notification means the user typed something, NOT that they finished — Vellum saves as you type, so edits arrive while a thought is still half-written. Wait until the note has settled before doing anything, and prefer acting on a whole batch of items over reacting to each keystroke. How long to wait is your call; err toward letting them finish.

Keep the subscription alive. If a request comes back 404 the session is gone — re-initialize and re-subscribe rather than retrying with the old session id. A subscriber that only reconnects the stream goes permanently deaf while still looking healthy, and silently misses everything written after that.

For each unchecked item:
1. Read it in full and work out what is actually being asked. Ask the user if it is ambiguous rather than guessing.
2. Do the work.
3. Tick its checkbox (`- [ ]` -> `- [x]`) with edit_note, so the user can see it was picked up and delete the line. Tick it only once the work is genuinely captured — a tick is a claim that it was handled.

Leave items you did not action unticked, and say which ones you skipped and why."#,
                vault = args.vault,
            ),
        )])
    }

    /// Review recent notes and surface loose ends.
    #[prompt(name = "vault_review")]
    async fn vault_review_prompt(
        &self,
        Parameters(args): Parameters<VaultArgs>,
    ) -> Result<Vec<PromptMessage>, ErrorData> {
        Ok(vec![PromptMessage::new_text(
            Role::User,
            format!(
                "Review Vellum vault `{}`. List the notes modified most recently, read the top \
                 handful, and summarise: what's in progress, what looks abandoned, and any \
                 unfinished thoughts or open questions worth revisiting. Don't modify anything.",
                args.vault
            ),
        )])
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for Vellum {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .enable_resources_subscribe()
                .enable_resources_list_changed()
                .build(),
        );
        info.server_info = Implementation::new("vellum", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(
            "Vellum is a local-first Markdown notes app. A vault is a collection of notes \
             addressed by path (`journal/2026-08-08.md`); folders are implicit in the path. \
             Start with list_vaults to get a vault id. Use edit_note or append_note for changes \
             rather than write_note, so concurrent edits from the user's other devices are \
             preserved. delete_note is a soft delete to .trash. Notes sync peer-to-peer, so a \
             write here reaches the user's other devices on its own. If a request returns \
             404 the session has ended: re-initialize and re-subscribe rather than \
             reusing the old session id, or a subscription will go silently deaf."
                .to_string(),
        );
        info
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        let mut resources = vec![Resource::new(format!("{SCHEME}vaults"), "vaults")
            .with_description("Every vault on this device")
            .with_mime_type("application/json")];
        for v in vault::all_vaults(node).await.map_err(internal)? {
            resources.push(
                Resource::new(tree_uri(&v.id), format!("{} — tree", v.name))
                    .with_description("Folder tree of this vault")
                    .with_mime_type("application/json"),
            );
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new(format!("{SCHEME}{{vault}}/notes/{{path}}"), "note")
                .with_description("A single note's Markdown body")
                .with_mime_type("text/markdown"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = ResourceUri::parse(&request.uri)
            .ok_or_else(|| bad_request(format!("unknown resource: {}", request.uri)))?;
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        let contents = match uri {
            ResourceUri::Vaults => {
                let vaults = op_list_vaults(node).await.map_err(internal)?;
                json_contents(&request.uri, &vaults)?
            }
            ResourceUri::Tree(vault) => {
                self.arm(node, &vault).await?;
                let doc = vault::open(node, &vault).await.map_err(internal)?;
                let tree = vault::build_tree(&doc).await.map_err(internal)?;
                json_contents(&request.uri, &tree)?
            }
            ResourceUri::Note(vault, path) => {
                self.arm(node, &vault).await?;
                let text = op_read_note(node, &vault, &path)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| bad_request(format!("note not found: {path}")))?;
                ResourceContents::text(text, &request.uri).with_mime_type("text/markdown")
            }
        };
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    /// The 2026-07-28 subscription path: accept resource subscriptions and the
    /// resource-list-changed category, and nothing else.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        let mut accepted = SubscriptionFilter::new();
        accepted.resources_list_changed = requested.resources_list_changed;
        accepted.resource_subscriptions = requested.resource_subscriptions.clone();
        Some(accepted)
    }

    /// Run one `subscriptions/listen` stream: forward vault mutations as
    /// `resources/updated` until the client cancels.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        // Arm every vault, otherwise a vault nobody has opened emits nothing.
        for v in vault::all_vaults(node).await.map_err(internal)? {
            let _ = self.arm(node, &v.id).await;
        }
        let mut rx = node.subscribe_changes();
        let watched: HashSet<String> = context
            .accepted()
            .resource_subscriptions
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let list_changed = context.accepted().resources_list_changed == Some(true);
        loop {
            // Cancellation stays responsive while waiting out the quiet period.
            let batch = tokio::select! {
                _ = context.cancelled() => return Ok(()),
                b = next_batch(&mut rx) => match b {
                    Some(b) => b,
                    None => return Ok(()),
                },
            };
            for uri in batch.uris {
                if watched.contains(&uri) {
                    let _ = context.sink().notify_resource_updated(uri).await;
                }
            }
            if list_changed && batch.list_changed {
                let _ = context.sink().notify_resource_list_changed().await;
            }
        }
    }

    /// Legacy (`< 2026-07-28`) subscription path. Same source of truth as
    /// `listen`, but the client subscribes URI by URI and we push through the
    /// session's peer handle.
    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if ResourceUri::parse(&request.uri).is_none() {
            return Err(bad_request(format!("unknown resource: {}", request.uri)));
        }
        self.subscribed.lock().unwrap().insert(request.uri.clone());

        // One pump per session, started lazily on the first subscribe.
        let start = {
            let mut pumping = self.pumping.lock().unwrap();
            let first = !*pumping;
            *pumping = true;
            first
        };
        if !start {
            return Ok(());
        }
        let mgr = self.app.state::<VaultManager>();
        let node = mgr.node().await.map_err(internal)?;
        for v in vault::all_vaults(node).await.map_err(internal)? {
            let _ = self.arm(node, &v.id).await;
        }
        let mut rx = node.subscribe_changes();
        let subscribed = self.subscribed.clone();
        let peer: Peer<RoleServer> = context.peer.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let Some(batch) = next_batch(&mut rx).await else {
                    return;
                };
                for uri in batch.uris {
                    if subscribed.lock().unwrap().contains(&uri) {
                        // A send error means the session is gone — stop pumping.
                        let param = ResourceUpdatedNotificationParam::new(uri);
                        if peer.notify_resource_updated(param).await.is_err() {
                            return;
                        }
                    }
                }
                if batch.list_changed && peer.notify_resource_list_changed().await.is_err() {
                    return;
                }
            }
        });
        Ok(())
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.subscribed.lock().unwrap().remove(&request.uri);
        Ok(())
    }
}

/// How long the notification pump waits for quiet before sending (#194).
///
/// A note's autosave fires on every keystroke, so without this a subscriber
/// receives one `resources/updated` per character — and each one invites a
/// re-read of the whole note, which is a CRDT merge. This window exists purely
/// to stop that waste.
///
/// It is deliberately SHORT, and deliberately not a policy. How long to wait
/// before acting on a change — whether the user has finished writing a request,
/// whether to batch several notes together — is the agent's judgement, not the
/// server's. The `triage` prompt describes that job; a server-side timeout
/// would only impose one answer on every client. Live collaboration will want
/// events sooner still, and the underlying broadcast already stays immediate
/// for the editor's own rebase.
const NOTIFY_QUIET: Duration = Duration::from_millis(600);

/// A settled batch of changes: what to tell the client after typing stopped.
struct Batch {
    /// Resource URIs to report, de-duplicated across the whole burst.
    uris: HashSet<String>,
    /// Whether the vault's listing changed (a note added or removed).
    list_changed: bool,
}

/// Collect changes until the stream goes quiet, then return them as one batch.
///
/// Returns `None` when the channel closes. A lagged receiver is reported
/// immediately rather than waited out — it already means "you missed changes",
/// so holding it back only makes the client staler.
///
/// Deliberately NOT a debounce on the underlying broadcast: that stays
/// immediate, because the editor's own rebase depends on it and live
/// collaboration will want per-keystroke events.
async fn next_batch(rx: &mut tokio::sync::broadcast::Receiver<VaultChange>) -> Option<Batch> {
    let mut batch = Batch {
        uris: HashSet::new(),
        list_changed: false,
    };
    // Block for the first change — no timer until something actually happens.
    match rx.recv().await {
        Ok(change) => absorb(&mut batch, change),
        Err(RecvError::Lagged(_)) => {
            batch.list_changed = true;
            return Some(batch);
        }
        Err(RecvError::Closed) => return None,
    }
    // Then keep absorbing until the stream is quiet for NOTIFY_QUIET.
    loop {
        match tokio::time::timeout(NOTIFY_QUIET, rx.recv()).await {
            Ok(Ok(change)) => absorb(&mut batch, change),
            Ok(Err(RecvError::Lagged(_))) => {
                batch.list_changed = true;
                return Some(batch);
            }
            Ok(Err(RecvError::Closed)) => return Some(batch),
            // Quiet period elapsed — the user stopped typing.
            Err(_) => return Some(batch),
        }
    }
}

fn absorb(batch: &mut Batch, change: VaultChange) {
    // A new or removed note changes the vault's listing.
    if change.path.is_some() {
        batch.list_changed = true;
    }
    batch.uris.extend(changed_uris(&change));
}

/// The resource URIs a vault mutation invalidates. A change always invalidates
/// the vault's tree; when the event carried an entry key it also invalidates
/// that note.
fn changed_uris(change: &VaultChange) -> Vec<String> {
    let vault = change.vault.to_string();
    let mut uris = vec![tree_uri(&vault)];
    if let Some(path) = &change.path {
        uris.push(note_uri(&vault, path));
    }
    uris
}

fn json_contents<T: Serialize>(uri: &str, value: &T) -> Result<ResourceContents, ErrorData> {
    let text = serde_json::to_string_pretty(value).map_err(internal)?;
    Ok(ResourceContents::text(text, uri).with_mime_type("application/json"))
}

// ============================ transport ============================

/// Reject anything without the right bearer token. The comparison is
/// constant-time so a wrong token can't be recovered by timing repeated
/// attempts. Loopback binding already keeps remote hosts out; this keeps *other
/// local processes* out, which is the real threat on a shared machine.
async fn require_bearer(
    AxumState(token): AxumState<Arc<str>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if ct_eq(presented.as_bytes(), token.as_bytes()) {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Constant-time byte comparison. Length is not secret (the token length is
/// fixed and public), so an early length check is fine.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Running-server state, managed by Tauri.
pub struct McpServer {
    dir: PathBuf,
    state: Mutex<Running>,
}

#[derive(Default)]
struct Running {
    cancel: Option<CancellationToken>,
    port: Option<u16>,
}

/// What the Settings UI needs to show and to hand to a client.
#[derive(Serialize, Clone)]
pub struct McpStatus {
    pub enabled: bool,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub token: String,
    /// Ready-to-paste command for adding this server to Claude Code.
    pub command: Option<String>,
}

impl McpServer {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: Mutex::new(Running::default()),
        }
    }
}

fn status_of(dir: &Path, port: Option<u16>) -> McpStatus {
    let cfg = load_config(dir);
    let url = port.map(|p| format!("http://127.0.0.1:{p}/mcp"));
    McpStatus {
        enabled: port.is_some(),
        command: url.as_ref().map(|u| {
            format!(
                "claude mcp add --transport http vellum {u} --header \"Authorization: Bearer {}\"",
                cfg.token
            )
        }),
        url,
        token: cfg.token,
        port,
    }
}

pub fn status(app: &AppHandle) -> McpStatus {
    let server = app.state::<McpServer>();
    let port = server.state.lock().unwrap().port;
    status_of(&server.dir, port)
}

/// Bind the preferred port, falling back to an ephemeral one if it's taken — a
/// stale `mcp.json` port must never stop the server from starting.
async fn bind(preferred: u16) -> std::io::Result<tokio::net::TcpListener> {
    if preferred != 0 {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, preferred));
        if let Ok(l) = tokio::net::TcpListener::bind(addr).await {
            return Ok(l);
        }
        tracing::info!("[mcp] port {preferred} unavailable; taking an ephemeral one");
    }
    tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
}

/// Start the server if it isn't already running. Returns the bound port.
pub async fn start(app: &AppHandle) -> Result<u16, String> {
    let server = app.state::<McpServer>();
    if let Some(port) = server.state.lock().unwrap().port {
        return Ok(port);
    }
    let mut cfg = load_config(&server.dir);
    let listener = bind(cfg.port).await.map_err(|e| format!("could not bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();

    let cancel = CancellationToken::new();
    let token: Arc<str> = Arc::from(cfg.token.as_str());
    let handle = app.clone();
    let service = StreamableHttpService::new(
        move || Ok(Vellum::new(handle.clone())),
        Arc::new(LocalSessionManager::default()),
        {
            let mut config = StreamableHttpServerConfig::default();
            // Cancelling this token drops every live session when the user
            // turns the server off.
            config.cancellation_token = cancel.clone();
            config
        },
    );
    let router = Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token, require_bearer));

    let shutdown = cancel.clone();
    tauri::async_runtime::spawn(async move {
        let serve = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await });
        if let Err(e) = serve.await {
            tracing::error!("[mcp] server stopped: {e}");
        }
    });

    {
        let mut state = server.state.lock().unwrap();
        state.cancel = Some(cancel);
        state.port = Some(port);
    }
    // Remember the port (so a copied connect command keeps working) and that we
    // are enabled (so a relaunch restarts us).
    cfg.port = port;
    cfg.enabled = true;
    save_config(&server.dir, &cfg);
    tracing::info!("[mcp] listening on 127.0.0.1:{port}");
    Ok(port)
}

/// Stop the server. Cancelling the token drops the listener and terminates the
/// live sessions.
pub fn stop(app: &AppHandle) {
    let server = app.state::<McpServer>();
    let cancel = {
        let mut state = server.state.lock().unwrap();
        state.port = None;
        state.cancel.take()
    };
    if let Some(cancel) = cancel {
        cancel.cancel();
        tracing::info!("[mcp] stopped");
    }
    let mut cfg = load_config(&server.dir);
    cfg.enabled = false;
    save_config(&server.dir, &cfg);
}

pub async fn set_enabled(app: &AppHandle, enabled: bool) -> Result<McpStatus, String> {
    if enabled {
        start(app).await?;
    } else {
        stop(app);
    }
    Ok(status(app))
}

/// Restart the server on launch if it was left on, mirroring how background
/// sync re-arms itself.
pub fn start_if_enabled(app: &AppHandle) {
    let server = app.state::<McpServer>();
    if !load_config(&server.dir).enabled {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = start(&app).await {
            tracing::error!("[mcp] could not start on launch: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::vault::init;

    /// A fresh node + empty vault in a temp dir, named for the calling test so
    /// concurrent tests don't share state.
    async fn fixture(name: &str) -> (Node, iroh_docs::api::Doc, String, PathBuf) {
        let dir = std::env::temp_dir().join(format!("vellum-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let node = init(dir.clone()).await.expect("node");
        let doc = node.docs().create().await.expect("create vault");
        let id = doc.id().to_string();
        (node, doc, id, dir)
    }

    #[test]
    fn clean_path_normalizes_and_rejects() {
        assert_eq!(clean_path("notes/a.md").unwrap(), "notes/a.md");
        // Leading slashes are agent noise, not an error.
        assert_eq!(clean_path("/notes/a.md").unwrap(), "notes/a.md");
        assert_eq!(clean_path("  /a.md  ").unwrap(), "a.md");
        for bad in ["", "/", "../secrets.md", "a/../../b.md", "a//b.md", "a\\b.md", "dir/"] {
            assert!(clean_path(bad).is_err(), "should reject {bad:?}");
        }
        assert!(clean_path("a\0b").is_err());
        // The .md suffix is added only by the note variant.
        assert_eq!(clean_note_path("journal/today").unwrap(), "journal/today.md");
        assert_eq!(clean_note_path("journal/today.md").unwrap(), "journal/today.md");
    }

    #[test]
    fn token_comparison_rejects_wrong_and_absent() {
        let token = new_token();
        assert_eq!(token.len(), 64, "32 random bytes, hex encoded");
        assert!(ct_eq(token.as_bytes(), token.as_bytes()));
        assert!(!ct_eq(b"", token.as_bytes()));
        assert!(!ct_eq(b"wrong", token.as_bytes()));
        // Same length, one bit different.
        let mut other = token.clone().into_bytes();
        other[0] ^= 1;
        assert!(!ct_eq(&other, token.as_bytes()));
        assert_ne!(new_token(), new_token(), "tokens must not repeat");
    }

    /// A burst of edits — one per keystroke — must settle into a single
    /// notification per URI, not one per character (#194).
    #[tokio::test(start_paused = true)]
    async fn notifications_coalesce_until_typing_stops() {
        use iroh_docs::NamespaceId;
        let (tx, mut rx) = tokio::sync::broadcast::channel(64);
        let vault = NamespaceId::from(&[7u8; 32]);

        // Ten "keystrokes" on the same note, faster than the quiet window.
        for _ in 0..10 {
            tx.send(VaultChange {
                vault,
                path: Some("notes/a.md".into()),
            })
            .unwrap();
        }
        let batch = next_batch(&mut rx).await.expect("batch");
        assert_eq!(
            batch.uris.len(),
            2,
            "one note URI + the vault's tree URI, however many keystrokes: {:?}",
            batch.uris
        );
        assert!(batch.list_changed, "an insert changes the listing");

        // Two different notes in one burst still settle together, one URI each.
        for p in ["notes/a.md", "notes/b.md"] {
            tx.send(VaultChange {
                vault,
                path: Some(p.into()),
            })
            .unwrap();
        }
        let batch = next_batch(&mut rx).await.expect("batch");
        assert_eq!(batch.uris.len(), 3, "two notes + the tree: {:?}", batch.uris);

        // A closed channel ends the pump rather than spinning.
        drop(tx);
        assert!(next_batch(&mut rx).await.is_none());
    }

    #[test]
    fn resource_uris_round_trip() {
        assert!(matches!(
            ResourceUri::parse("vellum://vaults"),
            Some(ResourceUri::Vaults)
        ));
        let uri = note_uri("abc123", "journal/a.md");
        match ResourceUri::parse(&uri) {
            Some(ResourceUri::Note(v, p)) => {
                assert_eq!(v, "abc123");
                assert_eq!(p, "journal/a.md");
            }
            other => panic!("expected a note uri, got {}", other.is_some()),
        }
        match ResourceUri::parse(&tree_uri("abc123")) {
            Some(ResourceUri::Tree(v)) => assert_eq!(v, "abc123"),
            _ => panic!("expected a tree uri"),
        }
        assert!(ResourceUri::parse("http://example.com").is_none());
        assert!(ResourceUri::parse("vellum://abc/notes/").is_none());
    }

    #[tokio::test]
    async fn create_edit_append_read_round_trip() {
        let (node, _doc, vault, dir) = fixture("crud").await;

        let path = op_create_note(&node, &vault, "journal/a.md", "hello\n")
            .await
            .expect("create");
        assert_eq!(path, "journal/a.md");
        assert_eq!(
            op_read_note(&node, &vault, &path).await.unwrap().as_deref(),
            Some("hello\n")
        );

        op_edit_note(&node, &vault, &path, "hello", "goodbye", false)
            .await
            .expect("edit");
        op_append_note(&node, &vault, &path, "tail")
            .await
            .expect("append");
        assert_eq!(
            op_read_note(&node, &vault, &path).await.unwrap().as_deref(),
            Some("goodbye\ntail")
        );

        // A second create at the same path de-duplicates rather than clobbering.
        let dup = op_create_note(&node, &vault, "journal/a.md", "").await.unwrap();
        assert_ne!(dup, path, "existing note must not be overwritten");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_note_reports_missing_and_ambiguous_matches() {
        let (node, _doc, vault, dir) = fixture("edit-errors").await;
        let path = op_create_note(&node, &vault, "a.md", "x\nx\n").await.unwrap();

        let missing = op_edit_note(&node, &vault, &path, "nope", "y", false).await;
        assert!(missing.unwrap_err().to_string().contains("not found"));

        let ambiguous = op_edit_note(&node, &vault, &path, "x", "y", false).await;
        assert!(ambiguous.unwrap_err().to_string().contains("2 times"));

        // replace_all resolves it and reports the count.
        let n = op_edit_note(&node, &vault, &path, "x", "y", true).await.unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            op_read_note(&node, &vault, &path).await.unwrap().as_deref(),
            Some("y\ny\n")
        );

        // Editing a note that doesn't exist at all is an error, not a create.
        assert!(op_edit_note(&node, &vault, "ghost.md", "a", "b", false)
            .await
            .is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The #99 guarantee, at the MCP layer: an agent edit must not discard a
    /// concurrent edit made elsewhere in the same note. `op_edit_note` reads the
    /// current merged text as its base, so the other edit survives.
    #[tokio::test]
    async fn edit_preserves_a_concurrent_edit() {
        let (node, doc, vault, dir) = fixture("merge").await;
        let path = op_create_note(&node, &vault, "shared.md", "L1\nL2\n")
            .await
            .unwrap();

        // Someone else (the editor, or a peer) appends a line...
        vault::write_note_merged(&node, &doc, path.as_bytes(), "L1\nL2\n", "L1\nL2\nfrom-editor\n")
            .await
            .expect("editor edit");
        // ...and the agent edits a different region.
        op_edit_note(&node, &vault, &path, "L1", "L1-agent", false)
            .await
            .expect("agent edit");

        let text = op_read_note(&node, &vault, &path).await.unwrap().unwrap();
        assert!(text.contains("L1-agent"), "agent edit lost: {text:?}");
        assert!(text.contains("from-editor"), "concurrent edit clobbered: {text:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_moves_to_trash_and_hides_it() {
        let (node, _doc, vault, dir) = fixture("trash").await;
        let path = op_create_note(&node, &vault, "a.md", "body").await.unwrap();

        let dest = op_delete_note(&node, &vault, &path).await.expect("delete");
        assert_eq!(dest, ".trash/a.md");
        assert!(
            op_read_note(&node, &vault, &path).await.unwrap().is_none(),
            "original path should be gone"
        );
        assert_eq!(
            op_read_note(&node, &vault, &dest).await.unwrap().as_deref(),
            Some("body"),
            "content must survive the move"
        );

        // Trashed notes are hidden from a normal listing, visible when asked for.
        let listed = op_list_notes(&node, &vault, None, None).await.unwrap();
        assert!(listed.iter().all(|n| !n.path.starts_with(TRASH)));
        let trashed = op_list_notes(&node, &vault, Some(TRASH), None).await.unwrap();
        assert_eq!(trashed.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_finds_matches_with_line_numbers() {
        let (node, _doc, vault, dir) = fixture("search").await;
        op_create_note(&node, &vault, "one.md", "alpha\nbeta\ngamma\n")
            .await
            .unwrap();
        op_create_note(&node, &vault, "two.md", "nothing here\n")
            .await
            .unwrap();
        op_create_note(&node, &vault, "notes/three.md", "BETA rising\n")
            .await
            .unwrap();

        let hits = op_search_notes(&node, &vault, "beta", None, 20).await.unwrap();
        assert_eq!(hits.len(), 2, "case-insensitive across both notes");
        let one = hits.iter().find(|h| h.path == "one.md").expect("one.md");
        assert_eq!(one.lines, vec!["2: beta"], "1-based line numbers");

        // path_contains narrows to a folder.
        let scoped = op_search_notes(&node, &vault, "beta", Some("notes/"), 20)
            .await
            .unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].path, "notes/three.md");

        // Trashed notes drop out of search.
        op_delete_note(&node, &vault, "one.md").await.unwrap();
        let after = op_search_notes(&node, &vault, "beta", None, 20).await.unwrap();
        assert_eq!(after.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn move_note_preserves_content_and_refuses_to_overwrite() {
        let (node, _doc, vault, dir) = fixture("move").await;
        op_create_note(&node, &vault, "a.md", "body").await.unwrap();
        op_create_note(&node, &vault, "b.md", "other").await.unwrap();

        op_move_note(&node, &vault, "a.md", "moved/a.md").await.expect("move");
        assert_eq!(
            op_read_note(&node, &vault, "moved/a.md").await.unwrap().as_deref(),
            Some("body")
        );
        assert!(op_read_note(&node, &vault, "a.md").await.unwrap().is_none());

        let clash = op_move_note(&node, &vault, "b.md", "moved/a.md").await;
        assert!(clash.is_err(), "must not silently overwrite the destination");
        assert_eq!(
            op_read_note(&node, &vault, "moved/a.md").await.unwrap().as_deref(),
            Some("body"),
            "destination content must be intact after a refused move"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_note_creates_then_replaces() {
        let (node, _doc, vault, dir) = fixture("write").await;
        op_write_note(&node, &vault, "new.md", "first").await.expect("create");
        assert_eq!(
            op_read_note(&node, &vault, "new.md").await.unwrap().as_deref(),
            Some("first")
        );
        op_write_note(&node, &vault, "new.md", "second").await.expect("replace");
        assert_eq!(
            op_read_note(&node, &vault, "new.md").await.unwrap().as_deref(),
            Some("second")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Seed a vault with a few notes into a chosen data dir, for manual
    /// end-to-end testing of the server against a real app *without* touching
    /// your own vaults. Run with:
    ///   VELLUM_DATA_DIR=/tmp/e2e/Library/Application\ Support/com.andymitch.vellum \
    ///     cargo test seed_smoke_vault -- --ignored --nocapture
    /// then launch the app with HOME=/tmp/e2e.
    #[tokio::test]
    #[ignore]
    async fn seed_smoke_vault() {
        let dir = PathBuf::from(std::env::var("VELLUM_DATA_DIR").expect("set VELLUM_DATA_DIR"));
        std::fs::create_dir_all(&dir).expect("create data dir");
        let node = init(dir).await.expect("node");
        let doc = node.docs().create().await.expect("create vault");
        let vault = doc.id().to_string();
        // Give it a name the way create_vault does, so it isn't "pending".
        doc.set_bytes(
            node.author(),
            b"\x00meta/name".to_vec(),
            {
                let mut v = vec![0x01u8];
                v.extend_from_slice(b"Smoke Test");
                v
            },
        )
        .await
        .expect("name");
        op_create_note(&node, &vault, "journal/2026-08-08.md", "# Today\n\nran the smoke test\n")
            .await
            .unwrap();
        op_create_note(&node, &vault, "ideas.md", "- ship the MCP server\n")
            .await
            .unwrap();
        // This process is about to exit, milliseconds after the last write. The
        // docs actor commits to redb on its own schedule, so give it a moment to
        // land before flushing blobs and dropping everything — otherwise the
        // final note is silently missing when the app next opens the dir. (The
        // app itself doesn't need this: it stays alive long after a write, and
        // its writes survive even an abrupt kill.)
        tokio::time::sleep(Duration::from_secs(2)).await;
        node.flush_blobs().await;
        println!("VAULT={vault}");
    }

    /// Print every key in the vaults of a data dir, for debugging a smoke run.
    /// The app must not be running against that dir (the store is single-writer).
    ///   VELLUM_DATA_DIR=… cargo test dump_smoke_vault -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn dump_smoke_vault() {
        let dir = PathBuf::from(std::env::var("VELLUM_DATA_DIR").expect("set VELLUM_DATA_DIR"));
        let node = init(dir).await.expect("node");
        for v in vault::all_vaults(&node).await.expect("vaults") {
            println!("vault {} ({})", v.id, v.name);
            let doc = vault::open(&node, &v.id).await.expect("open");
            for e in vault::list_entries(&doc).await.expect("entries") {
                let body = vault::read_note_text(&node, &doc, e.path.as_bytes())
                    .await
                    .expect("read");
                println!("  {:?} -> {:?}", e.path, body);
            }
        }
    }

    #[tokio::test]
    async fn list_notes_filters_by_folder_and_time() {
        let (node, _doc, vault, dir) = fixture("list").await;
        op_create_note(&node, &vault, "top.md", "").await.unwrap();
        op_create_note(&node, &vault, "sub/a.md", "").await.unwrap();
        op_create_folder(&node, &vault, "empty").await.unwrap();

        let all = op_list_notes(&node, &vault, None, None).await.unwrap();
        assert_eq!(all.len(), 2, ".keep markers are not notes: {all:?}");

        let sub = op_list_notes(&node, &vault, Some("sub"), None).await.unwrap();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].path, "sub/a.md");

        // Everything was just written, so a future cutoff excludes it all.
        let future = all[0].modified_ms + 60_000;
        assert!(op_list_notes(&node, &vault, None, Some(future))
            .await
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
