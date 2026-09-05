// The vault node runs here, not on the main thread, for two reasons: OPFS sync
// access handles are worker-only, and iroh's browser build is single-threaded,
// so CRDT merges and blob hashing would otherwise jank the editor.
//
// The message protocol is deliberately the shape of a command surface
// ({ cmd, args } -> value | error), because that is what the real port needs:
// the same commands vault.ts already calls.
import init, { start_persistent, write, dump } from "./pkg/iroh_wasm_spike.js";

const ready = init();

self.onmessage = async (e) => {
  const { id, cmd, args = [] } = e.data;
  try {
    await ready;
    let value;
    switch (cmd) {
      case "open":
        value = await start_persistent(args[0], args[1] ?? undefined);
        break;
      case "write":
        value = await write(args[0], args[1]);
        break;
      case "dump":
        value = await dump();
        break;
      default:
        throw new Error(`unknown command ${cmd}`);
    }
    self.postMessage({ id, ok: true, value });
  } catch (err) {
    self.postMessage({ id, ok: false, error: String(err?.message ?? err) });
  }
};
