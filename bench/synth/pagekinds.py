"""Builders for each page kind. See `pdfparts.PageBuild`.

Label rules, used consistently here and documented in bench/README.md:

* `script` is the script of the page's text layer (for legacy-font pages, the script the legacy
  font stands for). Pages without a text layer (raster_only, vector_only, empty) get "none".
* `invisible_ocr_under_image` is a full-page image plus a text layer that is not meant to be seen.
* `mixed` is visible native text together with a partial-page image.
"""
from __future__ import annotations

import random
from dataclasses import dataclass

import numpy as np
from PIL import Image

import pdfparts as pp
import scanimg
import synthtext as st
from pdfio import PdfFile, fmt
from pdfparts import (LABEL_OF, LATIN_FONTS, LEGACY_FONT_NAMES, FontPool, PageBuild, PagePlan, content_obj,
                      contents_entry, emit_lines, expected_from, flow_text, make_geo, resources)


@dataclass
class Style:
    latin_fonts: list[str]
    img_mix: dict[str, float]
    ocr_style: str            # glyphless | word_simple
    compress_p: float         # probability that content streams are Flate-compressed
    group: bool = False       # add /Group (transparency) to pages, as Word and browsers do


@dataclass
class Ctx:
    f: PdfFile
    pool: FontPool
    style: Style
    seed: int


def label(kind: str, script: str, legacy: bool) -> dict:
    return {"text_layer": LABEL_OF[kind], "script": script, "legacy_font": legacy}


def compress(ctx: Ctx, rng: random.Random) -> bool:
    return rng.random() < ctx.style.compress_p


# ---- native text --------------------------------------------------------------------------------

def _legacy_font(ctx: Ctx, rng: random.Random):
    fam = rng.choice(LEGACY_FONT_NAMES)
    tag = "".join(rng.choice("ABCDEFGHIJKLMNOPQRSTUVWXYZ") for _ in range(6))
    base = rng.choice([fam, fam.replace(" ", ""), f"{tag}+{fam.replace(' ', '')}"])
    enc = rng.choice(["WinAnsiEncoding", None])
    return ctx.pool.simple(base, "TrueType", True, True, 0.5, flags=rng.choice([4, 32]), family=fam,
                           encoding=enc), fam


