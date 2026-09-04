# Vellum

[![Release](https://img.shields.io/github/v/release/andymitch/vellum)](https://github.com/andymitch/vellum/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Android%20%7C%20Web-lightgrey)

A local-first Markdown notes app that syncs **peer-to-peer** — no account, no server, no cloud. Your notes live on your devices and sync directly between them. Built with [Tauri](https://tauri.app), [Svelte 5](https://svelte.dev), and [iroh](https://iroh.computer).

## Features

- **Markdown editor** (CodeMirror 6) with live source/preview modes, syntax highlighting, and formatting shortcuts — plus a formatting toolbar on mobile.
- **Note types** beyond Markdown — a **TODO list** that's a real checklist (tick, reorder by dragging, sweep away completed items) and a **Journal** that starts a new dated section each day. Typed notes have a single view: no source/preview toggle to think about.
- **Search & tags** — `Cmd`/`Ctrl+F` searches every note in the vault; write `#tags` anywhere in a note and click one to filter.
- **Vaults** — collections of notes organized in folders. Create as many as you like.
- **P2P sync** — share a vault to another device by scanning a QR code; edits then flow both ways automatically. Works over the internet (via relays) or directly on the same Wi-Fi (via mDNS).
- **No server** — there's nothing to sign up for and nothing to host. Devices find and sync with each other directly.
- **Folder tree** with drag-and-drop, rename, duplicate, and delete.
- **Eight themes & several fonts** — including a GitHub theme, and on Android 12+ a Material You theme that follows your wallpaper.
- **Cross-platform** — macOS desktop, Android, and a [hosted web version](https://andymitch.github.io/vellum/app/) you can install as an app (the iOS route, since there's no App Store build), all from one codebase.
- **Runs in a browser too** — the same app, storing notes in the browser instead of an iroh replica. No sync there; a zip export moves notes into the installed apps.
- **Self-updating** — desktop builds check for updates on launch and install them in place; Android updates via [Komi Store](https://github.com/kurikomi-labs/komi-store). Opt in to **beta updates** in Settings to track pre-releases.

## How sync works (in brief)

Each vault is an [iroh-docs](https://github.com/n0-computer/iroh) document; notes are entries keyed by their path. Sharing a vault produces a write-capability *ticket* (rendered as a QR code) — any device that joins it gets equal, full read/write access. Peers are remembered by their stable node ID and re-dialed through iroh's discovery, so sync survives IP changes, network switches, and restarts. There is no central authority: every synced device holds a complete copy.

## Note types

Every note is Markdown on disk, so nothing here changes how a note syncs, exports, or reads in another tool. The *type* just changes how Vellum presents it, and is recorded in a small frontmatter block at the top:

```
---
type: todo
---
- [ ] buy milk
```

- **Markdown** (the default) — the editor you already know, with the source/preview toggle.
- **TODO list** — a checklist. Tick items, drag to reorder, and sweep completed ones away with one button. Stored as ordinary `- [ ]` task lines.
- **Journal** — a running log that cuts a new dated section the first time you write on a new day, rendered as a full-width rule with the date inline. Sections alternate a faint background so they read as distinct entries.

Pick a type when naming a new note, or hold the **+** button and slide onto one.

## Agent access (MCP)

Vellum can host a local [MCP](https://modelcontextprotocol.io) server so agents — Claude Code, Claude Desktop, any MCP client — can read and write your notes. Turn it on in **Settings → Agents → Agent access**, then copy the connect command it shows:

```sh
claude mcp add --transport http vellum http://127.0.0.1:PORT/mcp \
  --header "Authorization: Bearer TOKEN"
```

Agents get tools to list, read, search, create, edit, append, move and delete notes, resources for live-reading a vault, and two prompts (`daily_note`, `vault_review`). Edits go through the same CRDT path the editor uses, so an agent's change appears in your open editor immediately and syncs to your other devices on its own — a note written by Claude on your Mac is on your phone seconds later.

Some details worth knowing:

- **It's off by default**, and listens on `127.0.0.1` only, with a bearer token — so it's reachable by programs on this machine that have the token, and nothing else. The port and token live in `mcp.json` in the [app data directory](#where-your-notes-live-macos).
- **Deletes are soft.** `delete_note` moves the note to `.trash/` rather than deleting it, because a real delete propagates to every synced device.
- **Concurrent edits are safe.** Agents never supply a merge base; the server reads the note's current state and merges against it, so an edit you're making on another device isn't clobbered.
- **Desktop only.** The Claude mobile app's connectors are dialled from Anthropic's servers rather than from your phone, so a loopback server on the phone would be unreachable — and Android freezes the app when it's backgrounded. Point an agent at your Mac instead and let sync carry the result to your phone.

## Install

[![Download for macOS](https://img.shields.io/badge/Download-macOS%20(Apple%20Silicon)-000000?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/andymitch/vellum/releases/latest/download/Vellum_aarch64.dmg)
[![Download for Android](https://img.shields.io/badge/Download-Android-3DDC84?style=for-the-badge&logo=android&logoColor=white)](https://github.com/andymitch/vellum/releases/latest/download/vellum-release.apk)

These always point at the [latest release](https://github.com/andymitch/vellum/releases). On macOS you can also install in one line:

```sh
curl -fsSL https://raw.githubusercontent.com/andymitch/vellum/main/scripts/install.sh | sh
```

**macOS** — download `Vellum_aarch64.dmg`, open it, drag Vellum to Applications. The app is unsigned, so on first launch right-click it → **Open** (or run `xattr -dr com.apple.quarantine /Applications/Vellum.app`). After that it **updates itself** — it checks on launch and prompts to download + install new releases.

**iPhone / iPad** — there is no App Store build; install the web version instead. Open [andymitch.github.io/vellum/app/](https://andymitch.github.io/vellum/app/) in Safari, tap **Share** → **Add to Home Screen**. It then launches full-screen and works offline. Notes live in that browser and **do not sync** — use **Settings → Export vault (.zip)** to move them to the desktop or Android app. Installing rather than leaving it as a tab also protects the data: Safari clears an unused site's storage after a week, but keeps an installed app's.

**Any browser** — same link, no install required. It updates on reload, and tracks `main` rather than the latest release.

**Android** — download `vellum-release.apk` and open it to install (you'll need to allow installing from your browser/files app). Tauri's in-app updater is desktop-only, so for automatic Android updates add this repo to **[Komi Store](https://github.com/kurikomi-labs/komi-store)** — an open-source app store for GitHub releases that watches for new versions and updates in place (optionally silently, via Shizuku). Komi verifies the signing key matches, so install the release `.apk` (not a locally built debug one) for updates to flow.

> Prefer to build it yourself? See [Development](#development).

### Where your notes live (macOS)

Vellum stores all vault data — the notes database, synced blobs, and this device's stable sync identity — under the app's data directory, keyed by its bundle identifier (`com.andymitch.vellum`):

```
~/Library/Application Support/com.andymitch.vellum/
├── docs.redb         # the notes database
├── blobs/            # synced note content
├── default-author    # this device's iroh author key
├── endpoint-secret   # this device's stable node identity
└── peers.json        # remembered peers
```

This directory is **not** removed when you reinstall or delete the app, so your notes survive updates. The matching cache lives at `~/Library/Caches/com.andymitch.vellum`.

> Deleting `endpoint-secret` / `default-author` changes this device's identity, so existing peers will no longer recognise it for sync.

### Uninstalling

```sh
rm -rf /Applications/Vellum.app
rm -rf ~/Library/Application\ Support/com.andymitch.vellum   # deletes all notes — irreversible
rm -rf ~/Library/Caches/com.andymitch.vellum
```

Remove only the first line to uninstall the app while keeping your notes for a later reinstall.

## Development

### Prerequisites

- [Bun](https://bun.sh)
- [Rust](https://rustup.rs) (stable toolchain)
- Platform tooling per the [Tauri prerequisites](https://tauri.app/start/prerequisites/) — Xcode Command Line Tools on macOS; the Android SDK/NDK for Android builds.

### Run

```sh
bun install
bun run tauri dev          # desktop, with hot reload
```

### Build

```sh
bun run tauri build        # native app + installer for the current desktop OS
```

To build and install a local macOS copy in one step (builds, copies to
`/Applications/Vellum.app`, clears Gatekeeper):

```sh
./scripts/install-macos.sh   # re-run any time to update in place; notes are untouched
```

> Updater artifacts are enabled, so a local desktop `tauri build` needs the
> updater signing key — export `TAURI_SIGNING_PRIVATE_KEY` (and
> `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) first, or it will fail to bundle. The
> `dev-*` scripts disable updater artifacts, so they don't need it.

### Android

```sh
bun run tauri android init       # one-time, generates the Android project
bun run tauri android dev        # run on a connected device / emulator
bun run tauri android build      # produce an APK / AAB
```

Android needs the SDK and NDK installed, with `ANDROID_HOME` / `NDK_HOME` set. The native side has Android-specific glue (network-change notifications and system-bar theming via JNI) so iroh can sync reliably as the device moves between Wi-Fi and cellular.

### Side-by-side dev build

Test changes without disturbing your production install — these build a separate **Vellum Dev** app (`com.andymitch.vellum.dev`, its own data dir) that sits alongside it:

```sh
./scripts/dev-macos.sh     # installs "Vellum Dev.app"
./scripts/dev-android.sh   # installs com.andymitch.vellum.dev on a connected device
```

The macOS variant uses `src-tauri/tauri.dev.conf.json`; the Android one suffixes the applicationId at build time (Tauri regenerates the Gradle config, so it can't be committed there) and signs with the debug key.

### Web build

The hosted version is the same bundle with a browser vault behind it (see
[Project layout](#project-layout)) plus a service worker and a web manifest:

```sh
bun run dev:web            # dev server with the browser backend selected
bun run build:web          # production build into build-web/
bun run preview            # serve that build locally
```

It is published to GitHub Pages under `/app/` by `.github/workflows/pages.yml`,
alongside the marketing site. `VELLUM_BASE` sets the subdirectory it is served
from and `VELLUM_VERSION` the version it reports; both are set by that workflow.

### Tests

The Rust P2P backend has integration tests that spin up real iroh nodes:

```sh
cd src-tauri && cargo test
```

The frontend's tests cover the browser vault's rules — tree shape, name
de-duplication, search ranking, tag counts, the zip container — which are
re-implementations of logic in `vault.rs` and would otherwise drift from it:

```sh
bun test
```

### Releases

Releases are cut with one click: **Actions → Release → Run workflow**. It
auto-increments the version tag (`v1`, `v2`, …), builds the macOS `.dmg` +
signed updater artifacts and the signed Android `.apk`, publishes a GitHub
Release, and writes a categorized changelog. Versions are a plain counter; CI
stamps the build version as `N.0.0` so Tauri and the updater get valid semver.

The changelog groups merged PRs into **Features / Fixes / Maintenance** by the
label of the **issue each PR closes** (so label issues, not PRs, and reference
the issue with `Closes #N`). Atomic PRs (one issue each) give the cleanest
changelog; a PR closing several issues collapses to a single entry.

## Project layout

| Path | What's there |
| --- | --- |
| `src/` | Svelte 5 frontend — editor, sidebar, components |
| `src/lib/vault.ts` | The vault API, and the switch between its two backends |
| `src/lib/vault-tauri.ts` | Backend 1: the Tauri commands (desktop + Android) |
| `src/lib/vault-web.ts` | Backend 2: IndexedDB, for the web build / iOS PWA |
| `src/lib/platform.ts` | Which shell we're in — the only user-agent sniffing |
| `src/lib/theme.svelte.ts` | Theme/font state + Material You (Android) |
| `src-tauri/src/vault.rs` | The iroh-docs P2P vault backend (the heart of the app) |
| `src-tauri/src/lib.rs` | Tauri setup, command registration, Android JNI glue |
| `scripts/` | Build/install helpers + `dev-*` side-by-side builds |
| `.github/workflows/release.yml` | One-click release CI |
| `.github/workflows/pages.yml` | Publishes the site + the hosted web app |

## License

MIT
