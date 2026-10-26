#!/usr/bin/env bash
# Regenerates the README screenshots (docs/screenshots/*.png) from a debug build,
# against the mock Ollama (examples/mock_ollama.rs) so no real data appears.
#
# Usage: scripts/screenshots.sh [output dir]
#   LLEGION_URL=http://127.0.0.1:11434 scripts/screenshots.sh   # use a real server instead
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=${1:-docs/screenshots}
PORT=11500

cargo build --bin llegion --example mock_ollama

if [ -z "${LLEGION_URL:-}" ]; then
  LOG=$(mktemp)
  ./target/debug/examples/mock_ollama "$PORT" >"$LOG" 2>/dev/null &
  MOCK=$!
  trap 'kill $MOCK 2>/dev/null || true; rm -f "$LOG"' EXIT
  for _ in $(seq 50); do
    grep -q "listening" "$LOG" && break
    kill -0 "$MOCK" 2>/dev/null || { echo "mock server exited (port $PORT in use?)" >&2; exit 1; }
    sleep 0.1
  done
  export LLEGION_URL="http://127.0.0.1:$PORT"
fi

LLEGION_SCREENSHOTS="$OUT" ./target/debug/llegion
ls -l "$OUT"/*.png
