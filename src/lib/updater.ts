// Desktop-only in-app auto-update. The Tauri updater plugin is registered only
// on desktop (mobile updates ship via the APK/stores), so check() rejects on
// mobile — we guard and swallow. Called once on launch from App.svelte.
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export async function checkForUpdate(): Promise<void> {
  let update;
  try {
    update = await check();
  } catch {
    return; // no updater (mobile), offline, or endpoint unreachable
  }
  if (!update?.available) return;
  const ok = window.confirm(
    `Vellum ${update.version} is available (you have ${update.currentVersion}).\n\nDownload and restart to update now?`,
  );
  if (!ok) return;
  try {
    await update.downloadAndInstall();
    await relaunch();
  } catch (e) {
    console.error("Update failed:", e);
  }
}
