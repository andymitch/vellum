# Spike: iroh-docs in the browser

Asks one question for #221/#222: **can a browser be a real Vellum peer?** If it
can, the hosted web version should run the same iroh-docs vault the native apps
do (compiled to WASM) rather than a separate browser-only store.

## Result: yes

1. **It compiles.** `iroh` 1.1, `iroh-docs` 0.101, `iroh-blobs` 0.103,
   `iroh-gossip` 0.101 and `yrs` 0.27 all build for `wasm32-unknown-unknown` at
   the versions `src-tauri/Cargo.toml` pins.
2. **It runs and syncs, browser to browser.** Two independent browser tabs, each
   with its own iroh node: one creates a vault, the other joins it from a share
   ticket, both write a note, and each sees the other's note **with its content**
   (read back out of the blob store) in **~0.5 s** over a relay. Same result for
   a release build. (`twopeers.mjs`)
3. **It syncs browser to native**, which is the pairing that matters — a phone
   browser against the installed desktop app. The browser fetched the desktop
   peer's ticket, joined, and read the desktop's note with content in ~0.5 s;
   the desktop peer received the browser's note in the same window
   (`HOST SUCCESS`). The native side ran with default features — `fs-store`,
   `rpc` — i.e. the stack the desktop app actually ships.
   (`crossshell.mjs` + `../iroh-native-peer`)
4. **Payload**: 8.8 MB wasm, **2.7 MB gzipped** over the wire, before
   `wasm-opt`.

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

## What is still unproven

- **Persistence.** Both stores above are in memory: reload the tab and the notes
  are gone. This is the substantial remaining question — see PERSISTENCE.md for
  the options and a recommendation.
- **Blocking the UI.** iroh's browser build is single-threaded; CRDT merges and
  blob hashing on the main thread would jank the editor. A Worker is probably
  required regardless of the storage choice.

## Running it

### Browser to browser

```sh
export CC_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/clang"
export AR_wasm32_unknown_unknown="$(brew --prefix llvm)/bin/llvm-ar"
export RUSTFLAGS='--cfg getrandom_backend="wasm_js"'
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128   # match the wasm-bindgen dep

cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/pkg --no-typescript \
  target/wasm32-unknown-unknown/release/iroh_wasm_spike.wasm

(cd web && python3 -m http.server 9334 --bind 127.0.0.1 &)
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new \
  --disable-gpu --no-sandbox --user-data-dir=/tmp/spike-profile \
  --remote-debugging-port=9333 --remote-allow-origins='*' about:blank &
node twopeers.mjs   # exits 0 only if both directions synced
```

### Browser to native

With the page server and Chrome from above still running:

```sh
cargo run --manifest-path ../iroh-native-peer/Cargo.toml &   # prints its ticket
node crossshell.mjs   # exits 0 only if the browser read the desktop's note;
                      # the native peer prints HOST SUCCESS for the other way
```

Needs internet: a browser peer has no direct addresses, so every connection
goes through a relay.
