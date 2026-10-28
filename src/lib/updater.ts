// Desktop-only in-app auto-update. The Tauri updater plugin is registered only
// on desktop (mobile updates ship via the APK/stores), so check() rejects on
// mobile — we guard and swallow.
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { appVersion, isAndroidApp, isMacApp } from "./platform";
import { betaChannel } from "./beta-channel.svelte";

const REPO = "andymitch/vellum";
// Public repo, so the GitHub Releases API needs no auth. `fetch` reaches it from
// the webview on both desktop and Android (CSP is null; the API sends CORS).
const LATEST_RELEASE_API = `https://api.github.com/repos/${REPO}/releases/latest`;
// All releases, newest first — used to find the newest PRE-release for the beta
// channel (#175). /releases/latest deliberately excludes pre-releases, which is
// what keeps a beta away from stable users, so the beta channel has to look here.
const RELEASES_API = `https://api.github.com/repos/${REPO}/releases`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;

export type LatestRelease = {
  version: string;
  tag: string;
  notes: string;
  url: string;
  /// Whether this is a pre-release, so callers can label it as such.
  prerelease: boolean;
};

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
      prerelease: false,
    };
  } catch {
    return null;
  }
}

// Newest release by version, stable or pre-release (#291). A beta tester follows
// both: the next beta, and the stable release that supersedes the beta they're on
// (11.0.0-beta.4 < 11.0.0). `needManifest` skips releases with no `latest.json`,
// which can't be updated to but still have a changelog worth reading.
async function newestRelease(
  needManifest: boolean,
): Promise<(LatestRelease & { manifest?: string }) | null> {
  try {
    const res = await fetch(RELEASES_API, { headers: { Accept: "application/vnd.github+json" } });
    if (!res.ok) return null;
    let best: (LatestRelease & { manifest?: string }) | null = null;
    for (const r of await res.json()) {
      const tag: string = r.tag_name ?? "";
      if (r.draft || !tag) continue;
      const manifest = (r.assets ?? []).find((a: { name: string }) => a.name === "latest.json")
        ?.browser_download_url;
      if (needManifest && !manifest) continue;
      if (best && cmpTag(tag, best.tag) <= 0) continue;
      best = {
        version: tag.replace(/^v/, ""),
        tag,
        notes: (r.body ?? "").trim(),
        url: r.html_url ?? RELEASES_PAGE,
        prerelease: !!r.prerelease,
        manifest,
      };
    }
    return best;
  } catch {
    return null;
  }
}

// Compare release tags (`v11`, `v11.1`, `v12-beta.3`); a stable release sorts
// after every beta of the same version.
function cmpTag(a: string, b: string): number {
  const [va, ba] = a.replace(/^v/, "").split("-beta.");
  const [vb, bb] = b.replace(/^v/, "").split("-beta.");
  const n = (beta?: string) => (beta === undefined ? Number.MAX_SAFE_INTEGER : parseInt(beta, 10) || 0);
  return cmpVersion(va, vb) || n(ba) - n(bb);
}

