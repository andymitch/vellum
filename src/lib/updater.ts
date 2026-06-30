// Desktop-only in-app auto-update. The Tauri updater plugin is registered only
// on desktop (mobile updates ship via the APK/stores), so check() rejects on
// mobile — we guard and swallow.
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { getVersion } from "@tauri-apps/api/app";

// Drop trailing zero components: 5.1.0 → v5.1, 5.0.1 → v5.0.1, 5.0.0 → v5.
const rel = (v: string) => {
  const parts = v.split(".");
  while (parts.length > 1 && parts[parts.length - 1] === "0") parts.pop();
  return "v" + parts.join(".");
};
// Dev builds run off a 0.x version — there's no real release to update to.
const isDevVersion = (v: string) => (parseInt(v.split(".")[0], 10) || 0) < 1;

// Prompt, then download + restart. Shared by the launch check and the manual one.
// Uses the dialog plugin, not window.confirm: the webview's synchronous
// confirm()/alert() are no-ops in Tauri v2 (they return false without rendering).
async function promptAndInstall(update: Update): Promise<void> {
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

// Silent check on launch: prompt only if an update is available. Skips dev
// builds (v0) so a locally-built 0.x app isn't nagged every launch (#110).
export async function checkForUpdate(): Promise<void> {
  let update;
  try {
    update = await check();
  } catch {
    return; // no updater (mobile), offline, or endpoint unreachable
  }
  if (!update?.available) return;
  if (isDevVersion(update.currentVersion)) return;
  await promptAndInstall(update);
}

// Manual "Check for updates" (Settings button). Unlike the launch check, this
// always gives feedback — dev build, up to date, error, or the update prompt —
// since the user explicitly asked.
export async function checkForUpdateInteractive(): Promise<void> {
  let current = "";
  try {
    current = await getVersion();
  } catch {
    // ignore — fall through; the update check below still reports status
  }
  if (current && isDevVersion(current)) {
    await message("This is a development build — in-app updates are disabled.", {
      title: "Check for updates",
      kind: "info",
    });
    return;
  }
  let update;
  try {
    update = await check();
  } catch (e) {
    await message(`Couldn't check for updates: ${e}`, {
      title: "Check for updates",
      kind: "error",
    });
    return;
  }
  if (!update?.available) {
    await message(`You're on the latest version${current ? ` (${rel(current)})` : ""}.`, {
      title: "Check for updates",
      kind: "info",
    });
    return;
  }
  await promptAndInstall(update);
}
