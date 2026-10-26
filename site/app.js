// pagetriage demo page. Everything that touches the analyser runs in worker.js; this file decodes
// files, draws the sheet and renders the slip. Untrusted strings (file names, producers, font
// names, report values) only ever reach the page through textContent.

const MAX_PIXELS = 36e6; // images above this are refused: a tab does not have the memory
const MAX_PDF_BYTES = 256 * 1024 * 1024; // the library's own cap
const HOWE_AUTO_PIXELS = 2.5e6; // Howe (static) runs by itself up to here, on request above
const JSON_INLINE_CAP = 200_000; // longer reports are offered as a download instead of inlined
const PDF_ROWS_SHOWN = 200;
const DOCS = "https://github.com/jtushar53/pagetriage/blob/main/docs/";

const METHODS = {
  otsu: { name: "Otsu", why: "One cut-off for the whole page. Fast, for clean pages." },
  sauvola: { name: "Sauvola", why: "A local cut-off that copes with uneven light." },
  wolf: { name: "Wolf", why: "A local cut-off that also adapts to low contrast." },
  howe: { name: "Howe", why: "Graph cut. Handles bleed-through and stains; the slowest." },
};
const STATES = {
  native_text: "Real text",
  raster_only: "Scan, no text",
  invisible_ocr_under_image: "Scan with hidden OCR text",
  mixed: "Text and scan mixed",
  vector_only: "Drawn lines, no text",
  empty: "Blank",
};

const $ = (sel, root = document) => root.querySelector(sel);

/** el("p.hint", {title: "..."}, "text", childNode, ...) */
function el(spec, attrs, ...children) {
  const [tag, ...classes] = spec.split(".");
  const node = document.createElement(tag || "div");
  if (classes.length) node.className = classes.join(" ");
  if (attrs) for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === "text") node.textContent = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else node.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    node.append(typeof c === "string" ? document.createTextNode(c) : c);
  }
  return node;
}
const svgIcon = (kind) => {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  s.setAttribute("viewBox", "0 0 20 20");
  s.setAttribute("aria-hidden", "true");
  const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
  p.setAttribute("fill", "none");
  p.setAttribute("stroke", "currentColor");
  p.setAttribute("stroke-width", "2");
  p.setAttribute("stroke-linecap", "round");
  p.setAttribute("stroke-linejoin", "round");
  p.setAttribute("d", kind === "ok"
    ? "M10 1.75a8.25 8.25 0 1 0 0 16.5 8.25 8.25 0 0 0 0-16.5Zm-3.75 8.5 2.5 2.5 5-5.5"
    : "M10 1.75a8.25 8.25 0 1 0 0 16.5 8.25 8.25 0 0 0 0-16.5ZM7 7l6 6M13 7l-6 6");
  s.append(p);
  return s;
};

const fmt = {
  deg: (d) => (Math.abs(d) < 0.1 ? "level" : `${d > 0 ? "+" : "−"}${Math.abs(d).toFixed(2)} deg, text rises to the ${d > 0 ? "right" : "left"}`),
  int: (n) => Math.round(n).toLocaleString("en-GB"),
  pct: (f, digits = 2) => `${(f * 100).toFixed(digits)}%`,
  bytes: (n) => (n < 1024 ? `${n} bytes` : n < 1024 * 1024 ? `${(n / 1024).toFixed(0)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`),
};

// ------------------------------------------------------------------ state

const state = {
  mode: "page",
  job: 0, // one per input; stale worker results are dropped
  seq: 0, // one per analysis of that input (dpi changes)
  results: { page: null, pdf: null },
  ready: false,
  version: null,
  samples: null,
};

const ui = {
  status: $("#status"),
  slipBody: $("#slip-body"),
  sheet: $("#sheet"),
  canvas: $("#page-canvas"),
  overlay: $("#overlay"),
  viewing: $("#viewing"),
  picker: $("#picker"),
  pdfList: $("#pdflist"),
  pdfDrop: $("#pdfdrop"),
  fileInputPage: $("#file-page"),
  fileInputPdf: $("#file-pdf"),
  tabs: Array.from(document.querySelectorAll('[role="tab"]')),
  panels: { page: $("#panel-page"), pdf: $("#panel-pdf") },
};

const setStatus = (text) => { ui.status.textContent = text; };

// ------------------------------------------------------------------ worker

let worker;
try {
  worker = new Worker("./worker.js", { type: "module" });
} catch (err) {
  showError("load", err);
}

function startWorker() {
  if (!worker) return;
  setStatus("Loading the analyser (0.6 MB, once)...");
  worker.postMessage({ type: "init" });
  worker.onmessage = (e) => onWorkerMessage(e.data);
  worker.onerror = (e) => {
    e.preventDefault();
    showError("worker", e.message || "the worker stopped");
  };
}

