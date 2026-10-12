#!/bin/sh
# Cargo "runner" hook (see ../.cargo/config.toml) — invoked instead of running
# the built binary directly, so we can codesign it first.
#
# Why this exists: macOS Keychain access grants ("Always Allow") are tied to
# the calling binary's code signature, not just its path. An unsigned debug
# build gets a new identity on every rebuild, so the OS treats it as a new,
# unrecognized app each time and re-prompts for the Mac password on every
# Keychain read — including switching LLM provider in Settings, which reads a
# separate Keychain item per provider. Signing with a stable local identity
# (see README "Local dev code signing") makes that grant persist across
# rebuilds during development, same as it will for real users once the
# release build carries a real Apple Developer ID.
#
# This is a dev-loop convenience only — release builds get properly signed
# and notarized per docs/PRODUCT-BRIEF.md §3, unrelated to this identity.
set -eu

BIN="$1"
shift

IDENTITY=""
for candidate in "Notepad Local Dev" "SyntaxCue Local Dev"; do
  if security find-identity -v -p codesigning 2>/dev/null | grep -q "$candidate"; then
    IDENTITY="$candidate"
    break
  fi
done

if [ -n "$IDENTITY" ]; then
  codesign --force --sign "$IDENTITY" "$BIN" 2>/dev/null || \
    echo "codesign-and-run.sh: signing failed, running unsigned" >&2
else
  # No local dev cert set up (e.g. a teammate's machine, or CI) — this is
  # optional tooling, not a build requirement, so just run unsigned.
  echo "codesign-and-run.sh: no local dev identity found, running unsigned" >&2
fi

exec "$BIN" "$@"
