#!/usr/bin/env bash
# Assemble the npm package from the WASM build (dist/wasm/{web,nodejs}) into dist/npm, pack it, and
# check that the packed tarball installs and runs under Node. It does NOT publish: publishing is a
# separate, deliberate step (`npm publish dist/npm/pagetriage-*.tgz`).
#
#   scripts/build-wasm.sh && scripts/package-npm.sh
#
# The package has two builds: `nodejs/` (CommonJS, loads the .wasm synchronously; the `node` and
# `require` entries) and `web/` (an ES module, `init()` then the functions; the default and `import`
# entries for bundlers and browsers). Both are the size-checked pixel build (no PDF parser); the PDF
# build is the demo site's.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC=${WASM_OUT:-dist/wasm}
for t in web nodejs; do
  [ -f "$SRC/$t/pagetriage_bg.wasm" ] || { echo "error: $SRC/$t is missing; run scripts/build-wasm.sh first" >&2; exit 1; }
done
VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { echo "error: no workspace version" >&2; exit 1; }

OUT=dist/npm
rm -rf "$OUT"
mkdir -p "$OUT/package"
cp -R "$SRC/web" "$SRC/nodejs" "$OUT/package/"
cp LICENSE-MIT LICENSE-APACHE "$OUT/package/"
cp README.md "$OUT/package/README.md"
cat > "$OUT/package/package.json" <<JSON
{
  "name": "pagetriage",
  "version": "$VERSION",
  "description": "Deterministic, non-ML page-content analyser for degraded scanned legal documents (WASM build)",
  "license": "MIT OR Apache-2.0",
  "repository": { "type": "git", "url": "git+https://github.com/jtushar53/pagetriage.git" },
  "keywords": ["document-analysis", "scanned-documents", "wasm", "deterministic", "ocr-triage"],
  "main": "nodejs/pagetriage.js",
  "module": "web/pagetriage.js",
  "types": "web/pagetriage.d.ts",
  "exports": {
    ".": {
      "types": "./web/pagetriage.d.ts",
      "node": "./nodejs/pagetriage.js",
      "import": "./web/pagetriage.js",
      "default": "./web/pagetriage.js"
    },
    "./nodejs": "./nodejs/pagetriage.js",
    "./web": "./web/pagetriage.js",
    "./web/pagetriage_bg.wasm": "./web/pagetriage_bg.wasm"
  },
  "files": ["web", "nodejs", "README.md", "LICENSE-MIT", "LICENSE-APACHE"],
  "sideEffects": false
}
JSON

( cd "$OUT" && npm pack --silent ./package --pack-destination . >/dev/null )
TARBALL=$(ls "$OUT"/pagetriage-*.tgz)
echo "packed $TARBALL ($(wc -c < "$TARBALL" | tr -d ' ') bytes)"
tar -tzf "$TARBALL" | sed 's/^/  /'

# Install the tarball in a scratch project and run it: a blank page and the wrong-length buffer error.
SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
( cd "$SCRATCH" && npm init -y >/dev/null && npm install --silent --no-audit --no-fund "$OLDPWD/$TARBALL" >/dev/null )
( cd "$SCRATCH" && node -e '
const wasm = require("pagetriage");
const gray = new Uint8Array(64 * 64).fill(255);
const out = JSON.parse(wasm.analyse_page(gray, undefined, 64, 64, 150, undefined, "{}"));
if (!out.findings.some((f) => f.kind === "blank_page")) throw new Error("blank page not reported");
try { wasm.analyse_page(new Uint8Array(3), undefined, 2, 2, 150, undefined, "{}"); throw new Error("no throw"); }
catch (e) { if (!(e instanceof Error) || e.message === "no throw") throw e; }
console.log("npm package ok: version " + wasm.version());
' )