function onWorkerMessage(m) {
  switch (m.type) {
    case "ready":
      state.ready = true;
      state.version = m.version;
      for (const n of document.querySelectorAll("[data-version]")) n.textContent = m.version;
      return;
    case "page-result": {
      const r = state.results.page;
      if (!r || m.job !== r.job || m.seq !== r.seq) return;
      r.json = m.json; r.sha = m.sha; r.ms = m.ms; r.grayInput = m.gray;
      try { r.report = JSON.parse(m.json); } catch (err) { showError("page", err); return; }
      r.pending = false;
      ui.sheet.classList.remove("is-scanning");
      if (state.mode === "page") { renderSlip(); drawGuide(r); }
      setStatus(`Analysed in ${fmt.int(m.ms)} ms on your device.`);
      requestMasks(r);
      return;
    }
    case "mask": {
      const r = state.results.page;
      if (!r || m.job !== r.job) return;
      r.masks[m.method] = { canvas: maskToCanvas(new Uint8Array(m.mask), m.w, m.h), ms: m.ms };
      r.running.delete(m.method);
      if (state.mode === "page") {
        updateMethodTile(r, m.method);
        if (r.showing === m.method) showOnSheet(r, m.method);
      }
      return;
    }
    case "pdf-result": {
      const r = state.results.pdf;
      if (!r || m.job !== r.job) return;
      r.json = m.json; r.sha = m.sha; r.ms = m.ms;
      try { r.report = JSON.parse(m.json); } catch (err) { showError("pdf", err); return; }
      r.pending = false;
      if (state.mode === "pdf") renderSlip();
      setStatus(`Analysed in ${fmt.int(m.ms)} ms on your device.`);
      return;
    }
    case "error":
      if (m.job !== state.job && m.code !== "load") return;
      if (m.code === "binarize") {
        // The report is fine; only mark the tile.
        const r = state.results.page;
        if (r) { r.running.clear(); for (const k of Object.keys(METHODS)) if (!r.masks[k]) updateMethodTile(r, k); }
        setStatus(ERRORS.binarize[0]);
        return;
      }
      showError(m.code, m.message);
      return;
    default:
      return;
  }
}

// ------------------------------------------------------------------ errors

const ERRORS = {
  load: ["The analyser did not load.", "Reload the page. It needs WebAssembly and module workers: Chrome or Edge 80, Safari 15, Firefox 114, or later."],
  binarize: ["The black-and-white mask could not be made.", "The rest of the report stands. Try another method, or open an issue on GitHub with the image dimensions."],
  worker: ["The analyser stopped working.", "Reload the page and try the file again. If it happens twice, open an issue on GitHub with the file type and size."],
  tiff: ["This browser cannot open TIFF files.", "Convert the page to PNG or JPEG and try again, or use the command-line tool, which reads TIFF directly."],
  image: ["This file is not an image this browser can open.", "Use a PNG, JPEG, WebP, GIF or BMP page image, or a PDF. TIFF and other formats open with the command-line tool."],
  big_image: ["This image is too large for a tab.", "Pages over 36 megapixels are refused. Scan at 150 to 300 dpi, or use the command-line tool, which has no such cap."],
  not_pdf: ["This file is not a PDF.", "It has no %PDF header. Check the file, or switch to the Page image tab for a scanned image."],
  big_pdf: ["This PDF is too large.", "Files over 256 MB are refused, in the browser and in the library. Use the command-line tool on a split copy."],
  pdf: ["The PDF could not be read.", "The file may be damaged or truncated. The command-line tool prints the parser's reason."],
  page: ["The page could not be analysed.", "Try another image. If this one should work, open an issue on GitHub with the image dimensions."],
};

function showError(code, detail) {
  const [title, advice] = ERRORS[code] || ERRORS.worker;
  ui.sheet.classList.remove("is-scanning");
  setStatus("");
  const box = el("div.alert", { role: "alert" },
    el("h3", { text: title }),
    el("p", { text: advice }),
    detail && typeof detail !== "object" && !["tiff", "image", "not_pdf", "big_pdf"].includes(code)
      ? el("p.small", { text: code === "big_image" ? `This one is ${String(detail)}.` : `Detail: ${String(detail)}` }) : null);
  ui.slipBody.replaceChildren(box);
  if (code === "load" && ui.status) { ui.status.textContent = ""; }
}

// ------------------------------------------------------------------ tabs

