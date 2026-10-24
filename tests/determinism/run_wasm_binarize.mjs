#!/usr/bin/env node
// Binarise the golden inputs through the WASM build and require the golden SHA-256 hashes.
// Build first with scripts/build-wasm.sh.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const golden = join(here, "..", "golden");
const pkgDir = process.env.WASM_PKG ?? join(here, "..", "..", "dist", "wasm", "nodejs");
const wasm = createRequire(import.meta.url)(join(pkgDir, "pagetriage.js"));

function readPgm(path) {
  const data = readFileSync(path);
  const isSpace = (b) => b === 0x20 || (b >= 0x09 && b <= 0x0d);
  let pos = 0;
  const tokens = [];
  while (tokens.length < 4) {
    while (isSpace(data[pos])) pos++;
    const start = pos;
    while (!isSpace(data[pos])) pos++;
    tokens.push(data.subarray(start, pos).toString("ascii"));
  }
  pos++;
  if (tokens[0] !== "P5" || tokens[3] !== "255") throw new Error(`${path}: unsupported PGM`);
  const [w, h] = [Number(tokens[1]), Number(tokens[2])];
  const pixels = new Uint8Array(data.subarray(pos));
  if (pixels.length !== w * h) throw new Error(`${path}: pixel count mismatch`);
  return { w, h, pixels };
}

const cases = JSON.parse(readFileSync(join(golden, "binarize_cases.json"), "utf8"));
const expected = JSON.parse(readFileSync(join(golden, "binarize_expected.json"), "utf8"));
const inputs = ["inputs", "binarize_inputs"]
  .flatMap((sub) => readdirSync(join(golden, sub)).filter((f) => f.endsWith(".pgm")).map((f) => join(sub, f)))
  .sort();
if (inputs.length < 6) {
  console.error(`expected at least 6 golden inputs, found ${inputs.length}`);
  process.exit(1);
}

let failures = 0;

// The browser demo turns canvas pixels into gray with this export, so pin it to the core's own test
// vector (black, white, red, green, blue).
const gray = Array.from(wasm.rgb_to_gray(new Uint8Array([0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255])));
if (gray.join() !== "0,255,76,150,29") {
  failures++;
  console.log(`FAIL wasm   rgb_to_gray: got ${gray.join()}`);
} else {
  console.log("ok   wasm   rgb_to_gray");
}

for (const file of inputs) {
  const name = file.replace(/^.*\//, "").replace(/\.pgm$/, "");
  const { w, h, pixels } = readPgm(join(golden, file));
  const bad = [];
  for (const c of cases) {
    const ink = wasm.binarize(pixels, w, h, c.method, c.params || undefined);
    const digest = createHash("sha256").update(ink).digest("hex");
    if (digest !== expected[name][c.name]) bad.push(c.name);
  }
  if (bad.length) {
    failures++;
    console.log(`FAIL wasm   binarize ${name}: ${bad.join(", ")}`);
  } else {
    console.log(`ok   wasm   binarize ${name.padEnd(16)} ${cases.length} methods`);
  }
}
process.exit(failures ? 1 : 0);
