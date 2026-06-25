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

The build also produces a `.dmg` at `src-tauri/target/release/bundle/dmg/` if you'd rather distribute that.

> The app is unsigned. The install script handles Gatekeeper for the local install; distributing it to other machines would require code-signing and notarization.

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
