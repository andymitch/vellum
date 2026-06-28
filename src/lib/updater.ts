// Desktop-only in-app auto-update. The Tauri updater plugin is registered only
// on desktop (mobile updates ship via the APK/stores), so check() rejects on
// mobile — we guard and swallow. Called once on launch from App.svelte.
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";

export async function checkForUpdate(): Promise<void> {
  let update;
  try {
    update = await check();
  } catch {
    return; // no updater (mobile), offline, or endpoint unreachable
  }
  if (!update?.available) return;
  // Show "vN": our release scheme stamps the tag's integer as the semver major
  // (vN -> N.0.0), so the major component is the release number (matches the
  // version shown in Settings).
  const rel = (v: string) => "v" + v.split(".")[0];
  // Use the dialog plugin, not window.confirm: the webview's synchronous
  // confirm()/alert() are no-ops in Tauri v2 (they return false without
  // rendering), which silently blocked every update.
  const ok = await ask(
    `Vellum ${rel(update.version)} is available (currently running ${rel(update.currentVersion)}).\n\nDownload and restart to update now?`,
    { title: "Update available", kind: "info" },
  );
  if (!ok) return;
  try {
    await update.downloadAndInstall();
    await relaunch();
  } catch (e) {
    // Surface the failure instead of swallowing it to the console (where it's
    // invisible on a release build).
    await message(`Update failed: ${e}`, { title: "Update", kind: "error" });
  }
}
