#!/usr/bin/env node
// The demo page claims "the report in your browser is byte-identical to the native build's". This
// checks that claim for every sample in site/samples/samples.json: it runs the WASM build on the
// sample and compares the SHA-256 of the report with the one the native CLI produced
// (site/tools/make_samples.py wrote it). Exit code 1 on any difference.
//
// Needs the PDF-enabled build: scripts/build-site.sh builds it, or run scripts/build-wasm.sh with
// PAGETRIAGE_WASM_FEATURES=pdf WASM_OUT=dist/wasm-pdf. WASM_PKG (an absolute path to a nodejs
// package folder) overrides where it looks.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkgDir = process.env.WASM_PKG ?? resolve(site, "..", "dist", "wasm-pdf", "nodejs");
const wasm = createRequire(import.meta.url)(join(pkgDir, "pagetriage.js"));
const sha256 = (data) => createHash("sha256").update(data).digest("hex");

/** 8-bit gray, non-interlaced PNG only: that is all the samples are. */
function decodeGrayPng(file) {
  const buf = readFileSync(file);
  if (buf.subarray(0, 8).toString("latin1") !== "\x89PNG\r\n\x1a\n") throw new Error(`${file}: not a PNG`);
  let pos = 8, w = 0, h = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.subarray(pos + 4, pos + 8).toString("latin1");
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0); h = data.readUInt32BE(4);
      if (data[8] !== 8 || data[9] !== 0 || data[12] !== 0) throw new Error(`${file}: need 8-bit gray, not interlaced`);
    } else if (type === "IDAT") idat.push(data);
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    const filter = raw[y * (w + 1)];
    const row = raw.subarray(y * (w + 1) + 1, (y + 1) * (w + 1));
    for (let x = 0; x < w; x++) {
      const a = x ? out[y * w + x - 1] : 0;
      const b = y ? out[(y - 1) * w + x] : 0;
      const c = x && y ? out[(y - 1) * w + x - 1] : 0;
      let v = row[x];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      out[y * w + x] = v & 255;
    }
  }
  return { w, h, gray: out };
}

const manifest = JSON.parse(readFileSync(join(site, "samples", "samples.json"), "utf8"));
let failures = 0;
const report = (ok, kind, id, got, want) => {
  if (!ok) failures++;
  console.log(`${ok ? "ok  " : "FAIL"} ${kind.padEnd(5)} ${id.padEnd(20)} ${got.slice(0, 16)}${ok ? "" : ` (native ${want.slice(0, 16)})`}`);
};

for (const p of manifest.pages) {
  const { w, h, gray } = decodeGrayPng(join(site, p.file));
  if (w !== p.width || h !== p.height) throw new Error(`${p.id}: size ${w}x${h} differs from the manifest`);
  const json = wasm.analyse_page(gray, undefined, w, h, p.dpi, undefined, "{}");
  const got = sha256(Buffer.from(json, "utf8"));
  report(got === p.native_report_sha256, "page", p.id, got, p.native_report_sha256);
}
for (const p of manifest.pdfs) {
  const json = wasm.analyse_pdf_structure(new Uint8Array(readFileSync(join(site, p.file))), undefined);
  const got = sha256(Buffer.from(json, "utf8"));
  report(got === p.native_report_sha256, "pdf", p.id, got, p.native_report_sha256);
}
process.exit(failures ? 1 : 0);
