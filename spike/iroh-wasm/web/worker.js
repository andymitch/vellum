// The vault node runs here, not on the main thread, for two reasons: OPFS sync
// access handles are worker-only, and iroh's browser build is single-threaded,
// so CRDT merges and blob hashing would otherwise jank the editor.
//
// This file is as small as a worker can be. A `Worker` entry point has to be a
// JavaScript module — you cannot point `new Worker()` at a `.wasm` — and wasm
// cannot instantiate itself, so `import init` and calling it is irreducible.
// The backlog is too: a message posted before `onmessage` is set is dropped
// rather than queued, and instantiating 9.5 MB of wasm takes longer than it
// takes a page to send its first command.
//
// Everything else — the command surface, the one-writer-per-vault rule, the
// sentence the second tab gets — is in Rust. See src/bridge.rs.
import init, { serve } from "./pkg/iroh_wasm_spike.js";

const backlog = [];
self.onmessage = (e) => backlog.push(e.data);

await init();
serve(backlog);