function selectTab(mode, focus = false) {
  state.mode = mode;
  for (const t of ui.tabs) {
    const on = t.dataset.mode === mode;
    t.setAttribute("aria-selected", on ? "true" : "false");
    t.tabIndex = on ? 0 : -1;
    if (on && focus) t.focus();
  }
  for (const [m, p] of Object.entries(ui.panels)) p.hidden = m !== mode;
  renderSlip();
  const r = state.results[mode];
  setStatus(r && !r.pending && r.ms != null ? `Analysed in ${fmt.int(r.ms)} ms on your device.` : r && r.pending ? "Reading the page..." : "");
}
for (const t of ui.tabs) {
  t.addEventListener("click", () => selectTab(t.dataset.mode));
  t.addEventListener("keydown", (e) => {
    const i = ui.tabs.indexOf(t);
    let j = null;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") j = (i + 1) % ui.tabs.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") j = (i - 1 + ui.tabs.length) % ui.tabs.length;
    else if (e.key === "Home") j = 0;
    else if (e.key === "End") j = ui.tabs.length - 1;
    if (j === null) return;
    e.preventDefault();
    selectTab(ui.tabs[j].dataset.mode, true);
  });
}

// ------------------------------------------------------------------ input: files and samples

async function sniff(file) {
  const head = new Uint8Array(await file.slice(0, 5).arrayBuffer());
  return String.fromCharCode(...head) === "%PDF-" ? "pdf" : "image";
}

/** `expect` is "pdf" when the file came through the PDF input or zone: a non-PDF there is told so. */
async function handleFile(file, expect) {
  if (!file) return;
  const kind = await sniff(file);
  if (kind !== "pdf" && expect === "pdf") {
    selectTab("pdf");
    showError("not_pdf");
    return;
  }
  if (kind === "pdf") {
    selectTab("pdf");
    markPressed(ui.pdfList, null);
    await analysePdf({ bytes: await file.arrayBuffer(), title: file.name, blurb: `Your file, ${fmt.bytes(file.size)}.`, native: null, size: file.size });
  } else {
    selectTab("page");
    markPressed(ui.picker, null);
    await analyseImage({ blob: file, title: file.name, blurb: null, built: null, native: null, dpi: Number(currentDpi()) || 150 });
  }
}

function markPressed(container, id) {
  for (const b of container.querySelectorAll("button[aria-pressed]")) b.setAttribute("aria-pressed", b.dataset.id === id ? "true" : "false");
}

ui.fileInputPage.addEventListener("change", () => { handleFile(ui.fileInputPage.files[0]); ui.fileInputPage.value = ""; });
ui.fileInputPdf.addEventListener("change", () => { handleFile(ui.fileInputPdf.files[0], "pdf"); ui.fileInputPdf.value = ""; });

// Drag and drop on the sheet and the PDF zone; a miss anywhere else must not navigate away.
for (const zone of [ui.sheet, ui.pdfDrop]) {
  zone.addEventListener("dragenter", (e) => { e.preventDefault(); zone.classList.add("is-dragover"); });
  zone.addEventListener("dragover", (e) => { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; zone.classList.add("is-dragover"); });
  zone.addEventListener("dragleave", (e) => { if (!zone.contains(e.relatedTarget)) zone.classList.remove("is-dragover"); });
  zone.addEventListener("drop", (e) => {
    e.preventDefault();
    zone.classList.remove("is-dragover");
    const file = e.dataTransfer.files && e.dataTransfer.files[0];
    if (file) handleFile(file, zone === ui.pdfDrop ? "pdf" : undefined);
  });
}
document.addEventListener("dragover", (e) => e.preventDefault());
document.addEventListener("drop", (e) => e.preventDefault());

// ------------------------------------------------------------------ page images

async function decodeImage(blob) {
  try {
    return await createImageBitmap(blob, { colorSpaceConversion: "none" });
  } catch (_) {
    return createImageBitmap(blob); // older browsers reject the options bag
  }
}

/** Width and height from the PNG or JPEG header, so an oversized image is refused before the
 *  browser spends the memory decoding it. Null when the format is something else. */
async function headerSize(blob) {
  const b = new Uint8Array(await blob.slice(0, 65536).arrayBuffer());
  if (b.length > 24 && b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) {
    const v = new DataView(b.buffer);
    return { w: v.getUint32(16), h: v.getUint32(20) };
  }
  if (b.length > 4 && b[0] === 0xff && b[1] === 0xd8) {
    let i = 2;
    while (i + 9 < b.length) {
      if (b[i] !== 0xff) { i++; continue; }
      const marker = b[i + 1];
      if (marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7) || marker === 0x01 || marker === 0xff) { i += marker === 0xff ? 1 : 2; continue; }
      const len = (b[i + 2] << 8) | b[i + 3];
      const sof = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
      if (sof) return { h: (b[i + 5] << 8) | b[i + 6], w: (b[i + 7] << 8) | b[i + 8] };
      if (marker === 0xda) break; // start of scan: no frame header found before the data
      i += 2 + len;
    }
  }
  return null;
}

