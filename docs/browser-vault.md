# Why the browser runs the real vault

Design notes for #221/#222, written while proving the approach and kept because
they carry the reasoning the code cannot: *why* an iroh-docs replica in the
browser rather than a browser-shaped store of its own, and what that cost.

The shipped code is `crates/vellum-vault` (the portable vault),
`crates/vellum-wasm` (the browser shell) and `src/lib/vault-wasm.ts` (the
frontend's transport). Storage design is in
[browser-vault-storage.md](./browser-vault-storage.md).

The exploration this came from lived in `spike/` and was deleted once the real
implementation landed — its Rust was duplicated by `crates/vellum-wasm`, and its
browser harnesses tested the spike's own wasm build, so they would have passed
while the shipped vault was broken. `git log` has them if they are ever wanted:
they are the four `spike:` commits on this branch.

Asks one question for #221/#222: **can a browser be a real Vellum peer?** If it
can, the hosted web version should run the same iroh-docs vault the native apps
do (compiled to WASM) rather than a separate browser-only store.

## What was proven

1. **It compiles.** `iroh` 1.1, `iroh-docs` 0.101, `iroh-blobs` 0.103,
   `iroh-gossip` 0.101 and `yrs` 0.27 all build for `wasm32-unknown-unknown` at
   the versions `src-tauri/Cargo.toml` pins.
2. **It runs and syncs, browser to browser.** Two independent browser tabs, each
   with its own iroh node: one creates a vault, the other joins it from a share
   ticket, both write a note, and each sees the other's note **with its content**
   (read back out of the blob store) in **~0.5 s** over a relay. Same result for
   a release build. 
3. **It syncs browser to native**, which is the pairing that matters — a phone
   browser against the installed desktop app. The browser fetched the desktop
   peer's ticket, joined, and read the desktop's note with content in ~0.5 s;
   the desktop peer received the browser's note in the same window
   (`HOST SUCCESS`). The native side ran with default features — `fs-store`,
   `rpc` — i.e. the stack the desktop app actually ships.
   
4. **It survives a reload, as a store rather than a demo.** The replica lives on
   OPFS (redb storage backend in `src/opfs.rs`) with note content and this
   device's identity in a second redb database beside it. Across three sessions
   on one vault file: notes come back with their content and no peer involved;
   re-saving identical content stores nothing new; the version an edit
   superseded is collected on the next boot; the endpoint id is unchanged, so
   peers see one device rather than three strangers; the author table stays at
   one key rather than growing per launch; and a second tab is refused with a
   sentence instead of an OPFS error. Needs the local iroh-docs patch in
   `patches/`. (see [browser-vault-storage.md](./browser-vault-storage.md))
5. **Payload**: 9.5 MB wasm, **2.9 MB gzipped** over the wire, before
   `wasm-opt`.

## How much of it is Rust

All of it, bar a bootstrap. 781 lines of Rust against **5 lines of
hand-written JavaScript** in the worker — the command surface, the storage
backends, the GC, the identity handling and the one-writer-per-vault rule
(including the sentence the second tab is shown) all live in `src/bridge.rs`
and `src/lib.rs`. The 1244-line `web/pkg/*.js` is wasm-bindgen output, not
maintained by hand.

Three things resist, and it is worth knowing which:

- **The worker entry point.** `new Worker()` needs a JavaScript module; you
  cannot point it at a `.wasm`, and wasm cannot instantiate itself.
- **A backlog in front of it.** A message posted to a worker whose `onmessage`
  is still unset is *dropped, not queued* — the port's message queue is enabled
  when the module starts evaluating, not when its top-level `await` finishes.
  Instantiating 9.5 MB takes longer than it takes a page to send its first
  command, so the bootstrap parks arrivals in an array and hands them to Rust.
  Skipping this is not a race you lose occasionally; `open` was lost every time.
- **`pagehide`.** Fires on the `Window`; a dedicated worker gets no notice that
  its page is going away, it is simply terminated. So the last-chance flush has
  to be triggered from the main thread — the one policy here that provably
  cannot be Rust-owned.

The main-thread proxy in `web/persist.html` stays JavaScript on purpose: it
stands in for `vault-wasm.ts`, and `src/lib/vault.ts` is already this exact
shape for the desktop build — a thin typed surface over commands, with the
logic behind it.

## What it took

- `rpc` feature **off** on iroh-docs/iroh-blobs: it pulls tokio's `net` feature
  (mio), which does not build for wasm. The in-process `api`/`protocol` surface
  that `vault.rs` already uses works without it.
- `fs-store` **off**: redb-on-a-file cannot work in a browser. The replica uses
  redb's in-memory (`Vec<u8>`) backend via `Docs::memory()`, blobs use
  `MemStore`.
- `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'` (iroh's own
  `.cargo/config.toml` does the same).
- A wasm-capable clang for `ring`'s build script; Apple clang cannot target
  wasm. `CC_wasm32_unknown_unknown=$(brew --prefix llvm)/bin/clang`.
- **`endpoint.online().await` before minting a ticket.** A browser endpoint has
  no direct addresses, so a ticket minted before the home relay is up is
  undialable. The first run of this spike failed exactly there: the join was
  accepted and then nothing synced, with no error anywhere.
- **An explicit `flush` after a save.** iroh-docs batches entries into one redb
  transaction and commits after 500 ms of idle or on a graceful shutdown. A
  browser tab is not guaranteed either, so a write that returned successfully
  can still vanish with the tab — this spike lost an edit to exactly that.
  Durability in the browser has to be asked for, not inherited.

## What is still unproven

- **Wiring it into Vellum**: splitting `vault.rs` into a portable core plus
  desktop-only shims, and exporting the command surface through `wasm-bindgen`
  behind the `VaultBackend` seam from PR #235. Steps 4–6 in [browser-vault-storage.md](./browser-vault-storage.md).
- The remaining shortcuts, listed under "What the spike still fakes" there —
  notably that entry and content are still two writes rather than one
  transaction, that whole blobs pass through memory, and that one vault still
  admits only one tab.
- **Blocking the UI.** iroh's browser build is single-threaded; CRDT merges and
  blob hashing on the main thread would jank the editor. A Worker is probably
  required regardless of the storage choice.

## Building it today

`bun run build` builds the wasm first (`bun run build:wasm`), so the frontend
now needs a Rust toolchain and `wasm-bindgen-cli` at the version
`crates/vellum-wasm/Cargo.toml` pins. On macOS the build also needs a
wasm-capable clang, because Apple's cannot target wasm and `ring` has a build
script:

```sh
export CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang"
export AR_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/llvm-ar"
```

`--cfg getrandom_backend="wasm_js"` is set for the wasm target in
`.cargo/config.toml`, so it needs no exporting.

There is no browser-level test suite yet. The spike had one and it was
genuinely useful; rebuilding it against the shipped shell — boot a vault, write,
reload, read it back, refuse a second tab, sync two tabs — is worthwhile
follow-up work.
