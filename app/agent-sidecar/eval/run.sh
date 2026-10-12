#!/bin/sh
# Usage: ./run.sh [company]   (default company: none; EVAL_REPEATS=3 by default; profile: JH_PROFILE, default = first in profiles/bot/)
# 1) static check of the knowledge base  2) dump real system prompt + routing hints  3) run questions.
set -e
cd "$(dirname "$0")"
EVAL_DIR="$(pwd)"
mkdir -p out
# profile-specific questions: questions-<profile>.json if present
Q_FILE="$EVAL_DIR/questions.json"
[ -n "$JH_PROFILE" ] && [ -f "$EVAL_DIR/questions-$JH_PROFILE.json" ] && Q_FILE="$EVAL_DIR/questions-$JH_PROFILE.json"
export JH_QUESTIONS="$Q_FILE"
node check-kb.mjs
( cd ../../src-tauri && \
  JH_COMPANY="${1:-}" JH_PROFILE="${JH_PROFILE:-}" JH_DUMP="$EVAL_DIR/out/system.txt" JH_HINTS="$EVAL_DIR/out/hints.json" \
  cargo test --lib dump_ -- --ignored --test-threads=1 >/dev/null )
node run.mjs
