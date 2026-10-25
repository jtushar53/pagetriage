#!/usr/bin/env bash
# Build the demo site: the PDF-enabled WASM package into site/pkg, then check that it reproduces the
# native reports for every sample (site/tools/check-samples.mjs). Needs what scripts/build-wasm.sh
# needs, plus node.
#
#   scripts/build-site.sh
#   python3 -m http.server 8000 --directory site     # then open http://localhost:8000
set -euo pipefail
cd "$(dirname "$0")/.."

PAGETRIAGE_WASM_FEATURES=pdf WASM_OUT=dist/wasm-pdf scripts/build-wasm.sh
rm -rf site/pkg
mkdir -p site/pkg
cp dist/wasm-pdf/web/pagetriage.js dist/wasm-pdf/web/pagetriage_bg.wasm site/pkg/
node site/tools/check-samples.mjs
echo "site ready in site/ (gzipped wasm: $(gzip -9 -c site/pkg/pagetriage_bg.wasm | wc -c | tr -d ' ') bytes)"
