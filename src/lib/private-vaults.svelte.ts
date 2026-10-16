// Private vaults (#274) — see src-tauri/src/private.rs.
//
// The backend owns which vaults are private and which are unlocked, so the MCP
// server obeys the same lock as the window. This mirrors that state for the UI
// and decides *when* to lock: on leaving a vault, when the app goes to the
// background, and after five minutes without input. Every launch starts
// locked, since unlocks live only in the backend's memory.

import { isTauri } from "./platform";
import { privateVaults, setVaultPrivate, unlockVault, lockVaults } from "./vault";

const IDLE_MS = 5 * 60_000;

let priv = $state<string[]>([]);
// Until the list arrives nothing is known to be safe to show (see App).
let ready = $state(!isTauri);
let unlocked = $state<string[]>([]);
let lastInput = Date.now();

function lockAll(except: string | null) {
  unlocked = unlocked.filter((v) => v === except);
  void lockVaults(except).catch((e) => console.error("could not lock vaults", e));
}

export async function initPrivateVaults() {
  if (!isTauri) return;
  priv = await privateVaults().catch(() => []);
  ready = true;
  // A hidden window is a backgrounded app on Android and a minimised or
  // hidden one on desktop.
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && unlocked.length) lockAll(null);
  });
  const touch = () => (lastInput = Date.now());
  for (const ev of ["pointerdown", "keydown", "wheel", "touchstart"])
    window.addEventListener(ev, touch, { capture: true, passive: true });
  setInterval(() => {
    if (unlocked.length && Date.now() - lastInput > IDLE_MS) lockAll(null);
  }, 15_000);
}

export const privacy = {
  /// Whether this platform can lock a vault at all.
  available: isTauri,
  get ready() {
    return ready;
  },
  isPrivate: (vault: string) => priv.includes(vault),
  locked: (vault: string) => priv.includes(vault) && !unlocked.includes(vault),
  async unlock(vault: string): Promise<boolean> {
    const ok = await unlockVault(vault);
    if (ok && !unlocked.includes(vault)) unlocked = [...unlocked, vault];
    lastInput = Date.now();
    return ok;
  },
  /// Asks the OS to confirm the owner first; false when they didn't.
  async setPrivate(vault: string, on: boolean): Promise<boolean> {
    if (!(await setVaultPrivate(vault, on))) return false;
    priv = on ? [...priv.filter((v) => v !== vault), vault] : priv.filter((v) => v !== vault);
    if (!unlocked.includes(vault)) unlocked = [...unlocked, vault];
    return true;
  },
  /// The open vault changed: whatever was left behind locks.
  entered(vault: string | null) {
    if (unlocked.some((v) => v !== vault)) lockAll(vault);
  },
};
