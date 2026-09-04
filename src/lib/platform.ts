// Which shell the frontend is running in — the single source of truth for that
// question (#221).
//
// One bundle now ships three ways: the Tauri desktop app, the Tauri Android
// app, and the hosted web build that iOS installs as a PWA (#221/#222). The web
// build has no Rust backend behind it, so everything that needs one — p2p sync,
// the MCP server, linked folders, the in-app updater, the native share sheet —
// has to be gated on `isTauri`, not on the user agent. Gating on the UA alone
// is what would light those up in Safari on a Mac, where every call rejects.
//
// Nothing else in the frontend should sniff the user agent; add a flag here.

import { getVersion } from "@tauri-apps/api/app";

const ua = navigator.userAgent;

/// Running inside the Tauri webview, so the Rust commands and the Tauri plugins
/// are available. Tauri v2 injects `__TAURI_INTERNALS__` before our bundle runs.
export const isTauri = "__TAURI_INTERNALS__" in window;
/// Running in an ordinary browser: the hosted web build, installed or not.
export const isWeb = !isTauri;

/// Touch-first form factor, in either shell — drives mobile UI affordances
/// (quick edit, the markdown toolbar, sheet layouts).
export const isMobile = /Android|iPhone|iPad|iPod/i.test(ua);
/// macOS keyboard conventions (⌘ vs Ctrl). A browser on a Mac counts, since
/// this only decides how a shortcut is *labelled*.
export const isMac = /Macintosh/.test(ua) && !/Android/.test(ua);
/// iOS or iPadOS, where installing the web app means "Add to Home Screen" and
/// nothing else (#222). iPadOS 13+ claims to be a Mac, so it is told apart by
/// having a touch screen.
export const isIOS =
  /iPhone|iPad|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);

/// The installed Android app, not Chrome on Android: Material You, the camera
/// scanner and the APK update check all need the native side.
export const isAndroidApp = isTauri && /Android/.test(ua);
/// The installed macOS app: overlay titlebar inset, in-app updater.
export const isMacApp = isTauri && isMac;
/// The desktop app, which hosts features in its own process: the MCP server,
/// linked folders, the background-sync hub.
export const isDesktopApp = isTauri && !isMobile;

/// Launched from the home screen (iOS) or installed as an app (Chrome), rather
/// than opened as a browser tab. `navigator.standalone` is the iOS-only legacy
/// signal, still the reliable one on older iOS.
export const isStandalone =
  window.matchMedia("(display-mode: standalone)").matches ||
  (navigator as Navigator & { standalone?: boolean }).standalone === true;

/// Version label of the running build.
///
/// The Tauri builds carry a real version stamped into tauri.conf.json at
/// release time, so they ask the backend. The web build is served straight from
/// `main`, so the Pages workflow stamps whatever `git describe` says at build
/// time — a tag (`v10-beta.2`) when the deploy is a release commit, `v10-beta.2+3`
/// when it is three commits past one, and `dev` for a local build.
export const buildVersion = __APP_VERSION__;

/// The running build's version string, resolved for whichever shell we're in.
/// Semver in the Tauri builds (`formatVersion` shortens it for display); the
/// `git describe` label above in the web build, which is already displayable.
export async function appVersion(): Promise<string> {
  if (!isTauri) return buildVersion;
  try {
    return await getVersion();
  } catch {
    return buildVersion;
  }
}
