// Opens the built site in headless Chromium, loads every sample page and reads the fingerprint
// comparison the page itself shows ("This tab" against "Native build"). Not run in CI (Playwright is not
// a dependency of the repository); run it after scripts/build-site.sh:
//
//   python3 -m http.server 8765 --directory site &
//   node site/tools/browser-check.mjs http://localhost:8765/
//
// It also lists every request for the package, which must carry the build id (?v=...).
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)("playwright");

const url = process.argv[2] ?? "http://localhost:8765/";
const browser = await chromium.launch();
const page = await browser.newPage();
const pkgRequests = new Set();
page.on("request", (r) => { if (r.url().includes("/pkg/")) pkgRequests.add(r.url().replace(url, "")); });
page.on("console", (m) => { if (m.type() === "error") console.log("console error:", m.text()); });
await page.goto(url);
await page.waitForSelector("#picker button");
const ids = await page.$$eval("#picker button", (bs) => bs.map((b) => b.dataset.id));
let bad = 0;
for (const id of ids) {
  await page.click(`#picker button[data-id="${id}"]`);
  await page.waitForTimeout(4000);
  await page.waitForSelector(".proof .verdict", { timeout: 60000 });
  const hashes = await page.$$eval(".proof .hash", (hs) => hs.map((h) => h.title.slice(0, 12)));
  const same = hashes.length === 2 && hashes[0] === hashes[1];
  if (!same) bad++;
  console.log(id.padEnd(14), hashes.join(" vs "), same ? "IDENTICAL" : "DIFFERENT");
}
console.log("package requests:", [...pkgRequests].join(", "));
await browser.close();
process.exit(bad ? 1 : 0);
