#!/usr/bin/env bash
# Fail if the gzipped WASM exceeds the size budget for the current phase.
# Phase 0 budget: 1 MB gzipped (1,000,000 bytes). Later phases raise it (Phase 5 target: 4 MB).
# Usage: scripts/check-wasm-size.sh [file.wasm]   (override budget with WASM_GZ_BUDGET_BYTES)
set -euo pipefail
cd "$(dirname "$0")/.."

FILE=${1:-dist/wasm/web/pagetriage_bg.wasm}
BUDGET=${WASM_GZ_BUDGET_BYTES:-1000000}

[ -f "$FILE" ] || { echo "error: $FILE not found; run scripts/build-wasm.sh first" >&2; exit 1; }
RAW=$(wc -c < "$FILE" | tr -d ' ')
GZ=$(gzip -9 -c "$FILE" | wc -c | tr -d ' ')
echo "$FILE: raw ${RAW} bytes, gzip -9 ${GZ} bytes, budget ${BUDGET} bytes"
if [ "$GZ" -gt "$BUDGET" ]; then
  echo "FAIL: gzipped size ${GZ} exceeds budget ${BUDGET}" >&2
  exit 1
fi
echo "ok: within budget ($(( BUDGET - GZ )) bytes spare)"
