// Background sync ("live sync") preference. When on, the backend arms every
// vault as an always-on hub and the platform keeps the process alive (desktop:
// tray + launch-at-login; Android: foreground service) so notes keep syncing
// with no window open / while the app is backgrounded. Persisted to localStorage
// and pushed to the backend on change and once on launch.

import { setBackgroundSync } from "./vault";

const KEY = "vellum-live-sync";

let enabled = $state<boolean>(localStorage.getItem(KEY) === "1");

// Push the persisted value to the backend at startup (e.g. re-arm the hub +
// restart the Android service after a relaunch). Fire-and-forget — the backend
// no-ops if the node isn't ready yet; the toggle path is the source of truth.
export function initLiveSync() {
  if (enabled) void setBackgroundSync(true).catch(() => {});
}

export const liveSync = {
  get enabled() {
    return enabled;
  },
  set enabled(v: boolean) {
    enabled = v;
    localStorage.setItem(KEY, v ? "1" : "0");
    void setBackgroundSync(v).catch(() => {});
  },
};

// Reflect a change made in the backend (e.g. "Turn off background sync" from the
// desktop tray) without calling back into setBackgroundSync — the backend has
// already applied it; this just keeps the persisted value + UI toggle in step.
export function applyLiveSyncFromBackend(v: boolean) {
  enabled = v;
  localStorage.setItem(KEY, v ? "1" : "0");
}
