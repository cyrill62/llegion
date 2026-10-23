#!/usr/bin/env bash
# Regenerates the README screenshots (docs/screenshots/*.png) from a debug build,
# against the mock Ollama in tests/fixtures so no real data appears.
#
# Usage: scripts/screenshots.sh [output dir]
#   OLLAMA_EGUI_URL=http://127.0.0.1:11434 scripts/screenshots.sh   # use a real server instead
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=${1:-docs/screenshots}
PORT=11500

if [ -z "${OLLAMA_EGUI_URL:-}" ]; then
  python3 tests/fixtures/mock_ollama.py "$PORT" 2>/dev/null &
  MOCK=$!
  trap 'kill $MOCK 2>/dev/null || true' EXIT
  export OLLAMA_EGUI_URL="http://127.0.0.1:$PORT"
fi

cargo build
OLLAMA_EGUI_SCREENSHOTS="$OUT" ./target/debug/ollama-egui
ls -l "$OUT"/*.png