async function analyseImage(src) {
  const job = ++state.job;
  const seq = ++state.seq;
  const r = {
    kind: "page", job, seq, pending: true, title: src.title, blurb: src.blurb, built: src.built, native: src.native,
    nativeDpi: src.dpi, dpi: src.dpi, masks: {}, running: new Set(), showing: null, report: null, json: null, sha: null, ms: null, w: 0, h: 0,
  };
  state.results.page = r;
  setStatus("Reading the page...");
  renderSlip();

  const declared = await headerSize(src.blob).catch(() => null);
  if (job !== state.job) return;
  if (declared && declared.w * declared.h > MAX_PIXELS) {
    showError("big_image", `${declared.w} by ${declared.h} pixels`);
    state.results.page = null;
    return;
  }
  let bitmap;
  try {
    bitmap = await decodeImage(src.blob);
  } catch (_) {
    if (job !== state.job) return;
    const name = (src.title || "").toLowerCase();
    showError(/\.tiff?$/.test(name) || /tiff$/.test(src.blob.type || "") ? "tiff" : "image");
    state.results.page = null;
    return;
  }
  if (job !== state.job) { bitmap.close(); return; }
  const { width: w, height: h } = bitmap;
  if (w * h > MAX_PIXELS) {
    bitmap.close();
    showError("big_image", `${w} by ${h} pixels`);
    state.results.page = null;
    return;
  }
  r.w = w; r.h = h;
  if (!src.blurb) r.blurb = `Your file, ${w} by ${h} pixels.`;

  const full = document.createElement("canvas");
  full.width = w; full.height = h;
  const ctx = full.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(bitmap, 0, 0);
  bitmap.close();
  const data = ctx.getImageData(0, 0, w, h);
  r.original = full;
  showOnSheet(r, null);
  ui.sheet.classList.remove("is-scanning");
  void ui.sheet.offsetWidth; // restart the sweep when a new file arrives mid-animation
  ui.sheet.classList.add("is-scanning");
  drawGuide(null);
  if (state.mode === "page") renderSlip();
  worker.postMessage({ type: "page", job, seq, rgba: data.data.buffer, width: w, height: h, dpi: r.dpi }, [data.data.buffer]);
}

function reanalyse(dpi) {
  const r = state.results.page;
  if (!r || r.pending || !r.report) return;
  r.dpi = dpi;
  r.seq = ++state.seq;
  r.pending = true;
  setStatus("Reading the page...");
  ui.sheet.classList.add("is-scanning");
  worker.postMessage({ type: "reanalyse", job: r.job, seq: r.seq, dpi });
}

function requestMasks(r) {
  const want = ["otsu", "sauvola", "wolf"];
  if (r.w * r.h <= HOWE_AUTO_PIXELS) want.push("howe");
  for (const m of want) requestMask(r, m);
}
function requestMask(r, method) {
  if (r.masks[method] || r.running.has(method)) return;
  r.running.add(method);
  worker.postMessage({ type: "binarize", job: r.job, method });
  if (state.mode === "page") updateMethodTile(r, method);
}

function maskToCanvas(mask, w, h) {
  const c = document.createElement("canvas");
  c.width = w; c.height = h;
  const ctx = c.getContext("2d");
  const img = ctx.createImageData(w, h);
  const px = new Uint32Array(img.data.buffer);
  for (let i = 0; i < mask.length; i++) px[i] = mask[i] ? 0xff000000 : 0xffffffff; // little-endian ABGR
  ctx.putImageData(img, 0, 0);
  return c;
}

/** Draw the original or a mask on the sheet; `method` null means the original. */
function showOnSheet(r, method) {
  const source = method ? r.masks[method] && r.masks[method].canvas : r.original;
  if (!source) return;
  r.showing = method;
  const { w, h } = r;
  const scale = Math.min(1, 1600 / Math.max(w, h));
  const dw = Math.round(w * scale), dh = Math.round(h * scale);
  if (ui.canvas.width !== dw || ui.canvas.height !== dh) { ui.canvas.width = dw; ui.canvas.height = dh; }
  ui.sheet.style.setProperty("--ratio", `${w} / ${h}`);
  const ctx = ui.canvas.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(source, 0, 0, dw, dh);
  ui.overlay.style.display = method ? "none" : "";
  ui.viewing.textContent = method ? `Showing the ${METHODS[method].name} black-and-white mask. Click it again for the original.` : "The original scan";
  ui.canvas.setAttribute("aria-label", `${r.title}: ${method ? `the ${METHODS[method].name} black-and-white mask` : "the original scan"}`);
  for (const b of ui.slipBody.querySelectorAll(".method")) b.setAttribute("aria-pressed", b.dataset.method === method ? "true" : "false");
}

