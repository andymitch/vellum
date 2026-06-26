# Vellum

A local-first Markdown notes app that syncs **peer-to-peer** — no account, no server, no cloud. Your notes live on your devices and sync directly between them. Built with [Tauri](https://tauri.app), [Svelte 5](https://svelte.dev), and [iroh](https://iroh.computer).

## Features

- **Markdown editor** with live source/preview modes, syntax highlighting, and a formatting toolbar (CodeMirror 6).
- **Vaults** — collections of notes organized in folders. Create as many as you like.
- **P2P sync** — share a vault to another device by scanning a QR code; edits then flow both ways automatically. Works over the internet (via relays) or directly on the same Wi-Fi (via mDNS).
- **No server** — there's nothing to sign up for and nothing to host. Devices find and sync with each other directly.
- **Obsidian-style titles** — a note's filename tracks its first `# Heading`, so renaming is just editing.
- **Folder tree** with drag-and-drop, rename, duplicate, and delete.
- **Cross-platform** — macOS desktop and Android (iOS targeted), from one codebase.

## How sync works (in brief)

Each vault is an [iroh-docs](https://github.com/n0-computer/iroh) document; notes are entries keyed by their path. Sharing a vault produces a write-capability *ticket* (rendered as a QR code) — any device that joins it gets equal, full read/write access. Peers are remembered by their stable node ID and re-dialed through iroh's discovery, so sync survives IP changes, network switches, and restarts. There is no central authority: every synced device holds a complete copy.

## Install (macOS)

A build-and-install script is provided:

```sh
./scripts/install-macos.sh
```

This installs JS dependencies, runs `tauri build` to produce a native `.app`, copies it to `/Applications/Vellum.app`, and clears the Gatekeeper quarantine flag so it launches without a right-click. Launch it with `open -a Vellum` or from Spotlight.

Re-run the script any time to **update in place** — it removes the existing `/Applications/Vellum.app` before copying the freshly built one, so a rebuild-and-reinstall is a single command. Your notes are stored separately (see below) and are untouched by reinstalls.

The build also produces a `.dmg` at `src-tauri/target/release/bundle/dmg/` if you'd rather distribute that.

> The app is unsigned. The install script handles Gatekeeper for the local install; distributing it to other machines would require code-signing and notarization.

### Where your notes live

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

### Side-by-side dev build

To test changes without disturbing your production install, build a separate
**Vellum Dev** app (`com.andymitch.vellum.dev`, its own data dir):

```sh
./scripts/dev-macos.sh     # installs "Vellum Dev.app" on macOS
./scripts/dev-android.sh   # installs com.andymitch.vellum.dev on a connected device
```

Both sit alongside the production app. The macOS variant uses
`src-tauri/tauri.dev.conf.json`; the Android one suffixes the applicationId at
build time (Tauri regenerates the Gradle config, so it can't be committed there)
and signs with the debug key.

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

### Android

```sh
bun run tauri android init       # one-time, generates the Android project
bun run tauri android dev        # run on a connected device / emulator
bun run tauri android build      # produce an APK / AAB
```

Android needs the SDK and NDK installed, with `ANDROID_HOME` / `NDK_HOME` set. The native side has Android-specific glue (network-change notifications and system-bar theming via JNI) so iroh can sync reliably as the device moves between Wi-Fi and cellular.

### Tests

The Rust P2P backend has integration tests that spin up real iroh nodes:

```sh
cd src-tauri && cargo test
```

## Project layout

| Path | What's there |
| --- | --- |
| `src/` | Svelte 5 frontend — editor, sidebar, components |
| `src/lib/vault.ts` | Typed wrapper over the Tauri commands |
| `src-tauri/src/vault.rs` | The iroh-docs P2P vault backend (the heart of the app) |
| `src-tauri/src/lib.rs` | Tauri setup, command registration, Android JNI glue |
| `scripts/` | Build/install helpers |

## License

MIT
