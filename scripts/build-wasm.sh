#!/usr/bin/env bash
# Build the WASM package into dist/wasm/{web,nodejs}.
#   web     -> browsers / bundlers (this is the size-checked artifact)
#   nodejs  -> used by tests/determinism/run_wasm.mjs
# Needs: the wasm32-unknown-unknown target (rust-toolchain.toml adds it), `wasm-bindgen` CLI at the
# exact version pinned in crates/pagetriage-wasm/Cargo.toml, and `wasm-opt` (binaryen).
set -euo pipefail
cd "$(dirname "$0")/.."

PINNED=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/pagetriage-wasm/Cargo.toml)
INSTALLED=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ -z "$INSTALLED" ]; then
  echo "error: wasm-bindgen CLI not found. Install: cargo install wasm-bindgen-cli --version $PINNED --locked (it lands in ~/.cargo/bin; make sure that is on PATH)" >&2
  exit 1
fi
if [ "$PINNED" != "$INSTALLED" ]; then
  echo "error: wasm-bindgen CLI $INSTALLED does not match the crate pin $PINNED" >&2
  echo "       cargo install wasm-bindgen-cli --version $PINNED --locked --force" >&2
  exit 1
fi
command -v wasm-opt >/dev/null || { echo "error: wasm-opt not found (brew install binaryen / apt install binaryen)" >&2; exit 1; }

cargo build --profile wasm --target wasm32-unknown-unknown -p pagetriage-wasm
RAW=target/wasm32-unknown-unknown/wasm/pagetriage_wasm.wasm

# Post-link features rustc emits for wasm32-unknown-unknown. The profile strips the target_features
# section, so wasm-opt cannot detect them and they are listed here. This is also the browser
# baseline: bulk memory (Chrome 75, Firefox 79, Safari 15), sign extension, mutable globals and
# saturating float-to-int (`as` casts; fully specified by WASM, so identical in every engine).
WASM_FEATURES="--enable-bulk-memory-opt --enable-sign-ext --enable-mutable-globals --enable-nontrapping-float-to-int"

OUT=dist/wasm
rm -rf "$OUT"
for target in web nodejs; do
  wasm-bindgen --target "$target" --out-dir "$OUT/$target" --out-name pagetriage "$RAW"
  # shellcheck disable=SC2086
  wasm-opt -Oz $WASM_FEATURES "$OUT/$target/pagetriage_bg.wasm" -o "$OUT/$target/pagetriage_bg.wasm"
done
echo "built $OUT/web and $OUT/nodejs"
