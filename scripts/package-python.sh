#!/usr/bin/env bash
# Build the Python wheel (abi3, CPython >= 3.9) into dist/wheels and check that it installs into a
# fresh virtual environment and runs. It does NOT publish. A wheel for PyPI has to be built in a
# manylinux container (or by CI per platform) so its tag is portable; the one built here carries this
# machine's glibc tag.
#
#   scripts/package-python.sh
set -euo pipefail
cd "$(dirname "$0")/.."

VENV=${VENV:-.venv}
if [ ! -x "$VENV/bin/maturin" ]; then
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install --quiet maturin
fi
rm -rf dist/wheels
"$VENV/bin/maturin" build --release -m crates/pagetriage-py/Cargo.toml -o dist/wheels
WHEEL=$(ls dist/wheels/*.whl)
echo "built $WHEEL ($(wc -c < "$WHEEL" | tr -d ' ') bytes)"

SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
python3 -m venv "$SCRATCH/venv"
"$SCRATCH/venv/bin/pip" install --quiet "$WHEEL"
"$SCRATCH/venv/bin/python" -I - <<'PY'
import json
import pagetriage_py

gray = bytes([255]) * (64 * 64)
out = json.loads(pagetriage_py.analyse_page(gray, 64, 64))
assert any(f["kind"] == "blank_page" for f in out["findings"]), out
try:
    pagetriage_py.analyse_page(b"\x00\x00\x00", 2, 2)
except ValueError:
    pass
else:
    raise SystemExit("an inconsistent buffer did not raise ValueError")
print("python wheel ok:", len(pagetriage_py.schema_json()), "bytes of schema,",
      len(pagetriage_py.params_toml()), "bytes of default params")
PY
