#!/usr/bin/env bash
# The determinism gate: the golden inputs must produce byte-identical reports on native, Python and
# WASM, each compared against the checked-in expected bytes in tests/golden/expected.
# CI runs this on Linux and macOS, so passing on both also means the two OSes agree.
set -euo pipefail
cd "$(dirname "$0")/.."

PYTHON=${PYTHON:-python3}
VENV=${VENV:-.venv}

echo "== native"
cargo test -p pagetriage-core --test golden --quiet
cargo test -p pagetriage-core --test binarize_golden --quiet
cargo test -p pagetriage-core --features pdf --test golden_pdf --quiet

echo "== python"
if [ ! -x "$VENV/bin/maturin" ]; then
  "$PYTHON" -m venv "$VENV"
  "$VENV/bin/pip" install --quiet maturin
fi
export VIRTUAL_ENV="$PWD/$VENV"
export PATH="$VIRTUAL_ENV/bin:$PATH"
maturin develop --release --quiet --manifest-path crates/pagetriage-py/Cargo.toml
python -I tests/determinism/run_python.py
python -I tests/determinism/run_python_binarize.py
python -I tests/determinism/run_python_pdf.py

echo "== wasm"
scripts/build-wasm.sh
scripts/check-wasm-size.sh
node tests/determinism/run_wasm.mjs
node tests/determinism/run_wasm_binarize.mjs

echo "== wasm with the pdf feature (PDF structural layer)"
PAGETRIAGE_WASM_FEATURES=pdf WASM_OUT=dist/wasm-pdf scripts/build-wasm.sh
scripts/check-wasm-size.sh dist/wasm-pdf/web/pagetriage_bg.wasm
node tests/determinism/run_wasm_pdf.mjs

echo "determinism: native, python and wasm all match the golden reports"