/** The skew guide: a dashed level line and a solid one at the measured angle. Positive = text
 *  rises to the right (counter-clockwise), so the right end goes up on screen. */
function drawGuide(r) {
  const svg = ui.overlay;
  const skew = r && r.report ? r.report.quality.skew_deg : 0;
  if (!r || Math.abs(skew) < 0.1) { svg.replaceChildren(); return; }
  const { w, h } = r;
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  const cx = w / 2, cy = h / 2, L = w * 0.42, t = Math.tan((skew * Math.PI) / 180);
  const line = (cls, x1, y1, x2, y2) => {
    const n = document.createElementNS("http://www.w3.org/2000/svg", "line");
    n.setAttribute("class", cls);
    n.setAttribute("x1", x1); n.setAttribute("y1", y1); n.setAttribute("x2", x2); n.setAttribute("y2", y2);
    return n;
  };
  svg.replaceChildren(line("ref", cx - L, cy, cx + L, cy), line("guide", cx - L, cy + L * t, cx + L, cy - L * t));
}

// ------------------------------------------------------------------ PDFs

async function analysePdf(src) {
  const job = ++state.job;
  if (src.size > MAX_PDF_BYTES) { showError("big_pdf"); return; }
  const r = { kind: "pdf", job, pending: true, title: src.title, blurb: src.blurb, native: src.native, size: src.size, report: null, json: null, sha: null, ms: null };
  state.results.pdf = r;
  setStatus("Reading the file...");
  renderSlip();
  const head = String.fromCharCode(...new Uint8Array(src.bytes.slice(0, 5)));
  if (head !== "%PDF-") { showError("not_pdf"); state.results.pdf = null; return; }
  worker.postMessage({ type: "pdf", job, bytes: src.bytes }, [src.bytes]);
}

// ------------------------------------------------------------------ the slip

function renderSlip() {
  const r = state.results[state.mode];
  if (!r) {
    ui.slipBody.replaceChildren(el("p.empty", {
      text: state.mode === "pdf"
        ? "Pick a sample PDF on the left, or drop your own. The readout appears here."
        : "Pick a sample page on the left, or drop your own. The readout appears here.",
    }));
    return;
  }
  if (r.pending && !r.report) {
    ui.slipBody.replaceChildren(slipHead(r, null));
    return;
  }
  ui.slipBody.replaceChildren(r.kind === "page" ? pageSlip(r) : pdfSlip(r));
}

function slipHead(r, grade) {
  const head = el("div.slip__head");
  if (grade) head.append(el("div.grade", { "data-grade": grade, role: "img", "aria-label": `Grade ${grade}. A is best, D is worst.`, text: grade }));
  const titleBox = el("div");
  titleBox.append(el("h2", { text: r.title }));
  if (r.blurb) titleBox.append(el("p.slip__blurb", { text: r.blurb }));
  if (r.built) titleBox.append(el("p.slip__built", null, el("b", { text: "How it was made: " }), r.built));
  head.append(titleBox);
  return head;
}

function row(label, value, hint) {
  const dd = el("dd", null, typeof value === "string" ? el("span.num", { text: value }) : value);
  if (hint) dd.append(el("span.hint", { text: hint }));
  return el("div", null, el("dt", { text: label }), dd);
}

function pageSlip(r) {
  const q = r.report.quality;
  const frag = document.createDocumentFragment();
  frag.append(slipHead(r, q.grade));

  const rec = METHODS[q.recommended_binarizer] || { name: q.recommended_binarizer, why: "" };
  const dl = el("dl.measures");
  dl.append(
    row("Grade", `Grade ${q.grade}`, "A is best, D is worst."),
    row("Tilt", fmt.deg(q.skew_deg), Math.abs(q.skew_deg) >= 0.1 ? `Rotate by ${(-q.skew_deg).toFixed(2)} degrees to straighten it. The violet line on the sheet follows the text.` : "Under 0.1 degrees."),
    row("Sharpness", fmt.int(q.blur_laplacian_var), "Variance of the Laplacian. Higher is sharper."),
    row("Contrast", q.contrast_rms.toFixed(3), "RMS contrast, 0 to 1."),
    row("Bleed-through", q.bleed_through.toFixed(2), "0 none, 1 strong."),
    row("Fold lines", q.fold_lines === 0 ? "none" : String(q.fold_lines), q.fold_lines === 0 ? null : "Creases that cross the page."),
    row("Specks", fmt.pct(q.speckle_density, 3), "Share of pixels that are isolated dark dots."),
    row("Black and white", rec.name, rec.why),
  );
  frag.append(dl);

  // Binariser tiles: click shows that mask on the sheet, click again returns the original.
  const methods = el("div.methods", { role: "group", "aria-label": "Black-and-white masks" });
  for (const m of Object.keys(METHODS)) methods.append(methodTile(r, m, m === q.recommended_binarizer));
  frag.append(methods);

  frag.append(proofBlock(r));

  const dpi = el("input", { type: "number", id: "dpi", min: 50, max: 2400, step: 1, value: String(r.dpi), inputmode: "numeric" });
  dpi.addEventListener("change", () => {
    const v = Math.min(2400, Math.max(50, Math.round(Number(dpi.value) || 150)));
    dpi.value = String(v);
    if (v !== r.dpi) reanalyse(v);
  });
  frag.append(el("div.tools", null,
    el("label", { for: "dpi" }, "Scan resolution (dpi)", dpi),
    el("button.btn", { type: "button", onclick: () => download(r) }, "Download report"),
  ));
  frag.append(rawJson(r));
  return frag;
}

