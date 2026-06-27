#!/usr/bin/env bash
#
# Build and install a side-by-side "Vellum Dev" APK on a connected Android
# device/emulator, so you can test changes without uninstalling the production
# app or losing its notes.
#
# It installs as com.andymitch.vellum.dev with the label "Vellum Dev" — a
# separate app + data dir from production (com.andymitch.vellum).
#
# How it works: Tauri regenerates app/build.gradle.kts on every build, so a
# committed product flavor wouldn't survive. Instead we let Tauri generate +
# build the project, THEN inject `applicationIdSuffix ".dev"` and the dev label
# and re-assemble the debug variant directly with Gradle (auto-signed with the
# debug key). The patches are reverted on exit.
#
# Usage:
#   ./scripts/dev-android.sh
#
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_ROOT"

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
export ANDROID_HOME
export NDK_HOME="${NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)}"

GEN=src-tauri/gen/android
APP_GRADLE="$GEN/app/build.gradle.kts"
STRINGS="$GEN/app/src/main/res/values/strings.xml"

echo "==> Generating Android project + assets (tauri android build)"
# Produces the rust lib + bundles the web assets into the gradle project.
bun tauri android build --apk --target aarch64

echo "==> Patching dev applicationId suffix + label (reverted on exit)"
# Back up OUTSIDE the source tree — a .bak inside res/ breaks the resource merger.
BK="$(mktemp -d)"
cp "$APP_GRADLE" "$BK/build.gradle.kts.bak"
cp "$STRINGS" "$BK/strings.xml.bak"
trap 'mv "$BK/build.gradle.kts.bak" "$APP_GRADLE"; mv "$BK/strings.xml.bak" "$STRINGS"; rmdir "$BK" 2>/dev/null || true' EXIT
# Suffix the release variant's applicationId so it installs alongside
# production. (The debug variant's rust task expects the `tauri android dev`
# websocket, so we build release — standalone — and sign it ourselves.)
perl -0pi -e 's/(getByName\("release"\)\s*\{)/$1\n            applicationIdSuffix = ".dev"/' "$APP_GRADLE"
# Relabel the app "Vellum Dev".
perl -0pi -e 's/(<string name="app_name">)"?Vellum"?(<\/string>)/$1Vellum Dev$2/; s/(<string name="main_activity_title">)"?Vellum"?(<\/string>)/$1Vellum Dev$2/' "$STRINGS"

echo "==> Re-assembling dev release APK (reusing the already-built rust lib)"
# Skip rustBuild* — those tasks only run under the tauri CLI (they connect to
# its websocket). The .so is already in jniLibs from the build above, so Gradle
# just repackages it with the .dev applicationId.
( cd "$GEN" && ./gradlew assembleUniversalRelease \
    -x rustBuildArmRelease -x rustBuildArm64Release \
    -x rustBuildX86Release -x rustBuildX86_64Release )

UNSIGNED="$GEN/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk"
[[ -f "$UNSIGNED" ]] || { echo "APK not found: $UNSIGNED" >&2; exit 1; }

echo "==> Signing with the debug key + installing com.andymitch.vellum.dev"
BT="$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)"
OUT="$(mktemp -d)/vellum-dev.apk"
"$BT/zipalign" -f -p 4 "$UNSIGNED" "$OUT"
"$BT/apksigner" sign --ks "$HOME/.android/debug.keystore" \
  --ks-pass pass:android --ks-key-alias androiddebugkey --key-pass pass:android "$OUT"
ADB="$ANDROID_HOME/platform-tools/adb"
# `adb install` needs an explicit serial when more than one target is attached
# (a wireless device can even show up twice via mDNS). Honor ANDROID_SERIAL if
# set, else pick the first online device.
SERIAL="${ANDROID_SERIAL:-$("$ADB" devices | awk '/\tdevice$/{print $1; exit}')}"
[[ -n "$SERIAL" ]] || { echo "No connected device/emulator found." >&2; exit 1; }
"$ADB" -s "$SERIAL" install -r "$OUT"

echo "==> Done. 'Vellum Dev' installed alongside production Vellum."
