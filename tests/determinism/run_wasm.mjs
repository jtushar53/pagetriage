#!/usr/bin/env node
// Run the golden inputs through the WASM build (wasm-bindgen `nodejs` target) and require
// byte-identical output. Build first with scripts/build-wasm.sh.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const golden = join(here, "..", "golden");
const pkgDir = process.env.WASM_PKG ?? join(here, "..", "..", "dist", "wasm", "nodejs");
const wasm = createRequire(import.meta.url)(join(pkgDir, "pagetriage.js"));

const OPTS = '{"debug":true}';
const DPI = 150;

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

const inputs = ["inputs", "quality_inputs"]
  .flatMap((sub) => readdirSync(join(golden, sub)).filter((f) => f.endsWith(".pgm")).map((f) => join(sub, f)))
  .sort();
if (inputs.length < 4) {
  console.error(`expected at least 4 golden inputs, found ${inputs.length}`);
  process.exit(1);
}

let failures = 0;
for (const file of inputs) {
  const name = file.replace(/^.*\//, "").replace(/\.pgm$/, "");
  const { w, h, pixels } = readPgm(join(golden, file));
  const actual = wasm.analyse_page(pixels, undefined, w, h, DPI, undefined, OPTS) + "\n";
  const expected = readFileSync(join(golden, "expected", `${name}.json`), "utf8");
  const digest = createHash("sha256").update(actual).digest("hex").slice(0, 16);
  if (actual === expected) {
    console.log(`ok   wasm   ${name.padEnd(16)} sha256=${digest}`);
  } else {
    failures++;
    let at = 0;
    while (at < actual.length && actual[at] === expected[at]) at++;
    console.log(`FAIL wasm   ${name}: first difference at byte ${at}`);
    console.log(`  actual  : ...${JSON.stringify(actual.slice(Math.max(0, at - 30), at + 40))}`);
    console.log(`  expected: ...${JSON.stringify(expected.slice(Math.max(0, at - 30), at + 40))}`);
  }
}

try {
  wasm.analyse_page(new Uint8Array(3), undefined, 2, 2, DPI, undefined, "{}");
  failures++;
  console.log("FAIL wasm   inconsistent buffer length did not throw");
} catch (e) {
  if (e instanceof Error) console.log("ok   wasm   bad buffer throws Error");
  else {
    failures++;
    console.log(`FAIL wasm   thrown value is not an Error: ${e}`);
  }
}

process.exit(failures ? 1 : 0);
