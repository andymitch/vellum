# How a browser vault survives a reload

Storage design for the browser build, and the options weighed getting there.
Everything sequenced below is implemented, in `crates/vellum-wasm`.

The section that still describes the shipped code is **"What is still crude"**
at the end: content is copied to a durable store after each mutation rather than
living in a real `iroh-blobs` store on OPFS, and one tab per vault is the rule.

The spike proves a browser can be a real peer (see README). This is the other
half: whether it can keep the notes.

**It can — steps 1–3 below are implemented and passing** :
notes written in one session, with the tab then closed, are read back in the
next session from OPFS with **no peer involved**, content included. Edits
supersede cleanly, superseded content is collected, the device keeps one
identity across reloads, and a second tab on the same vault is turned away with
a sentence rather than an OPFS error. The rest of this document is the design
that got there, and what is left.

## What has to persist

| What | Today (native) | Size | Notes |
| --- | --- | --- | --- |
| Replica (entries, authors, signatures, namespaces) | `docs.redb` file | small, many small writes | The authority for what exists. Loses history if rebuilt. |
| Blob content (note bodies as yrs state) | `blobs/` via `FsStore` | one blob per saved version | Content-addressed and immutable — the easy half. |
| Endpoint secret | `endpoint-secret` | 32 B | This device's stable identity. Losing it makes peers treat us as a new device. |
| Author key | `default-author` | 32 B | Authorship of our own edits. |
| Vault list + local names | `vault-names.json`, `peers.json` | tiny | Already localStorage-shaped. |

The two 32-byte keys turned out not to need IndexedDB at all: they sit in a
`meta` table in the same sidecar database as the content (see Option A), which
keeps identity and replica in one place — a device identity that disagrees with
the replica beside it is worse than none. The vault list is still
localStorage-shaped and untouched here. The interesting ones remain the first
two.

## The upstream hook this needs

**Confirmed, and patched locally** — see `crates/iroh-docs-vellum`, vendored and
pointed at by `[patch.crates-io]` in `crates/vellum-wasm/Cargo.toml`. It is
exactly the ~5 lines predicted below, because `Engine::spawn` already accepts a
`Store`; nothing else upstream had to move.

A second, smaller gap turned up while wiring the author key: `DefaultAuthorStorage`
offers only `Mem` and `Persistent(PathBuf)`, with no way to hand it an author we
already have. So every boot mints a throwaway author into the (persisted) author
table, and the browser side has to set its own author as default and delete the
throwaway to stop the table growing by a key per launch. Worth raising with n0
alongside the constructor below, though unlike that one it is a papercut rather
than a blocker.

`iroh_docs::store::fs::Store` builds redb itself:

```rust
pub fn memory() -> Self {
    let db = Database::builder().create_with_backend(redb::backends::InMemoryBackend::new())?;
    Self::new_impl(db)               // new_impl is private
}
pub fn persistent(path: impl AsRef<std::path::Path>) -> Result<Self> { … }
```

So the only two shapes offered are "a file" (impossible in a browser) and "a
`Vec<u8>` we cannot reach". **Every option below needs the same small upstream
addition** — a public constructor taking a `redb::StorageBackend` (or a
`redb::Database`), which `new_impl` already is behind a private door. That is a
~5-line PR to iroh-docs; until it merges, `[patch.crates-io]` against a fork
keeps us moving. Worth confirming with n0 before building on it.

## Option A — redb on OPFS (chosen; implemented in `crates/vellum-wasm/src/opfs.rs`)

Implement `redb::StorageBackend` over an OPFS
`FileSystemSyncAccessHandle`. The two interfaces line up almost exactly:

| redb `StorageBackend` | OPFS sync access handle |
| --- | --- |
| `len()` | `getSize()` |
| `read(offset, len)` | `read(buf, { at })` |
| `write(offset, data)` | `write(buf, { at })` |
| `set_len(len)` | `truncate(len)` |
| `sync_data()` | `flush()` |

Both sides are synchronous, which is what redb requires and what makes this
work at all. This is the same technique SQLite's WASM build uses for its OPFS
VFS, so it is well-trodden.

Consequences:

- **Sync access handles are worker-only**, so the vault node moves into a
  dedicated Web Worker. That is a plus regardless: iroh's browser build is
  single-threaded, and CRDT merges plus blob hashing on the main thread would
  jank the editor.
- Durability comes from redb, not from us: no whole-database rewrites, and a
  crash mid-write is redb's problem to recover, which it is designed for.
- Blobs stay in `MemStore`, backed by a `hash -> bytes` table in a **second
  redb database** beside the replica (`<vault>.store`), with `MemStore` as the
  serving layer. Safe because blob content is immutable and content-addressed:
  re-adding the bytes reproduces the same hashes. Not an `iroh-blobs` store —
  that crate has no store trait (stores are actors behind an irpc channel), so a
  real OPFS blob store means implementing partial blobs, bao trees, range
  requests and tags, which belongs upstream.

  Boot reads the content hashes off the replica's live entries and loads exactly
  those, then drops everything else — so startup is proportional to the vault,
  not to its history, and superseded versions do not accumulate. That same
  database's `meta` table holds the endpoint secret and author key.
