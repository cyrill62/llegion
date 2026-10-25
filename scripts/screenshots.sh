#!/usr/bin/env bash
# Regenerates the README screenshots (docs/screenshots/*.png) from a debug build,
# against the mock Ollama in tests/fixtures so no real data appears.
#
# Usage: scripts/screenshots.sh [output dir]
#   LLEGION_URL=http://127.0.0.1:11434 scripts/screenshots.sh   # use a real server instead
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=${1:-docs/screenshots}
PORT=11500

if [ -z "${LLEGION_URL:-}" ]; then
  python3 tests/fixtures/mock_ollama.py "$PORT" 2>/dev/null &
  MOCK=$!
  trap 'kill $MOCK 2>/dev/null || true' EXIT
  export LLEGION_URL="http://127.0.0.1:$PORT"
fi

cargo build
LLEGION_SCREENSHOTS="$OUT" ./target/debug/llegion
ls -l "$OUT"/*.png