function methodTile(r, method, recommended) {
  const info = METHODS[method];
  const b = el("button.method", { type: "button", "data-method": method, "aria-pressed": r.showing === method ? "true" : "false" });
  const thumb = el("span.thumb", null, el("canvas", { "aria-hidden": "true", width: 1, height: 1 }), el("span.pending"));
  b.append(thumb, el("strong", { text: info.name }), el("span.meta"), el("span.why", { text: info.why }));
  if (recommended) b.append(el("span.tag", { text: "Recommended" }));
  b.addEventListener("click", () => {
    if (!r.masks[method]) { requestMask(r, method); return; }
    showOnSheet(r, r.showing === method ? null : method);
  });
  queueMicrotask(() => updateMethodTile(r, method, b));
  return b;
}

function updateMethodTile(r, method, tile) {
  const b = tile || ui.slipBody.querySelector(`.method[data-method="${method}"]`);
  if (!b) return;
  const canvas = $("canvas", b), pending = $(".pending", b), meta = $(".meta", b);
  const m = r.masks[method];
  if (m) {
    const tw = 160, th = Math.max(1, Math.round((tw * r.h) / r.w));
    canvas.width = tw; canvas.height = th;
    const ctx = canvas.getContext("2d");
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(m.canvas, 0, 0, tw, th);
    canvas.style.display = "";
    pending.textContent = "";
    meta.textContent = `${fmt.int(m.ms)} ms`;
    b.setAttribute("aria-label", `${METHODS[method].name}: show this mask on the sheet`);
  } else {
    canvas.style.display = "none";
    const running = r.running.has(method);
    pending.textContent = running ? "Running" : "Not run";
    // Howe static ran at about 4 s per megapixel in Chrome on an Apple-silicon laptop.
    meta.textContent = running ? "" : method === "howe" ? `Click to run. About ${Math.max(1, Math.round(((r.w * r.h) / 1e6) * 4))} s for this page.` : "Click to run.";
    b.setAttribute("aria-label", running ? `${METHODS[method].name}: running` : `${METHODS[method].name}: run this method`);
  }
}

function proofBlock(r) {
  const box = el("section.proof", { "aria-labelledby": "proof-h" });
  box.append(el("h3", { id: "proof-h", text: "Report fingerprint (SHA-256)" }));
  const hash = (v) => el("span.hash", { title: v, text: v ? `${v.slice(0, 16)}…` : "" });
  box.append(el("div.row", null, el("span", { text: "This tab" }), hash(r.sha)));
  if (r.native && r.nativeDpi !== undefined && r.dpi !== r.nativeDpi) {
    box.append(el("div.row", null, el("span", { text: "Native build" }), el("span", { text: `recorded at ${r.nativeDpi} dpi` })));
    box.append(el("p.note", { text: `The stored reference is for ${r.nativeDpi} dpi. Set the resolution back to compare fingerprints; the report above is still deterministic at ${r.dpi} dpi.` }));
  } else if (r.native) {
    box.append(el("div.row", null, el("span", { text: "Native build" }), hash(r.native)));
    const ok = r.sha === r.native;
    box.append(el("p.verdict", { class: `verdict ${ok ? "is-ok" : "is-bad"}` }, svgIcon(ok ? "ok" : "bad"),
      el("span", { text: ok ? "Identical to the native build." : "Different from the native build." })));
    box.append(el("p.note", {
      text: ok
        ? "Byte for byte the report the native library produced for this file. Hover a hash for the full value, or open the JSON."
        : "This should never happen. Check that site/pkg was built from the same commit as samples.json (scripts/build-site.sh checks this), then open an issue with your browser's name and version.",
    }));
  } else {
    box.append(el("p.verdict.is-ok", null, svgIcon("ok"), el("span", { text: r.kind === "pdf" ? "Same file in, same fingerprint out, on every build." : "Same pixels in, same fingerprint out, on every build." })));
    box.append(el("p.note", {
      text: r.kind === "pdf"
        ? "There is no stored reference for your own file. The command-line tool's structure command and the Python binding's analyse_pdf_structure print this same report; hash that output without its trailing newline to compare."
        : "There is no stored reference for your own file. Different JPEG decoders can produce slightly different pixels from the same file, so to compare with the native build use a PNG.",
    }));
  }
  return box;
}