def build_native(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    w, h = size
    geo = make_geo(rng, w, h)
    script, legacy = plan.script, plan.legacy
    rtl = script == "arabic"
    fs = rng.choice([9, 9.5, 10, 10.5, 11, 12, 13])
    detail: dict = {}
    title_text = None
    if legacy:
        body, fam = _legacy_font(ctx, rng)
        fs = rng.choice([11, 12, 13, 14, 16])
        avg = 0.5
        wf = None
        detail["legacy_font_family"] = fam
        if plan.variant == "english_heading":
            heading = ctx.pool.latin(rng.choice(ctx.style.latin_fonts), True)
            bold = heading
            title_text = rng.choice(["DEED OF SALE", "SALE DEED", "AFFIDAVIT", "POWER OF ATTORNEY"])
        else:
            bold = body
    elif script == "latin":
        key = rng.choice(ctx.style.latin_fonts)
        body, bold = ctx.pool.latin(key), ctx.pool.latin(key, True)
        avg, wf = LATIN_FONTS[key][5], None
    else:
        body = bold = ctx.pool.cid(script)
        avg, wf = 0.62, body.width
    columns = 2 if (w >= 500 and not rtl and rng.random() < 0.18) else 1
    fill = rng.choice([rng.uniform(0.25, 0.6), rng.uniform(0.6, 1.0), 1.0])
    lines, _ = flow_text(rng, script, legacy, geo, fs, avg, columns, fill, rng.random() < 0.7,
                         rng.random() < 0.5, rtl=rtl, width_fn=wf, title_text=title_text)
    res = {False: ("F1", body), True: ("F2", bold)}
    ops = emit_lines(lines, res, rng, per_line_bt=rng.random() < 0.6, tj_p=0.0 if rtl else 0.2)
    fonts = {"F1": body.obj, "F2": bold.obj}
    contents = contents_entry(ctx.f, ops, compress(ctx, rng), rng, split_ok=True)
    entries = {"Contents": contents, "Resources": resources(fonts, procset=rng.random() < 0.5)}
    detail.update({"expected_text": expected_from(lines), "columns": columns, "rtl": rtl,
                   "script_basis": "font_name" if legacy else "text_layer"})
    return PageBuild(entries, label("native", script, legacy), detail)


# ---- images shared by raster / ocr / mixed ------------------------------------------------------

ENC_DPI = {"dct_gray": [72, 100, 120, 150], "dct_rgb": [72, 100, 120, 150], "flate_gray": [60, 72, 96],
           "flate_pred_gray": [60, 72, 96], "flate_rgb": [60, 72], "flate_1bit": [100, 150, 200],
           "ccitt": [100, 150, 200]}
MAX_PIXELS = 3_200_000


def pick_scan_format(rng: random.Random, style: Style, w: float, h: float,
                     force: str | None = None) -> tuple[str, int]:
    enc = force or rng.choices(list(style.img_mix), weights=list(style.img_mix.values()))[0]
    dpi = rng.choice(ENC_DPI[enc])
    while dpi > 40 and round(w * dpi / 72) * round(h * dpi / 72) > MAX_PIXELS:
        dpi = int(dpi * 0.8)
    return enc, dpi


def encode_image(rng: random.Random, enc: str, img: Image.Image) -> tuple[str, bytes, dict]:
    """`img` is mode L. Returns (stream entries, data, detail)."""
    if enc == "dct_gray":
        return scanimg.enc_dct(img, rng.choice([20, 35, 50, 65, 75, 85, 92]))
    if enc == "dct_rgb":
        return scanimg.enc_dct(scanimg.tint_paper(img, rng), rng.choice([25, 40, 55, 70, 85]))
    if enc == "flate_gray":
        return scanimg.enc_flate(img, rng.choice([1, 6, 9]), False)
    if enc == "flate_pred_gray":
        return scanimg.enc_flate(img, 6, True)
    if enc == "flate_rgb":
        return scanimg.enc_flate(scanimg.tint_paper(img, rng), 6, rng.random() < 0.5)
    b = scanimg.to_bilevel(img, rng.choice([110, 128, 150]))
    if enc == "flate_1bit":
        return scanimg.enc_flate_1bit(b, 6)
    return scanimg.enc_ccitt(b)


def scan_params(rng: random.Random, enc: str, dpi: int, max_skew: float) -> dict:
    light = enc.startswith("flate") and "1bit" not in enc
    return dict(skew_deg=rng.choice([0.0, rng.uniform(-0.4, 0.4), rng.uniform(-max_skew, max_skew)]),
                noise=rng.uniform(0.5, 3) if light else rng.uniform(2, 11),
                speckle=rng.choice([0.0, 0.0001, 0.0006, 0.002]),
                blur=min(1.2, rng.uniform(0.3, 1.0) * max(0.5, dpi / 120)),
                shade=rng.uniform(0.0, 0.22), ink_level=rng.choice([20, 40, 70]))


def content_script(plan: PagePlan, rng: random.Random) -> str:
    if plan.script in ("latin",) or plan.script in st.BLOCK:
        return plan.script
    return "latin"


def make_scan(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size, enc, dpi,
              text: bool = True, max_skew: float = 1.8) -> tuple[scanimg.Scan, tuple[float, float]]:
    w, h = size
    w_px, h_px = round(w * dpi / 72), round(h * dpi / 72)
    media = (round(w_px * 72 / dpi, 2), round(h_px * 72 / dpi, 2))
    params = scan_params(rng, enc, dpi, max_skew)
    script = content_script(plan, rng)
    paras = []
    if text:
        for _ in range(rng.randint(3, 9)):
            paras.append(st.paragraph(rng, script))
    margins = tuple(rng.uniform(0.07, 0.16) for _ in range(4))
    scan = scanimg.render_scan(rng, np_rng, w_px, h_px, dpi, script, paras, rng.choice([9, 10, 11, 12]),
                               margins, title=st.title(rng, script) if text and rng.random() < 0.6 else None,
                               **params)
    return scan, media


def image_xobj(ctx: Ctx, rng: random.Random, enc: str, img: Image.Image) -> tuple[int, dict]:
    entries, data, meta = encode_image(rng, enc, img)
    meta = dict(meta)
    meta.update({"width": img.width, "height": img.height})
    return ctx.f.add_stream(entries, data), meta


# ---- raster-only --------------------------------------------------------------------------------

def build_raster(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    enc, dpi = pick_scan_format(rng, ctx.style, *size, force=plan.enc)
    variant = plan.variant or rng.choices(["text", "blank", "photo", "strips"], [74, 6, 6, 14])[0]
    detail: dict = {"variant": variant, "dpi": dpi}
    if variant == "blank":
        w_px, h_px = round(size[0] * dpi / 72), round(size[1] * dpi / 72)
        scan = scanimg.blank_scan(np_rng, rng, w_px, h_px, dpi, rng.uniform(2, 8), rng.choice([0.0002, 0.001]), 0.1)
        media = (round(w_px * 72 / dpi, 2), round(h_px * 72 / dpi, 2))
        img = scan.image
    elif variant == "photo":
        scan, media = make_scan(ctx, plan, rng, np_rng, size, enc, dpi, text=False)
        w_px, h_px = scan.image.size
        img = scanimg.stamp_image(rng, np_rng, w_px, h_px, False).convert("L")
    else:
        scan, media = make_scan(ctx, plan, rng, np_rng, size, enc, dpi)
        img = scan.image
        detail["skew_deg"] = round(scan.angle, 3)
    W, H = media
    if variant == "strips":
        n = rng.randint(2, 4)
        bounds = [round(img.height * i / n) for i in range(n + 1)]
        xobjs, ops, metas = {}, [], []
        for i in range(n):
            strip = img.crop((0, bounds[i], img.width, bounds[i + 1]))
            obj, meta = image_xobj(ctx, rng, enc, strip)
            metas.append(meta)
            xobjs[f"Im{i}"] = obj
            y0 = H - bounds[i + 1] * 72 / dpi
            hh = (bounds[i + 1] - bounds[i]) * 72 / dpi
            ops += ["q", f"{fmt(W)} 0 0 {fmt(hh)} 0 {fmt(y0)} cm", f"/Im{i} Do", "Q"]
        detail["images"] = metas
    else:
        obj, meta = image_xobj(ctx, rng, enc, img)
        detail["images"] = [meta]
        xobjs = {"Im0": obj}
        ops = ["q", f"{fmt(W)} 0 0 {fmt(H)} 0 0 cm", "/Im0 Do", "Q"]
    entries = {"Contents": contents_entry(ctx.f, ops, compress(ctx, rng), rng),
               "Resources": resources(xobjs=xobjs, procset=rng.random() < 0.5)}
    if rng.random() < 0.08:
        entries["Rotate"] = str(rng.choice([90, 180, 270]))
    detail.update({"expected_text": None, "content_script": content_script(plan, rng) if variant == "text" else "none",
                   "script_basis": "none"})
    return PageBuild(entries, label("raster", "none", False), detail, media)


# ---- invisible OCR text layer under a full-page image -------------------------------------------

def build_ocr(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    enc, dpi = pick_scan_format(rng, ctx.style, *size, force=plan.enc)
    # OCR engines deskew before recognising, so OCR'd scans are nearly straight; large skew also makes
    # poppler split words in the hidden text, which says nothing about the page label.
    scan, media = make_scan(ctx, plan, rng, np_rng, size, enc, dpi, max_skew=0.4)
    W, H = media
    s = 72.0 / dpi
    script, legacy = plan.script, plan.legacy
    pool = ctx.pool
    if legacy:
        fam_font, fam = _legacy_font(ctx, rng)
        font, per_word = fam_font, True
    elif ctx.style.ocr_style == "glyphless" or script != "latin":
        if ctx.style.ocr_style == "glyphless":
            font, per_word = pool.glyphless_font(), False
        else:
            font, per_word = pool.cid(script), True
    else:
        font, per_word = pool.latin(rng.choice(["arial", "tnr", "calibri"])), True
    variant = plan.variant or rng.choices(["after", "before", "occluded"], [66, 22, 12])[0]
    tr = 0 if variant == "occluded" else 3
    obj, meta = image_xobj(ctx, rng, enc, scan.image)
    fres = "F1"
    ops: list[str] = ["q", f"{fmt(W)} 0 0 {fmt(H)} 0 0 cm", "/Im0 Do", "Q"]
    text_ops: list[str] = ["q"]
    expected_lines = []
    for line in scan.lines:
        if not line:
            continue
        # a legacy-encoded word is about 1.2 characters per glyph cell of the image word
        shown = [(st.legacy_word_of_length(rng, max(1, round(1.2 * wd.w / (0.55 * wd.size)))) if legacy else wd.text)
                 for wd in line]
        expected_lines.append(" ".join(shown))
        if per_word:
            for wd, t in zip(line, shown):
                fs = wd.size * s
                nat = font.width(t, fs)
                tz = max(1.0, min(1000.0, 100.0 * (wd.w * s) / nat)) if nat > 0 else 100.0
                text_ops += ["BT", f"{tr} Tr", f"/{fres} {fmt(fs)} Tf", f"{fmt(tz)} Tz",
                             f"1 0 0 1 {fmt(wd.x * s)} {fmt(H - wd.y * s)} Tm",
                             f"{font.encode(t)} Tj", "ET"]
        else:
            t = " ".join(shown)
            first, last = line[0], line[-1]
            fs = first.size * s
            nat = font.width(t, fs)
            span = (last.x + last.w - first.x) * s
            tz = max(1.0, min(1000.0, 100.0 * span / nat)) if nat > 0 else 100.0
            text_ops += ["BT", f"{tr} Tr", f"/{fres} {fmt(fs)} Tf", f"{fmt(tz)} Tz",
                         f"1 0 0 1 {fmt(first.x * s)} {fmt(H - first.y * s)} Tm",
                         f"{font.encode(t)} Tj", "ET"]
    text_ops.append("Q")
    use_c = compress(ctx, rng)
    if variant == "after":
        order = [ops, text_ops]
    else:
        order = [text_ops, ops]
    if rng.random() < 0.5:
        objs = [content_obj(ctx.f, part, use_c) for part in order]
        contents = "[" + " ".join(f"{o} 0 R" for o in objs) + "]"
    else:
        contents = f"{content_obj(ctx.f, order[0] + order[1], use_c)} 0 R"
    entries = {"Contents": contents,
               "Resources": resources({fres: font.obj}, {"Im0": obj}, procset=rng.random() < 0.5)}
    detail = {"expected_text": "\n".join(expected_lines), "variant": variant, "dpi": dpi, "images": [meta],
              "skew_deg": round(scan.angle, 3), "content_script": script, "columns": 1,
              "ocr_font": "glyphless" if isinstance(font, pp.GlyphlessFont) else "named", "per_word": per_word,
              "script_basis": "font_name" if legacy else "text_layer"}
    if legacy:
        detail["legacy_font_family"] = fam
    return PageBuild(entries, label("ocr", script, legacy), detail, media)


# ---- mixed: native text with a partial-page image -----------------------------------------------

def build_mixed(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    w, h = size
    geo = make_geo(rng, w, h)
    script = plan.script
    rtl = script == "arabic"
    variant = plan.variant or rng.choice(["logo_header", "side_image", "watermark", "figures"])
    fs = rng.choice([10, 11, 12])
    if script == "latin":
        key = rng.choice(ctx.style.latin_fonts)
        body, bold, avg, wf = ctx.pool.latin(key), ctx.pool.latin(key, True), LATIN_FONTS[key][5], None
    else:
        body = bold = ctx.pool.cid(script)
        avg, wf = 0.62, body.width
    # image rectangle in PDF points (x0, y0, x1, y1)
    iw = w * rng.uniform(0.16, 0.3)
    ih = iw * rng.uniform(0.7, 1.3)
    if variant == "logo_header":
        x0 = geo.ml if rng.random() < 0.5 else w - geo.mr - iw
        rect = (x0, h - geo.mt - ih, x0 + iw, h - geo.mt)
        geo.mt += ih + 14
    elif variant == "side_image":
        iw = w * rng.uniform(0.25, 0.35)
        ih = h * rng.uniform(0.25, 0.4)
        rect = (geo.ml, h - geo.mt - ih, geo.ml + iw, h - geo.mt)
        geo.ml += iw + 18
    elif variant == "watermark":
        iw, ih = w * rng.uniform(0.55, 0.85), h * rng.uniform(0.3, 0.5)
        x0, y0 = (w - iw) / 2, h * rng.uniform(0.2, 0.45)
        rect = (x0, y0, x0 + iw, y0 + ih)
    else:  # figures: image in the middle, lines that would overlap it are dropped
        iw, ih = w * rng.uniform(0.3, 0.55), h * rng.uniform(0.12, 0.22)
        x0, y0 = geo.ml + rng.uniform(0, 0.3) * (w - geo.ml - geo.mr - iw), h * rng.uniform(0.3, 0.55)
        rect = (x0, y0, x0 + iw, y0 + ih)
    lines, _ = flow_text(rng, script, False, geo, fs, avg, 1, rng.uniform(0.6, 1.0), variant != "side_image",
                         rng.random() < 0.4, rtl=rtl, width_fn=wf)
    if variant == "figures":
        lines = [ln for ln in lines if not (rect[1] - ln.size * 1.5 < ln.y < rect[3] + ln.size)]
    if len([ln for ln in lines if ln.size <= fs + 0.1]) < 4:
        variant = "watermark"
        geo2 = make_geo(rng, w, h)
        lines, _ = flow_text(rng, script, False, geo2, fs, avg, 1, 0.8, True, False, rtl=rtl, width_fn=wf)
        iw, ih = w * 0.6, h * 0.35
        rect = ((w - iw) / 2, h * 0.3, (w + iw) / 2, h * 0.3 + ih)
    dpi = rng.choice([72, 100, 120, 150])
    ipx_w, ipx_h = max(16, round((rect[2] - rect[0]) * dpi / 72)), max(16, round((rect[3] - rect[1]) * dpi / 72))
    img = scanimg.stamp_image(rng, np_rng, ipx_w, ipx_h, rng.random() < 0.5)
    if variant == "watermark":
        arr = np.asarray(img, dtype=np.float32)
        img = Image.fromarray((255 - (255 - arr) * 0.22).clip(0, 255).astype(np.uint8), img.mode)
    if img.mode == "RGB":
        entries_i, data, meta = scanimg.enc_dct(img, rng.choice([40, 60, 80]))
    elif rng.random() < 0.5:
        entries_i, data, meta = scanimg.enc_dct(img, rng.choice([40, 60, 80]))
    else:
        entries_i, data, meta = scanimg.enc_flate(img, 6, False)
    meta = dict(meta, width=img.width, height=img.height)
    obj = ctx.f.add_stream(entries_i, data)
    res = {False: ("F1", body), True: ("F2", bold)}
    text_ops = emit_lines(lines, res, rng, per_line_bt=rng.random() < 0.6, tj_p=0.0 if rtl else 0.2)
    img_ops = ["q", f"{fmt(rect[2] - rect[0])} 0 0 {fmt(rect[3] - rect[1])} {fmt(rect[0])} {fmt(rect[1])} cm",
               "/Im0 Do", "Q"]
    ops = (img_ops + text_ops) if (variant == "watermark" or rng.random() < 0.7) else (text_ops + img_ops)
    contents = contents_entry(ctx.f, ops, compress(ctx, rng), rng)
    entries = {"Contents": contents,
               "Resources": resources({"F1": body.obj, "F2": bold.obj}, {"Im0": obj}, procset=rng.random() < 0.5)}
    detail = {"expected_text": expected_from(lines), "columns": 1, "rtl": rtl, "variant": variant,
              "images": [meta], "image_rect_pt": [round(v, 1) for v in rect], "script_basis": "text_layer"}
    return PageBuild(entries, label("mixed", script, False), detail)


# ---- vector only --------------------------------------------------------------------------------

def _rects_from_mask(mask: np.ndarray) -> list[tuple[int, int, int, int]]:
    """Merge a boolean bitmap into rectangles: horizontal runs, merged vertically when identical."""
    h, w = mask.shape
    out: list[tuple[int, int, int, int]] = []
    open_: dict[tuple[int, int], int] = {}
    for y in range(h + 1):
        runs: list[tuple[int, int]] = []
        if y < h:
            row = np.concatenate([[0], mask[y].astype(np.int8), [0]])
            d = np.diff(row)
            starts, ends = np.nonzero(d == 1)[0], np.nonzero(d == -1)[0]
            runs = list(zip(starts.tolist(), ends.tolist()))
        now = set(runs)
        for key in sorted(open_):
            if key not in now:
                y0 = open_.pop(key)
                out.append((key[0], y0, key[1] - key[0], y - y0))
        for key in runs:
            open_.setdefault(key, y)
    return out


def build_vector(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    w, h = size
    geo = make_geo(rng, w, h)
    variant = plan.variant or rng.choices(["grid", "chart", "plan", "outlined"], [3, 2, 3, 3])[0]
    ops: list[str] = []
    detail: dict = {"variant": variant}
    if variant == "grid":
        cols, rows = rng.randint(2, 6), rng.randint(5, 25)
        x0, x1, y1, y0 = geo.ml, w - geo.mr, h - geo.mt, geo.mb + rng.uniform(0, 0.4) * (h - geo.mt - geo.mb)
        ops += ["q", f"{fmt(rng.choice([0.4, 0.75, 1.0]))} w", "0 G", f"{fmt(0.88)} g",
                f"{fmt(x0)} {fmt(y1 - 26)} {fmt(x1 - x0)} 26 re f", "0 g"]
        for r in range(rows + 1):
            y = y1 - (y1 - y0) * r / rows
            ops.append(f"{fmt(x0)} {fmt(y)} m {fmt(x1)} {fmt(y)} l S")
        xs = sorted(rng.uniform(x0, x1) for _ in range(cols - 1))
        for x in [x0] + xs + [x1]:
            ops.append(f"{fmt(x)} {fmt(y0)} m {fmt(x)} {fmt(y1)} l S")
        for _ in range(rng.randint(0, 12)):
            ops.append(f"{fmt(rng.uniform(x0, x1 - 12))} {fmt(rng.uniform(y0, y1 - 14))} 8 8 re S")
        ops.append("Q")
    elif variant == "chart":
        x0, x1, y1, y0 = geo.ml, w - geo.mr, h - geo.mt, h * rng.uniform(0.35, 0.6)
        ops += ["q", "1 w 0 G", f"{fmt(x0)} {fmt(y1)} m {fmt(x0)} {fmt(y0)} l {fmt(x1)} {fmt(y0)} l S"]
        n = rng.randint(4, 12)
        bw = (x1 - x0) / (n * 1.5)
        for i in range(n):
            bh = rng.uniform(0.1, 0.95) * (y1 - y0)
            ops.append(f"{fmt(rng.random())} {fmt(rng.random())} {fmt(rng.random())} rg "
                       f"{fmt(x0 + 10 + i * bw * 1.5)} {fmt(y0 + 1)} {fmt(bw)} {fmt(bh)} re f")
        pts = [(x0 + (x1 - x0) * i / 10, y0 + rng.uniform(0.1, 0.9) * (y1 - y0)) for i in range(11)]
        ops.append("0 0.3 0.8 RG 1.5 w " + f"{fmt(pts[0][0])} {fmt(pts[0][1])} m " +
                   " ".join(f"{fmt(px)} {fmt(py)} l" for px, py in pts[1:]) + " S")
        ops.append("Q")
    elif variant == "plan":
        ops += ["q", "0.8 w 0 G", "[6 3] 0 d"]
        for _ in range(rng.randint(3, 7)):
            cx, cy = rng.uniform(geo.ml, w - geo.mr), rng.uniform(geo.mb, h - geo.mt)
            pts = [(cx + rng.uniform(-120, 120), cy + rng.uniform(-90, 90)) for _ in range(rng.randint(3, 7))]
            ops.append(f"{fmt(rng.uniform(0.6, 1))} g " + f"{fmt(pts[0][0])} {fmt(pts[0][1])} m " +
                       " ".join(f"{fmt(px)} {fmt(py)} l" for px, py in pts[1:]) + " h B")
        ops.append("[] 0 d 1.2 w")
        for _ in range(rng.randint(2, 5)):
            a = [rng.uniform(geo.ml, w - geo.mr) if i % 2 == 0 else rng.uniform(geo.mb, h - geo.mt) for i in range(8)]
            ops.append(f"{fmt(a[0])} {fmt(a[1])} m {fmt(a[2])} {fmt(a[3])} {fmt(a[4])} {fmt(a[5])} {fmt(a[6])} {fmt(a[7])} c S")
        ops += ["Q", "q", f"{fmt(geo.ml)} {fmt(geo.mb)} {fmt(w - geo.ml - geo.mr)} {fmt(h - geo.mb - geo.mt)} re W n",
                "0.3 w 0.5 G"]
        for i in range(rng.randint(10, 40)):
            ops.append(f"{fmt(geo.ml + i * 14)} {fmt(geo.mb)} m {fmt(geo.ml + i * 14 + 200)} {fmt(h - geo.mt)} l S")
        ops.append("Q")
    else:  # text outlined into path shapes
        script = plan.script if plan.script in ("latin",) or plan.script in st.BLOCK else "latin"
        w_px, h_px = max(100, round(w)), max(100, round(h))
        rng2 = random.Random(rng.random())
        paras = [st.paragraph(rng2, script) for _ in range(rng.randint(2, 5))]
        scan = scanimg.render_scan(rng2, np_rng, w_px, h_px, 72, script, paras, rng.choice([11, 12, 14]),
                                   (0.12, 0.1, 0.12, 0.1), 0.0, 0.0, 0.0, 0.0, 0.0, ink_level=0,
                                   title=st.title(rng2, script))
        mask = np.asarray(scan.image) < 128
        rects = _rects_from_mask(mask)
        ops += ["q", "0 g"]
        for i in range(0, len(rects), 150):
            ops.append(" ".join(f"{x} {fmt(h_px - y - hh)} {ww} {hh} re" for x, y, ww, hh in rects[i:i + 150]) + " f")
        ops.append("Q")
        detail["outlined_script"] = script
        detail["rects"] = len(rects)
    entries = {"Contents": contents_entry(ctx.f, ops, compress(ctx, rng), rng),
               "Resources": resources(procset=rng.random() < 0.5)}
    if rng.random() < 0.1:
        entries["Rotate"] = str(rng.choice([90, 270]))
    detail.update({"expected_text": None, "script_basis": "none"})
    return PageBuild(entries, label("vector", "none", False), detail)


# ---- empty --------------------------------------------------------------------------------------

def build_empty(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    variant = plan.variant or rng.choice(["no_contents", "empty_stream", "whitespace", "q_Q", "bt_et",
                                          "empty_array", "state_only"])
    f = ctx.f
    entries: dict[str, str] = {"Resources": "<< >>"}
    if variant == "no_contents":
        pass
    elif variant == "empty_stream":
        entries["Contents"] = f"{f.add_stream('', b'')} 0 R"
    elif variant == "whitespace":
        ws = b" \n \n"
        entries["Contents"] = f"{f.add_stream('', ws)} 0 R"
    elif variant == "q_Q":
        qq = b"q\nQ\n"
        entries["Contents"] = f"{f.add_stream('', qq)} 0 R"
    elif variant == "bt_et":
        font = ctx.pool.latin(rng.choice(ctx.style.latin_fonts))
        entries["Contents"] = f"{f.add_stream('', b'BT\n/F1 12 Tf\nET\n')} 0 R"
        entries["Resources"] = resources({"F1": font.obj})
    elif variant == "empty_array":
        entries["Contents"] = "[]"
    else:
        entries["Contents"] = f"{f.add_stream('', b'q 1 0 0 1 0 0 cm 0 g 0 G 1 w Q')} 0 R"
    if rng.random() < 0.05:
        entries["Rotate"] = str(rng.choice([90, 180]))
    return PageBuild(entries, label("empty", "none", False),
                     {"variant": variant, "expected_text": None, "script_basis": "none"})


BUILDERS = {"native": build_native, "raster": build_raster, "ocr": build_ocr, "mixed": build_mixed,
            "vector": build_vector, "empty": build_empty}


def build_page(ctx: Ctx, plan: PagePlan, rng: random.Random, np_rng, size) -> PageBuild:
    return BUILDERS[plan.kind](ctx, plan, rng, np_rng, size)
