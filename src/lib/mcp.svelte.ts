// Agent access — the local MCP server (#164), desktop only.
//
// Unlike the background-sync toggle, the backend is the source of truth here:
// the port and token live in `mcp.json` in the app data dir, and the server
// restarts itself on launch if it was left on. So this store *reads* status
// from the backend rather than mirroring a localStorage value.

import { mcpStatus, setMcpEnabled, type McpStatus } from "./vault";

const OFF: McpStatus = { enabled: false, port: null, url: null, token: "", command: null };

let status = $state<McpStatus>(OFF);
let busy = $state(false);

/// Read the current state once at startup. Fire-and-forget: on mobile the
/// command reports disabled, and a failure just leaves the toggle off.
export async function initMcp() {
  try {
    status = await mcpStatus();
  } catch {
    status = OFF;
  }
}

export const mcp = {
  get enabled() {
    return status.enabled;
  },
  get url() {
    return status.url;
  },
  get command() {
    return status.command;
  },
  /// True while a toggle is in flight — starting the server binds a port, so
  /// it isn't instantaneous and the checkbox shouldn't be double-fired.
  get busy() {
    return busy;
  },
  async toggle(on: boolean) {
    if (busy) return;
    busy = true;
    try {
      status = await setMcpEnabled(on);
    } catch (e) {
      console.error("[mcp] toggle failed", e);
      // Re-read rather than assume: the server may have started and then
      // failed to persist, and the checkbox must reflect reality.
      await initMcp();
    } finally {
      busy = false;
    }
  },
};