function rawJson(r) {
  const d = el("details.raw");
  d.append(el("summary", { text: "Full report (JSON)" }));
  if (r.json.length > JSON_INLINE_CAP) {
    d.append(el("p", { text: `The report is ${fmt.bytes(r.json.length)}, too long to show here. Download it instead.` }));
  } else {
    d.append(el("pre", null, el("code", { text: JSON.stringify(r.report, null, 1) })));
  }
  return d;
}

function download(r) {
  const base = (r.title || "report").replace(/\.[a-z0-9]+$/i, "").replace(/[^\w.-]+/g, "_") || "report";
  const blob = new Blob([r.json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = el("a", { href: url, download: `${base}.pagetriage.json` });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

function pdfSlip(r) {
  const rep = r.report;
  const doc = rep.document || {}, st = rep.structure || {};
  const frag = document.createDocumentFragment();
  frag.append(slipHead(r, null));

  if (st.encrypted) {
    frag.append(el("div.alert", { role: "alert" },
      el("h3", { text: "This PDF is encrypted." }),
      el("p", { text: "The empty password does not open it, so no pages were analysed. The report records the fact rather than guessing." })));
  }
  for (const w of rep.warnings || []) if (!st.encrypted) frag.append(el("p.notice", { text: w }));

  const updates = doc.incremental_updates || 0;
  const edited = updates === 0 ? "No"
    : `Yes: ${updates} update${updates === 1 ? "" : "s"} appended after the first save${st.content_changed_in_update ? ", and page content changed in one of them" : ""}${st.signature_count ? `; the file carries ${st.signature_count} signature${st.signature_count === 1 ? "" : "s"}` : ""}.`;
  const dl = el("dl.measures");
  dl.append(
    row("Pages", fmt.int(doc.pages || 0)),
    row("Producer", doc.producer ? doc.producer : "Not stated", st.creator && st.creator !== doc.producer ? `Creator: ${st.creator}` : null),
    row("Edited after first save", edited, st.tamper_signal && st.tamper_reasons && st.tamper_reasons.length ? st.tamper_reasons.join(" ") : null),
    row("OCR producer", st.ocr_producer ? `Yes${st.producer_family ? `: ${st.producer_family}` : ""}` : "No", st.ocr_producer ? "Matched by name. The text layer is the OCR's guess." : null),
    row("Encrypted", st.encrypted ? "Yes" : "No"),
  );
  frag.append(dl);

  const pages = rep.pages || [];
  if (pages.length) {
    const table = el("table.pages");
    table.append(el("caption", { text: "Page by page" }));
    table.append(el("thead", null, el("tr", null,
      el("th", { scope: "col", text: "Page" }), el("th", { scope: "col", text: "Text layer" }),
      el("th", { scope: "col", text: "Script" }), el("th", { scope: "col", text: "Notes" }))));
    const tbody = el("tbody");
    for (const p of pages.slice(0, PDF_ROWS_SHOWN)) tbody.append(pdfRow(p));
    table.append(tbody);
    frag.append(table);
    if (pages.length > PDF_ROWS_SHOWN) frag.append(el("p.more", { text: `Showing the first ${PDF_ROWS_SHOWN} of ${fmt.int(pages.length)} pages. The JSON has all of them.` }));
  }

  frag.append(proofBlock(r));
  frag.append(el("div.tools", null, el("button.btn", { type: "button", onclick: () => download(r) }, "Download report")));
  frag.append(rawJson(r));
  return frag;
}

function pdfRow(p) {
  const tl = p.text_layer || {}, sc = p.script || {};
  const tdText = el("td", null, el("span.state", { text: STATES[tl.state] || tl.state || "Unknown" }));
  if (typeof tl.confidence === "number") tdText.append(el("span.small", { text: `${Math.round(tl.confidence * 100)}% confidence${tl.reliable === false ? ". Do not trust the text." : ""}` }));

  const tdScript = el("td");
  const name = (sc.name || "").replace(/\s*\(legacy-encoded\)$/, "");
  tdScript.append(el("span.state", { text: !name || name === "None" ? "No text" : name }));
  if (name === "Unknown" && sc.glyphs_undecodable > 0) {
    tdScript.append(el("span.small", { text: `${fmt.int(sc.glyphs_undecodable)} glyph${sc.glyphs_undecodable === 1 ? "" : "s"} could not be decoded, so the script is not known.` }));
  }
  if (sc.legacy_encoded) {
    tdScript.append(el("span.legacy", { text: "Legacy font" }));
    for (const f of sc.fonts || []) if (f.legacy_encoded) tdScript.append(el("span.small", { text: `${f.name} looks like Hindi on screen but copies out as Latin letters.` }));
  }

  const notes = [...(p.notes || []), ...(p.conflicts || [])];
  const tdNotes = el("td");
  if (p.images && p.images.length) {
    const j = p.images.find((i) => i.jpeg);
    notes.push(`${p.images.length} image${p.images.length === 1 ? "" : "s"}${j && j.jpeg.luma_quality != null ? `, JPEG quality ${j.jpeg.luma_quality}` : ""}.`);
  }
  if (p.covered_text && p.covered_text.length) {
    const white = p.covered_text.filter((c) => c.fill === "white").length, n = p.covered_text.length;
    notes.push(`${n} box${n === 1 ? "" : "es"} painted over text that is still in the file${white ? ` (${white} white)` : ""}: the text can be copied out from under ${n === 1 ? "it" : "them"}.`);
  }
  if (p.covered_text && p.covered_text.length) {
    const white = p.covered_text.filter((c) => c.fill === "white").length, n = p.covered_text.length;
    notes.push(`${n} box${n === 1 ? "" : "es"} painted over text that is still in the file${white ? ` (${white} white)` : ""}: the text can be copied out from under ${n === 1 ? "it" : "them"}.`);
  }
  for (const n of notes) tdNotes.append(el("span.small", { text: n }));
  return el("tr", null, el("td", { text: String((p.index || 0) + 1) }), tdText, tdScript, tdNotes);
}

function currentDpi() {
  const r = state.results.page;
  return r ? r.dpi : 150;
}

// ------------------------------------------------------------------ samples, ledger, boot

async function loadSample(p) {
  markPressed(ui.picker, p.id);
  const res = await fetch(p.file);
  const blob = await res.blob();
  await analyseImage({ blob, title: p.title, blurb: p.blurb, built: p.built, native: p.native_report_sha256, dpi: p.dpi });
}

async function loadSamplePdf(p) {
  markPressed(ui.pdfList, p.id);
  const res = await fetch(p.file);
  const bytes = await res.arrayBuffer();
  await analysePdf({ bytes, title: p.title, blurb: p.blurb, native: p.native_report_sha256, size: bytes.byteLength });
}

function buildPicker(samples) {
  ui.picker.replaceChildren(...samples.pages.map((p) =>
    el("button", { type: "button", "aria-pressed": "false", "data-id": p.id, onclick: () => loadSample(p) },
      el("img", { src: p.thumb, alt: "", width: 56, height: 79, loading: "lazy" }),
      el("span", { text: p.title }))));
  ui.pdfList.replaceChildren(...samples.pdfs.map((p) =>
    el("li", null, el("button", { type: "button", "aria-pressed": "false", "data-id": p.id, onclick: () => loadSamplePdf(p) },
      el("strong", { text: p.title }), el("span", { text: p.blurb })))));
}

function buildLedger(ledger) {
  const tbody = $("#ledger-body");
  if (!tbody) return;
  tbody.replaceChildren(...ledger.rows.map((row) => {
    const status = row.status === "working" ? "Working" : "Planned";
    const label = row.status === "working" ? "Detector notes" : "Module notes";
    const link = row.doc ? el("a", { href: (ledger.docs_base || DOCS) + row.doc, text: label, "aria-label": `${label}: ${row.question}` }) : null;
    return el("tr", null,
      el("td", { "data-label": "Question", text: row.question }),
      el("td", { "data-label": "How", text: row.how }),
      el("td", { "data-label": "Status" }, el("span", { class: `status-word ${row.status}`, text: status })),
      el("td", { "data-label": "Details" }, link));
  }));
}

async function boot() {
  startWorker();
  selectTab("page");
  try {
    const [samples, ledger] = await Promise.all([
      fetch("samples/samples.json").then((r) => r.json()),
      fetch("ledger.json").then((r) => r.json()),
    ]);
    state.samples = samples;
    buildPicker(samples);
    buildLedger(ledger);
    const first = samples.pages.find((p) => p.id === "tilted") || samples.pages[0];
    if (first) await loadSample(first);
  } catch (err) {
    showError("worker", err);
  }
}

boot();
