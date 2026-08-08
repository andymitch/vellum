# Vellum

[![Release](https://img.shields.io/github/v/release/andymitch/vellum?sort=semver)](https://github.com/andymitch/vellum/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Android-lightgrey)

A local-first Markdown notes app that syncs **peer-to-peer** — no account, no server, no cloud. Your notes live on your devices and sync directly between them. Built with [Tauri](https://tauri.app), [Svelte 5](https://svelte.dev), and [iroh](https://iroh.computer).

## Features

- **Markdown editor** (CodeMirror 6) with live source/preview modes, syntax highlighting, and formatting shortcuts — plus a formatting toolbar on mobile.
- **Vaults** — collections of notes organized in folders. Create as many as you like.
- **P2P sync** — share a vault to another device by scanning a QR code; edits then flow both ways automatically. Works over the internet (via relays) or directly on the same Wi-Fi (via mDNS).
- **No server** — there's nothing to sign up for and nothing to host. Devices find and sync with each other directly.
- **Folder tree** with drag-and-drop, rename, duplicate, and delete.
- **Eight themes & several fonts** — including a GitHub theme, and on Android 12+ a Material You theme that follows your wallpaper.
- **Cross-platform** — macOS desktop and Android, from one codebase (iOS scaffolded).
- **Self-updating** — desktop builds check for updates on launch and install them in place; Android updates via [Komi Store](https://github.com/kurikomi-labs/komi-store).

## How sync works (in brief)

Each vault is an [iroh-docs](https://github.com/n0-computer/iroh) document; notes are entries keyed by their path. Sharing a vault produces a write-capability *ticket* (rendered as a QR code) — any device that joins it gets equal, full read/write access. Peers are remembered by their stable node ID and re-dialed through iroh's discovery, so sync survives IP changes, network switches, and restarts. There is no central authority: every synced device holds a complete copy.

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

### Tests

The Rust P2P backend has integration tests that spin up real iroh nodes:

```sh
cd src-tauri && cargo test
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
| `src/lib/vault.ts` | Typed wrapper over the Tauri commands |
| `src/lib/theme.svelte.ts` | Theme/font state + Material You (Android) |
| `src-tauri/src/vault.rs` | The iroh-docs P2P vault backend (the heart of the app) |
| `src-tauri/src/lib.rs` | Tauri setup, command registration, Android JNI glue |
| `scripts/` | Build/install helpers + `dev-*` side-by-side builds |
| `.github/workflows/release.yml` | One-click release CI |

## License

MIT
