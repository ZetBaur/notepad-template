#!/bin/sh
# Fetches the speech model that ships inside the app bundle.
#
# The model is bundled rather than downloaded on first run because the archive
# is downloaded *after* purchase — the user has already paid, so size is not a
# conversion risk, and in exchange they get "unzip, run, works" with no wait
# and no dependency on a third-party host being reachable at the worst
# possible moment (right before an interview). See README.
#
# Not checked into git (see .gitignore) — CI and local release builds run this.
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESOURCE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/resources"
MODEL="ggml-small.bin"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$MODEL"

mkdir -p "$RESOURCE_DIR"
OUT="$RESOURCE_DIR/$MODEL"

if [ -f "$OUT" ]; then
  echo "fetch-model.sh: $MODEL already present, skipping"
  exit 0
fi

echo "fetch-model.sh: downloading $MODEL (~466 MB)…"
# Download to a temp path and rename, so an interrupted run can't leave a
# truncated file that later fails to load with a confusing error.
curl -fL --progress-bar "$URL" -o "$OUT.partial"
mv "$OUT.partial" "$OUT"
echo "fetch-model.sh: saved to $OUT"
