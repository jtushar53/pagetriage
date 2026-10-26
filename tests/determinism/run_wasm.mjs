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
const STAMP_OPTS = '{"debug":true,"detectors":["stamp"]}';
const LAYOUT_OPTS = '{"debug":true,"detectors":["layout","script","handwriting","rulings","signature","thumb","redaction","qr","emboss","photo","orientation","plan"]}';
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

// The opt-in layout detector, on the layout pages and the quality pages.
const layoutInputs = ["layout_inputs", "quality_inputs"]
  .flatMap((sub) => readdirSync(join(golden, sub)).filter((f) => f.endsWith(".pgm")).map((f) => join(sub, f)))
  .sort();
if (layoutInputs.length < 16) {
  console.error(`expected at least 16 layout inputs, found ${layoutInputs.length}`);
  process.exit(1);
}
for (const file of layoutInputs) {
  const name = file.replace(/^.*\//, "").replace(/\.pgm$/, "");
  const { w, h, pixels } = readPgm(join(golden, file));
  const actual = wasm.analyse_page(pixels, undefined, w, h, DPI, undefined, LAYOUT_OPTS) + "\n";
  const expected = readFileSync(join(golden, "layout_expected", `${name}.json`), "utf8");
  if (actual === expected) {
    console.log(`ok   wasm   layout ${name}`);
  } else {
    failures++;
    console.log(`FAIL wasm   layout ${name}`);
  }
}

// The opt-in coverage detector, with text boxes over the top third of the page.
{
  const { w, h, pixels } = readPgm(join(golden, "layout_inputs", "layout_one_column.pgm"));
  const opts = '{"debug":true,"detectors":["coverage"],"text_boxes":[[0,0,319,86]]}';
  const actual = wasm.analyse_page(pixels, undefined, w, h, DPI, undefined, opts) + "\n";
  const expected = readFileSync(join(golden, "coverage_expected", "layout_one_column.json"), "utf8");
  if (actual === expected) {
    console.log("ok   wasm   coverage layout_one_column");
  } else {
    failures++;
    console.log("FAIL wasm   coverage layout_one_column");
  }
}

// The opt-in stamp detector needs a colour page: the P6 files in stamp_inputs/.
function readPpm(path) {
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
  if (tokens[0] !== "P6" || tokens[3] !== "255") throw new Error(`${path}: unsupported PPM`);
  const [w, h] = [Number(tokens[1]), Number(tokens[2])];
  const rgb = new Uint8Array(data.subarray(pos));
  if (rgb.length !== 3 * w * h) throw new Error(`${path}: pixel count mismatch`);
  return { w, h, rgb };
}
const stampInputs = readdirSync(join(golden, "stamp_inputs")).filter((f) => f.endsWith(".ppm")).sort();
if (stampInputs.length < 3) {
  console.error(`expected at least 3 stamp inputs, found ${stampInputs.length}`);
  process.exit(1);
}
for (const file of stampInputs) {
  const name = file.replace(/\.ppm$/, "");
  const { w, h, rgb } = readPpm(join(golden, "stamp_inputs", file));
  const gray = wasm.rgb_to_gray(rgb);
  const actual = wasm.analyse_page(gray, rgb, w, h, DPI, undefined, STAMP_OPTS) + "\n";
  const expected = readFileSync(join(golden, "stamp_expected", `${name}.json`), "utf8");
  if (actual === expected) {
    console.log(`ok   wasm   stamp ${name}`);
  } else {
    failures++;
    console.log(`FAIL wasm   stamp ${name}`);
  }
}

// The document-level pass.
{
  const docIn = readFileSync(join(golden, "document_inputs", "deed_pages.json"), "utf8");
  const actual = wasm.finish_document(docIn, "{}") + "\n";
  const expected = readFileSync(join(golden, "document_expected", "deed_pages.json"), "utf8");
  if (actual === expected) {
    console.log("ok   wasm   document deed_pages");
  } else {
    failures++;
    console.log("FAIL wasm   document deed_pages");
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
