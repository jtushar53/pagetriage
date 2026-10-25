// Module worker: every call into the analyser happens here, so the page never blocks.
// Messages in:  init | page {job, seq, rgba, width, height, dpi} | reanalyse {job, seq, dpi}
//               binarize {job, method} | pdf {job, bytes}
// Messages out: ready {version} | page-result {job, seq, json, sha, ms, gray}
//               mask {job, method, mask, w, h, ms} | pdf-result {job, json, sha, ms}
//               error {job, code, message}
import init, { analyse_page, analyse_pdf_structure, binarize, rgb_to_gray, version } from "./pkg/pagetriage.js";

const HOWE_STATIC = '[howe]\nmode = "static"'; // the dual mode takes about 23 s per megapixel

let loading = null;
let cur = null; // the page in memory: { job, gray, rgb, w, h }

function load() {
  if (!loading) {
    loading = init()
      .then(() => postMessage({ type: "ready", version: version() }))
      .catch((err) => {
        loading = null;
        throw err;
      });
  }
  return loading;
}

async function sha256Hex(text) {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

function fail(job, code, err) {
  postMessage({ type: "error", job, code, message: String((err && err.message) || err) });
}

async function analyse(job, seq, dpi) {
  const t0 = performance.now();
  const json = analyse_page(cur.gray, cur.rgb, cur.w, cur.h, dpi, undefined, "{}");
  const ms = Math.round(performance.now() - t0);
  const sha = await sha256Hex(json);
  postMessage({ type: "page-result", job, seq, json, sha, ms, gray: cur.rgb === undefined });
}

/** RGBA from a canvas to what the analyser wants: gray, plus RGB when the page has colour.
 *  Transparency is composited over white. A page whose pixels all have R = G = B is treated as an
 *  8-bit gray image, exactly as the native CLI treats a gray PNG: gray = R, no RGB passed. */
function unpack(rgba, w, h) {
  const n = w * h;
  const rgb = new Uint8Array(n * 3);
  let isGray = true;
  for (let i = 0, p = 0, q = 0; i < n; i++, p += 4, q += 3) {
    let r = rgba[p], g = rgba[p + 1], b = rgba[p + 2];
    const a = rgba[p + 3];
    if (a !== 255) {
      const bg = 255 - a;
      r = Math.round((r * a + 255 * bg) / 255);
      g = Math.round((g * a + 255 * bg) / 255);
      b = Math.round((b * a + 255 * bg) / 255);
    }
    if (r !== g || g !== b) isGray = false;
    rgb[q] = r; rgb[q + 1] = g; rgb[q + 2] = b;
  }
  if (isGray) {
    const gray = new Uint8Array(n);
    for (let i = 0, q = 0; i < n; i++, q += 3) gray[i] = rgb[q];
    return { gray, rgb: undefined };
  }
  return { gray: rgb_to_gray(rgb), rgb };
}

onmessage = async (e) => {
  const m = e.data;
  try {
    await load();
  } catch (err) {
    fail(m.job, "load", err);
    return;
  }
  try {
    switch (m.type) {
      case "init":
        return;
      case "page": {
        const { gray, rgb } = unpack(new Uint8ClampedArray(m.rgba), m.width, m.height);
        cur = { job: m.job, gray, rgb, w: m.width, h: m.height };
        await analyse(m.job, m.seq, m.dpi);
        return;
      }
      case "reanalyse":
        if (cur && cur.job === m.job) await analyse(m.job, m.seq, m.dpi);
        return;
      case "binarize":
        // Deferred one turn: a page or reanalyse message that is already queued runs first, and a
        // mask for a page the user has since replaced is skipped instead of holding up the new one.
        setTimeout(() => {
          if (!cur || cur.job !== m.job) return;
          try {
            const t0 = performance.now();
            const mask = binarize(cur.gray, cur.w, cur.h, m.method, m.method === "howe" ? HOWE_STATIC : undefined);
            const ms = Math.round(performance.now() - t0);
            postMessage({ type: "mask", job: m.job, method: m.method, mask: mask.buffer, w: cur.w, h: cur.h, ms }, [mask.buffer]);
          } catch (err) {
            fail(m.job, "binarize", err);
          }
        }, 0);
        return;
      case "pdf": {
        const t0 = performance.now();
        const json = analyse_pdf_structure(new Uint8Array(m.bytes), undefined);
        const ms = Math.round(performance.now() - t0);
        const sha = await sha256Hex(json);
        postMessage({ type: "pdf-result", job: m.job, json, sha, ms });
        return;
      }
      default:
        return;
    }
  } catch (err) {
    fail(m.job, m.type, err);
  }
};