- Baseline: OPFS sync access handles need Safari 16.4+ / Chrome 108+, which is
  the baseline the PWA already assumes.

## Option B — snapshot the in-memory database

Keep the in-memory backend, but own the `Vec<u8>` and write it to IndexedDB on a
debounce.

Simpler to write, worse to run: every snapshot rewrites the whole database, a
crash between snapshots loses recent edits, and it needs the *same* upstream
hook as Option A (an `InMemoryBackend` we cannot read is no use). Given the hook
is required either way, Option A is strictly better — this is only worth doing
as a stepping stone if the OPFS backend proves troublesome.

## Option C — no replica persistence, rehydrate from peers

Store only the namespace secret plus a local copy of note text; on launch, build
a fresh replica and re-sync from a peer.

Rejected unless A and B both fail: it needs a peer online to be whole, so the
first offline launch shows an incomplete vault, and it keeps two sources of
truth for note text — the failure mode that produced #167.

## Status

Option A, sequenced so each step is independently checkable:

1. **Done.** Upstream constructor confirmed and patched locally (`patches/`).
2. **Done.** redb `StorageBackend` over OPFS, in a worker. *Checked by*: write
   notes, close the tab, reopen the same file — entries are there, no peer.
3. **Done.** Note content survives, in a hash-keyed store beside the replica.
   *Checked by*: the reopened vault reads note **values**, not just keys; a
   re-saved identical value stores nothing new; and the next boot collects the
   version an edit superseded.
4. **Done.** This device keeps one identity across reloads — endpoint secret and
   author key in the same store. *Checked by*: three sessions on one vault file
   report the same endpoint id, so peers see one device rather than three
   strangers, and the author table still holds exactly one key.
5. **Done.** One writer per vault, refused in words. A Web Lock taken before the
   OPFS handle turns the second tab's `NoModificationAllowedError` from deep
   inside redb into a sentence a user can act on. *Checked by*: a second tab on
   an open vault is rejected with "already open in another tab".
6. **Done.** The node runs in the worker behind a message bridge whose messages
   are the `VaultBackend` commands — the seam already in PR #235. The bridge is
   Rust (`src/bridge.rs`); `web/worker.js` is down to five lines of JavaScript,
   which is the floor (see "How much of it is Rust" in the README). *Checked by*:
   every command the persistence test issues goes through it.
7. Split `vault.rs` into portable core and platform shims (`FsStore`, `notify`,
   mDNS, MCP, linked folders, tray stay desktop-only), and export the command
   surface through `wasm-bindgen`.
8. `vault-wasm.ts` behind `VaultBackend`; keep `zip.ts` for export/import; drop
   `vault-web.ts`.

## What is still crude

Things worth fixing, in rough order of how much they will bite:

- **Entry and content are still two writes**, not one transaction. Content goes
  first now, so the surviving failure is an orphaned blob — which the next
  boot's sweep collects — rather than an entry whose content is missing until a
  peer supplies it. Good enough to stop being a correctness problem; a port that
  wants the pair atomic needs them in one database, which means the upstream
  blob store, not this sidecar.
- **The whole blob is held in memory** on both the write and the read path, and
  `MemStore` holds every live note's content at once. Fine for notes; wrong the
  moment vaults carry attachments. Wants a real OPFS `iroh-blobs` store.
- **One writer per vault.** The Web Lock makes the refusal legible, but two tabs
  on one vault still cannot both work. The way out is a SharedWorker owning the
  node with the tabs as clients, and the baseline allows it: SharedWorker landed
  in Safari 16, which is *older* than the 16.4 the OPFS sync access handle
  already requires. So this is work, not a wall — the spike simply stops at
  telling the second tab the truth.
- **The blob store never forgets on demand.** `retain` runs at boot only, so a
  long session accumulates superseded versions until the next reload.

## Risks to keep in view

**Writes are not durable when they return.** The docs actor batches entries into
one redb transaction and commits after 500 ms of idle, or on a graceful
shutdown. A desktop process gets that shutdown; a browser tab does not — it can
be closed, discarded while backgrounded, or killed by the OS with no chance to
run async cleanup. This spike lost an edit to exactly that, and the fix is the
`flush` command: durability in the browser has to be asked for at the save
boundary rather than inherited from a clean exit. `pagehide` is a last-chance
backstop, not a guarantee, since it is not given time to finish async work. (The
content store needs no equivalent — it commits per write.)

Beyond that: the 2.9 MB (gzipped) module has to download and compile before the
app works on a phone; iOS still evicts storage for a site that is not installed,
so "Add to Home Screen" stays load-bearing; sync from a browser is relay-only,
so no LAN-speed transfers and no mDNS; and the upstream patch is a dependency we
carry until n0 takes it (or forever, if they would rather not).