/// Newest release on the channel the user is actually on (#210).
///
/// `/releases/latest` excludes pre-releases by design — that exclusion is what
/// keeps a beta away from stable users. Reading the changelog through it meant a
/// beta tester was shown the notes for an OLDER stable release, describing none
/// of what they were testing.
export async function fetchChannelRelease(): Promise<LatestRelease | null> {
  let beta = betaChannel.enabled;
  if (!beta) {
    // Also treat a running pre-release as the beta channel. Someone who opted in,
    // updated, then switched the toggle off is still running a beta until stable
    // overtakes them, and should not be shown notes for a build older than the
    // one in front of them. The web build says the same thing with the tag it
    // was cut from (`v10-beta.2`, `v10-beta.2+3`), so the hyphen test holds.
    beta = (await appVersion()).includes("-");
  }
  if (!beta) return fetchLatestRelease();
  return (await newestRelease(false)) ?? fetchLatestRelease();
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

// Watch for updates while the app runs, not only at launch (#292): the app
// re-checks whenever it comes back to the foreground, at most every 30 minutes.
const RECHECK_MS = 30 * 60 * 1000;
let lastCheck = 0;
let checking = false;
// A version the user said no to this session. Background checks don't ask about
// it again; an explicit "Check for updates" still does.
let declined: string | null = null;

export async function checkForUpdatesIfDue(force = false): Promise<void> {
  if (checking || (!force && Date.now() - lastCheck < RECHECK_MS)) return;
  checking = true;
  lastCheck = Date.now();
  try {
    if (isMacApp) await checkForUpdate();
    else if (isAndroidApp) await checkForUpdateMobile();
  } finally {
    checking = false;
  }
}

// Uses the dialog plugin, not window.confirm: the webview's synchronous
// confirm()/alert() are no-ops in Tauri v2 (they return false without rendering).
async function askOnce(version: string, interactive: boolean, text: string, title: string) {
  if (!interactive && version === declined) return false;
  const yes = await ask(text, { title, kind: "info" });
  if (!yes) declined = version;
  return yes;
}

// Prompt, then download + restart. Shared by the background check and the manual one.
async function promptAndInstall(update: Update, interactive: boolean): Promise<void> {
  const ok = await askOnce(
    update.version,
    interactive,
    `Vellum ${rel(update.version)} is available (currently running ${rel(update.currentVersion)}).\n\nDownload and restart to update now?`,
    "Update available",
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

// Silent background check: prompt only if an update is available. Skips dev
// builds (v0) so a locally-built 0.x app isn't nagged every launch (#110).
export async function checkForUpdate(): Promise<void> {
  // Beta testers follow betas and stable releases both, which the stable
  // endpoint alone can't do (#175, #291).
  if (betaChannel.enabled) {
    await checkForBetaUpdate();
    return;
  }
  let update;
  try {
    update = await check();
  } catch {
    return; // no updater (mobile), offline, or endpoint unreachable
  }
  if (!update?.available) return;
  if (isDevVersion(update.currentVersion)) return;
  await promptAndInstall(update, false);
}

// Manual "Check for updates" (Settings button). Unlike the launch check, this
// always gives feedback — dev build, up to date, error, or the update prompt —
// since the user explicitly asked.
export async function checkForUpdateInteractive(): Promise<void> {
  const current = await appVersion();
  if (current && isDevVersion(current)) {
    await message("This is a development build — in-app updates are disabled.", {
      title: "Check for updates",
      kind: "info",
    });
    return;
  }
  if (betaChannel.enabled) {
    await checkForBetaUpdate(true);
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
  await promptAndInstall(update, true);
}

// Mobile update check (#145). Android ships via APK (no updater plugin), so we
// compare against the latest GitHub release and, if newer, offer to open the
// release page to grab the APK. Silent on launch; `interactive` (Settings button)
// always reports status. Skips dev (0.x) builds like the desktop path.
export async function checkForUpdateMobile(interactive = false): Promise<void> {
  const current = await appVersion();
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
  const ok = await askOnce(
    latest.version,
    interactive,
    `Vellum ${rel(latest.version)} is available${current ? ` (currently running ${rel(current)})` : ""}.\n\nOpen the download page?`,
    "Update available",
  );
  // open_update_page routes to the Komi Store app if installed, else the browser.
  if (ok) await invoke("open_update_page", { url: latest.url });
}


// ---- beta channel (#175) ----

/// Check the beta channel and offer the update. `interactive` reports status
/// even when there's nothing to do (the Settings button); the launch check is
/// silent. Returns true if an update was offered.
export async function checkForBetaUpdate(interactive = false): Promise<boolean> {
  // The updater's compiled-in endpoint is /releases/latest/, which excludes
  // pre-releases, so resolve the newest manifest here and hand its URL over.
  const pre = await newestRelease(true);
  if (!pre?.manifest) {
    if (interactive)
      await message("Couldn't find a release to update to.", {
        title: "Check for updates",
        kind: "info",
      });
    return false;
  }
  let available: string | null = null;
  try {
    available = await invoke<string | null>("check_update_at", { url: pre.manifest });
  } catch (e) {
    if (interactive)
      await message(`Couldn't check the beta channel: ${e}`, {
        title: "Check for updates",
        kind: "error",
      });
    return false;
  }
  if (!available) {
    if (interactive)
      await message("You're on the latest version, counting betas.", {
        title: "Check for updates",
        kind: "info",
      });
    return false;
  }
  const yes = await askOnce(
    available,
    interactive,
    `Vellum ${pre.tag} is available. Install and restart?`,
    pre.prerelease ? "Beta update available" : "Update available",
  );
  if (!yes) return true;
  try {
    await invoke("install_update_at", { url: pre.manifest });
    await relaunch();
  } catch (e) {
    await message(`Update failed: ${e}`, { title: "Beta update", kind: "error" });
  }
  return true;
}
