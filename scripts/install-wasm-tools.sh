#!/usr/bin/env bash
# Install the pinned WASM toolchain from official release binaries, verifying checksums:
#   - wasm-bindgen CLI, at the exact version pinned in crates/pagetriage-wasm/Cargo.toml
#   - binaryen (wasm-opt), at BINARYEN_VERSION
# Used by CI; also fine locally. Installs under $INSTALL_ROOT (default ~/.local/pagetriage-tools)
# and, when run in GitHub Actions, adds the bin directories to PATH for later steps. Otherwise it
# prints the PATH line to use.
# wasm-opt links against ../lib/libbinaryen, so the whole binaryen tree is kept, not just the binary.
set -euo pipefail
cd "$(dirname "$0")/.."

WBG_VERSION=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/pagetriage-wasm/Cargo.toml)
BINARYEN_VERSION=${BINARYEN_VERSION:-133}
ROOT=${INSTALL_ROOT:-$HOME/.local/pagetriage-tools}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  WBG_TARGET=x86_64-unknown-linux-musl; BIN_TARGET=x86_64-linux ;;
  Darwin-arm64)  WBG_TARGET=aarch64-apple-darwin;      BIN_TARGET=arm64-macos ;;
  Darwin-x86_64) WBG_TARGET=x86_64-apple-darwin;       BIN_TARGET=x86_64-macos ;;
  *) echo "error: unsupported platform $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# download <url> <name>; the tarball and its checksum file are fetched side by side.
download() { curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$TMP/$2" "$1"; }

# check <tarball> <checksum-file>: the first field of the checksum file is the SHA-256.
check() {
  local want got
  want=$(cut -d' ' -f1 "$2")
  got=$(shasum -a 256 "$1" | cut -d' ' -f1)
  if [ "$want" != "$got" ]; then
    echo "error: checksum mismatch for $(basename "$1"): expected $want, got $got" >&2
    exit 1
  fi
}

mkdir -p "$ROOT"

WBG_NAME="wasm-bindgen-${WBG_VERSION}-${WBG_TARGET}"
WBG_URL="https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WBG_VERSION}/${WBG_NAME}.tar.gz"
download "$WBG_URL" wbg.tar.gz
download "$WBG_URL.sha256sum" wbg.sha
check "$TMP/wbg.tar.gz" "$TMP/wbg.sha"
rm -rf "$ROOT/wasm-bindgen"; mkdir -p "$ROOT/wasm-bindgen"
tar -xzf "$TMP/wbg.tar.gz" -C "$ROOT/wasm-bindgen" --strip-components=1

BIN_NAME="binaryen-version_${BINARYEN_VERSION}-${BIN_TARGET}"
BIN_URL="https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/${BIN_NAME}.tar.gz"
download "$BIN_URL" bin.tar.gz
download "$BIN_URL.sha256" bin.sha
check "$TMP/bin.tar.gz" "$TMP/bin.sha"
rm -rf "$ROOT/binaryen"; mkdir -p "$ROOT/binaryen"
tar -xzf "$TMP/bin.tar.gz" -C "$ROOT/binaryen" --strip-components=1

echo "installed wasm-bindgen ${WBG_VERSION} and binaryen ${BINARYEN_VERSION} under $ROOT"
if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$ROOT/wasm-bindgen" >> "$GITHUB_PATH"
  echo "$ROOT/binaryen/bin" >> "$GITHUB_PATH"
else
  echo "add to PATH:  export PATH=\"$ROOT/wasm-bindgen:$ROOT/binaryen/bin:\$PATH\""
fi
