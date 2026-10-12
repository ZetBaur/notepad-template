#!/bin/sh
# Builds the platform-specific native audio-capture helper as a Tauri
# "sidecar" binary (src-tauri/binaries/<name>-<target-triple>), so
# `tauri dev` / `tauri build` can find and bundle it. No-op on platforms that
# don't have a helper yet (Windows WASAPI capture is a later phase — see
# README.md "Status").
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC_TAURI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN_DIR="$SRC_TAURI_DIR/binaries"
mkdir -p "$BIN_DIR"

OS="$(uname -s)"

case "$OS" in
  Darwin)
    if ! command -v swiftc >/dev/null 2>&1; then
      echo "build-native.sh: swiftc not found — install Xcode Command Line Tools" >&2
      exit 1
    fi
    TRIPLE="$(rustc -Vv | sed -n 's/^host: //p')"
    if [ -z "$TRIPLE" ]; then
      echo "build-native.sh: could not determine host target triple via rustc -Vv" >&2
      exit 1
    fi
    OUT="$BIN_DIR/notepad-audiotap-$TRIPLE"
    echo "build-native.sh: compiling audiotap.swift -> $(basename "$OUT")"
    swiftc -O "$SRC_TAURI_DIR/native/macos/audiotap.swift" -o "$OUT"
    chmod +x "$OUT"
    ;;
  *)
    echo "build-native.sh: no native audio-capture helper for '$OS' yet — skipping"
    ;;
esac
