// Desktop-only in-app auto-update. The Tauri updater plugin is registered only
// on desktop (mobile updates ship via the APK/stores), so check() rejects on
// mobile — we guard and swallow.
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getVersion } from "@tauri-apps/api/app";

const REPO = "andymitch/vellum";
// Public repo, so the GitHub Releases API needs no auth. `fetch` reaches it from
// the webview on both desktop and Android (CSP is null; the API sends CORS).
const LATEST_RELEASE_API = `https://api.github.com/repos/${REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;

export type LatestRelease = { version: string; tag: string; notes: string; url: string };

// Latest published GitHub release (tag like `v3` / `v3.1`, `version` stripped of
// the leading `v`, plus the markdown release notes). Returns null on any failure
// (offline, rate-limited, no release yet) so callers degrade quietly.
export async function fetchLatestRelease(): Promise<LatestRelease | null> {
  try {
    const res = await fetch(LATEST_RELEASE_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!res.ok) return null;
    const j = await res.json();
    const tag: string = j.tag_name ?? "";
    if (!tag) return null;
    return {
      version: tag.replace(/^v/, ""),
      tag,
      notes: (j.body ?? "").trim(),
      url: j.html_url ?? RELEASES_PAGE,
    };
  } catch {
    return null;
  }
}

// Compare dotted numeric versions; missing trailing components count as 0, so
// `3` == `3.0.0` and `3.1` > `3.0.1`. >0 when a is newer than b.
export function cmpVersion(a: string, b: string): number {
  const pa = a.split(".").map((n) => parseInt(n, 10) || 0);
  const pb = b.split(".").map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

// Format a version for display: drop trailing zero components only, keeping at
// least the major. e.g. 5.0.0 -> v5, 5.1.0 -> v5.1, 5.0.1 -> v5.0.1 (#124).
export const formatVersion = (v: string) => {
  const parts = v.split(".");
  while (parts.length > 1 && parts[parts.length - 1] === "0") parts.pop();
  return "v" + parts.join(".");
};
const rel = formatVersion;
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

// Mobile update check (#145). Android ships via APK (no updater plugin), so we
// compare against the latest GitHub release and, if newer, offer to open the
// release page to grab the APK. Silent on launch; `interactive` (Settings button)
// always reports status. Skips dev (0.x) builds like the desktop path.
export async function checkForUpdateMobile(interactive = false): Promise<void> {
  let current = "";
  try {
    current = await getVersion();
  } catch {
    // ignore — treated as unknown current version below
  }
  if (current && isDevVersion(current)) {
    if (interactive) {
      await message("This is a development build — update checks are disabled.", {
        title: "Check for updates",
        kind: "info",
      });
    }
    return;
  }
  const latest = await fetchLatestRelease();
  if (!latest) {
    if (interactive) {
      await message("Couldn't check for updates. Try again later.", {
        title: "Check for updates",
        kind: "error",
      });
    }
    return;
  }
  if (current && cmpVersion(latest.version, current) <= 0) {
    if (interactive) {
      await message(`You're on the latest version${current ? ` (${rel(current)})` : ""}.`, {
        title: "Check for updates",
        kind: "info",
      });
    }
    return;
  }
  const ok = await ask(
    `Vellum ${rel(latest.version)} is available${current ? ` (currently running ${rel(current)})` : ""}.\n\nOpen the download page?`,
    { title: "Update available", kind: "info" },
  );
  if (ok) await openUrl(latest.url);
}
