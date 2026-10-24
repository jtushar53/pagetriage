#!/usr/bin/env node
// Run the golden PDFs through the WASM build that has the `pdf` feature (wasm-bindgen `nodejs`
// target) and require byte-identical structure reports. Build first with
// `PAGETRIAGE_WASM_FEATURES=pdf WASM_OUT=dist/wasm-pdf scripts/build-wasm.sh`.
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const golden = join(here, "..", "golden");
const pkgDir = process.env.WASM_PKG ?? join(here, "..", "..", "dist", "wasm-pdf", "nodejs");
const wasm = createRequire(import.meta.url)(join(pkgDir, "pagetriage.js"));

const inputs = readdirSync(join(golden, "pdf")).filter((f) => f.endsWith(".pdf")).sort();
if (inputs.length < 6) {
  console.error(`expected at least 6 golden PDFs, found ${inputs.length}`);
  process.exit(1);
}

let failures = 0;
for (const file of inputs) {
  const name = file.replace(/\.pdf$/, "");
  const actual = wasm.analyse_pdf_structure(new Uint8Array(readFileSync(join(golden, "pdf", file))), undefined) + "\n";
  const expected = readFileSync(join(golden, "pdf_expected", `${name}.json`), "utf8");
  const digest = createHash("sha256").update(actual).digest("hex").slice(0, 16);
  if (actual === expected) {
    console.log(`ok   wasm   ${name.padEnd(20)} sha256=${digest}`);
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
  wasm.analyse_pdf_structure(new TextEncoder().encode("definitely not a pdf"), undefined);
  failures++;
  console.log("FAIL wasm   non-PDF input did not throw");
} catch (e) {
  if (e instanceof Error) console.log("ok   wasm   non-PDF input throws Error");
  else {
    failures++;
    console.log(`FAIL wasm   thrown value is not an Error: ${e}`);
  }
}

process.exit(failures ? 1 : 0);
