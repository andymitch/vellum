# Making a browser vault survive a reload

The spike proves a browser can be a real peer (see README). This is the other
half: whether it can keep the notes.

**It can — steps 1–3 below are implemented and passing** (`persistence.mjs`):
notes written in one session, with the tab then closed, are read back in the
next session from OPFS with **no peer involved**, content included. The rest of
this document is the design that got there, and what is left.

## What has to persist

| What | Today (native) | Size | Notes |
| --- | --- | --- | --- |
| Replica (entries, authors, signatures, namespaces) | `docs.redb` file | small, many small writes | The authority for what exists. Loses history if rebuilt. |
| Blob content (note bodies as yrs state) | `blobs/` via `FsStore` | one blob per saved version | Content-addressed and immutable — the easy half. |
| Endpoint secret | `endpoint-secret` | 32 B | This device's stable identity. Losing it makes peers treat us as a new device. |
| Author key | `default-author` | 32 B | Authorship of our own edits. |
| Vault list + local names | `vault-names.json`, `peers.json` | tiny | Already localStorage-shaped. |

The last three are trivial in IndexedDB. The interesting ones are the first two.

## The upstream hook this needs

**Confirmed, and patched locally** — see `patches/`, applied by
`patches/apply.sh` and pointed at by `[patch.crates-io]` in Cargo.toml. It is
exactly the ~5 lines predicted below, because `Engine::spawn` already accepts a
`Store`; nothing else upstream had to move.

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

## Option A — redb on OPFS (chosen; implemented in `src/opfs.rs`)

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
- Blobs stay in `MemStore` plus a `hash -> bytes` table in the same OPFS
  directory, rehydrated on boot. Safe because blob content is immutable and
  content-addressed: re-adding the bytes reproduces the same hashes.
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
3. **Done.** Note content survives, via an append-only blob log beside the
   replica. *Checked by*: the reopened vault reads note **values**, not just
   keys.
4. Move the node into the worker behind a message bridge whose messages are the
   `VaultBackend` commands — the seam already in PR #235. (`web/worker.js` is a
   sketch of exactly this shape: `{ cmd, args } -> value | error`.)
5. Split `vault.rs` into portable core and platform shims (`FsStore`, `notify`,
   mDNS, MCP, linked folders, tray stay desktop-only), and export the command
   surface through `wasm-bindgen`.
6. `vault-wasm.ts` behind `VaultBackend`; keep `zip.ts` for export/import; drop
   `vault-web.ts`.

## What the spike still fakes

Things a real port must do that this deliberately does not:

- **The blob log replays every version on boot.** Blobs are written per save, so
  the log grows without bound and startup cost grows with it. Needs compaction,
  or a real blob store on OPFS rather than `MemStore` plus a log.
- **Entry and content are written separately** (entry first, then the log), so a
  crash between them leaves an entry whose content is missing until a peer
  supplies it. Acceptable for a spike; a port should make the pair atomic or
  order it content-first.
- **The endpoint secret and author key are regenerated every session**, so each
  reload looks like a new device to peers. Both are 32 bytes and belong in
  IndexedDB; nothing here is hard, it just is not done.
- **One writer only.** The OPFS handle is an exclusive lock, so a second tab
  cannot open the same vault. The real app needs to either coordinate tabs
  (shared worker / lock negotiation) or tell the user plainly.

## Risks to keep in view

The 2.7 MB (gzipped) module has to download and compile before the app works on
a phone; iOS still evicts storage for a site that is not installed, so "Add to
Home Screen" stays load-bearing; sync from a browser is relay-only, so no
LAN-speed transfers and no mDNS; and the upstream patch is a dependency we
carry until n0 takes it (or forever, if they would rather not).
