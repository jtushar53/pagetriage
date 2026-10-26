"""Synthetic page images for the PDF corpus: text lines rendered with Pillow, then degraded.

Latin text uses Pillow's bundled default font. Other scripts are drawn as procedural pseudo-glyphs
(random strokes in character cells, with a headline for Devanagari, Bengali and Gurmukhi), so the
output never depends on fonts installed on the host. The images contain no real document content.

Encoders produce the byte streams a PDF image XObject needs: DCTDecode (JPEG), FlateDecode (with
and without a PNG predictor), 1-bit Flate and CCITT Group 4.
"""
from __future__ import annotations

import io
import math
import random
import unicodedata
import zlib
from dataclasses import dataclass

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

import synthtext as scr


@dataclass
class Word:
    text: str
    x: float       # left end of the baseline, pixels, origin top-left, after skew
    y: float       # baseline, pixels
    w: float       # advance width, pixels
    size: float    # font size, pixels


@dataclass
class Scan:
    image: Image.Image          # mode "L"
    lines: list[list[Word]]
    angle: float                # degrees counter-clockwise, as applied to the page
    dpi: int


_FONT_CACHE: dict[int, ImageFont.FreeTypeFont] = {}


def font(size_px: int):
    size_px = max(6, int(size_px))
    if size_px not in _FONT_CACHE:
        _FONT_CACHE[size_px] = ImageFont.load_default(size=size_px)
    return _FONT_CACHE[size_px]


def _is_base(ch: str) -> bool:
    """Letters and digits take a character cell; combining marks do not."""
    return unicodedata.category(ch)[0] in "LN"


class _Measure:
    """Advance widths in pixels for Latin (real font) and pseudo-glyph scripts (fixed cells)."""

    def __init__(self, script: str, size: int):
        self.script, self.size = script, size
        self.f = font(size) if script == "latin" else None

    def width(self, text: str) -> float:
        if self.f is not None:
            return float(self.f.getlength(text))
        n_base = sum(1 for ch in text if _is_base(ch) and ch != " ")
        n_space = text.count(" ")
        return n_base * 0.55 * self.size + n_space * 0.32 * self.size + (len(text) - n_base - n_space) * 0.0


def _stroke(draw: ImageDraw.ImageDraw, rng: random.Random, x: float, yb: float, cw: float, em: float,
            width: int, ink: int) -> None:
    top = yb - 0.72 * em
    mid = yb - 0.36 * em
    for _ in range(rng.randint(2, 4)):
        kind = rng.randint(0, 4)
        a = x + rng.uniform(0.05, 0.4) * cw
        b = x + rng.uniform(0.45, 0.95) * cw
        if kind == 0:
            draw.line([(a, top + 0.1 * em), (a, yb)], fill=ink, width=width)
        elif kind == 1:
            draw.arc([a, top, b, yb], rng.randint(0, 180), rng.randint(190, 360), fill=ink, width=width)
        elif kind == 2:
            draw.line([(a, rng.uniform(top, mid)), (b, rng.uniform(mid, yb))], fill=ink, width=width)
        elif kind == 3:
            draw.ellipse([a, mid - 0.15 * em, b, mid + 0.2 * em], outline=ink, width=width)
        else:
            draw.line([(a, mid), (b, mid)], fill=ink, width=width)


def _draw_pseudo_cursive_word(draw, rng, w, x, yb, em, width, ink):
    """One word of joined pseudo-letters along a baseline, with dots; returns the end x. Letters
    come in runs of 2-4 joined by the baseline, runs are separated by a small gap, as in the Arabic
    script where some letters do not join to the next."""
    cw = 0.5 * em
    base = yb - 0.18 * em
    letters = [ch for ch in w if _is_base(ch)]
    i = 0
    while i < len(letters):
        run = min(len(letters) - i, rng.randint(2, 4))
        x1 = x + run * cw
        draw.line([(x, base), (x1, base)], fill=ink, width=width)
        for k in range(run):
            lx = x + k * cw + 0.5 * cw
            kind = rng.randint(0, 3)
            if kind == 0:  # tall stem
                draw.line([(lx, base), (lx, yb - 0.72 * em)], fill=ink, width=width)
            elif kind == 1:  # bump above the baseline
                draw.arc([lx - 0.45 * cw, base - 0.5 * em, lx + 0.45 * cw, base + 0.05 * em], 180, 360, fill=ink, width=width)
            elif kind == 2:  # bowl below the baseline
                draw.arc([lx - 0.45 * cw, base - 0.3 * em, lx + 0.45 * cw, base + 0.3 * em], 0, 180, fill=ink, width=width)
            else:  # three short teeth
                for t in (-0.3, 0.0, 0.3):
                    draw.line([(lx + t * cw, base), (lx + t * cw, base - 0.3 * em)], fill=ink, width=width)
            if rng.random() < 0.6:
                r = max(1.0, 0.06 * em)
                dy = yb - 0.5 * em if rng.random() < 0.5 else base + 0.22 * em
                for d in range(rng.randint(1, 3)):
                    cx = lx + (d - 0.5) * 3 * r
                    draw.ellipse([cx - r, dy - r, cx + r, dy + r], fill=ink)
        x = x1 + 0.12 * em
        i += run
    return x


def _draw_pseudo_line(draw, rng, script, text, x, yb, em, ink):
    """Draw one line of pseudo-glyphs; return list of (word, x_start, width)."""
    width = max(1, round(em * 0.07))
    if script == "arabic":
        out, cx = [], x
        for w in text.split(" "):
            wx = cx
            cx = _draw_pseudo_cursive_word(draw, rng, w, cx, yb, em, width, ink)
            out.append((w, wx, cx - wx))
            cx += 0.32 * em
        return out
    cw = 0.55 * em
    cx = x
    out = []
    for w in text.split(" "):
        wx = cx
        bases = 0
        for ch in w:
            if _is_base(ch):
                _stroke(draw, rng, cx, yb, cw, em, width, ink)
                cx += cw
                bases += 1
            else:  # combining mark: a short stroke above the previous cell
                mx = cx - cw * 0.6
                draw.line([(mx, yb - 0.95 * em), (mx + 0.25 * cw, yb - 0.8 * em)], fill=ink, width=width)
        if script in scr.HEADLINE and bases:
            hy = yb - 0.72 * em
            draw.line([(wx, hy), (cx, hy)], fill=ink, width=width)
        out.append((w, wx, cx - wx))
        cx += 0.32 * em
    return out


def render_scan(rng: random.Random, np_rng: np.random.Generator, w_px: int, h_px: int, dpi: int,
                script: str, paragraphs: list[str], pt: float, margin_frac: tuple[float, float, float, float],
                skew_deg: float, noise: float, speckle: float, blur: float, shade: float,
                ink_level: int = 40, title: str | None = None) -> Scan:
    """Render paragraphs (list of strings) as justified-left lines, then skew, blur, shade, add noise."""
    img = Image.new("L", (w_px, h_px), 255)
    draw = ImageDraw.Draw(img)
    em = max(7, round(pt * dpi / 72))
    meas = _Measure(script, em)
    ml, mt, mr, mb = (int(f * (w_px if i % 2 == 0 else h_px)) for i, f in enumerate(margin_frac))
    avail = w_px - ml - mr
    lead = 1.38 * em
    y = mt + em
    lines: list[list[Word]] = []

    def put_line(text: str, size_em: int, ybase: float, indent: float = 0.0):
        words: list[Word] = []
        x0 = ml + indent
        if script == "latin":
            f = font(size_em)
            draw.text((x0, ybase), text, font=f, fill=ink_level, anchor="ls")
            pos = 0
            for wtxt in text.split(" "):
                if wtxt:
                    xs = x0 + f.getlength(text[:pos])
                    words.append(Word(wtxt, xs, ybase, float(f.getlength(wtxt)), size_em))
                pos += len(wtxt) + 1
        else:
            for wtxt, xs, ww in _draw_pseudo_line(draw, rng, script, text, x0, ybase, size_em, ink_level):
                if wtxt:
                    words.append(Word(wtxt, xs, ybase, ww, size_em))
        lines.append(words)

    if title:
        big = round(em * 1.5)
        tm = _Measure(script, big)
        put_line(title, big, y, max(0.0, (avail - tm.width(title)) / 2))
        y += 2.0 * lead
    for para in paragraphs:
        cur = ""
        for w in para.split(" "):
            cand = (cur + " " + w) if cur else w
            if cur and meas.width(cand) > avail:
                done, cur = scr.break_line(cur, w)
                put_line(done, em, y)
                y += lead
                if y > h_px - mb:
                    break
            else:
                cur = cand
        if cur and y <= h_px - mb:
            put_line(cur, em, y)
            y += lead
        y += 0.8 * lead
        if y > h_px - mb:
            break

    # --- degradation -------------------------------------------------------------------------
    theta = skew_deg
    if abs(theta) > 1e-6:
        img = img.rotate(theta, resample=Image.BICUBIC, fillcolor=255)
        cx, cy = w_px / 2, h_px / 2
        c, s = math.cos(math.radians(theta)), math.sin(math.radians(theta))
        for line in lines:
            for wd in line:
                dx, dy = wd.x - cx, wd.y - cy
                wd.x, wd.y = cx + dx * c + dy * s, cy - dx * s + dy * c
    if blur > 0:
        img = img.filter(ImageFilter.GaussianBlur(blur))
    arr = np.asarray(img, dtype=np.float32)
    if shade > 0:
        gx = np.linspace(-1, 1, w_px, dtype=np.float32)[None, :]
        gy = np.linspace(-1, 1, h_px, dtype=np.float32)[:, None]
        direction = rng.uniform(0, math.tau)
        ramp = gx * math.cos(direction) + gy * math.sin(direction)
        arr = arr * (1.0 - shade * (0.5 + 0.5 * ramp))
    if noise > 0:
        arr = arr + np_rng.normal(0.0, noise, size=arr.shape).astype(np.float32)
    if speckle > 0:
        n = int(speckle * w_px * h_px)
        if n:
            ys = np_rng.integers(0, h_px, size=n)
            xs = np_rng.integers(0, w_px, size=n)
            vals = np_rng.integers(0, 90, size=n).astype(np.float32)
            arr[ys, xs] = vals
            # a few larger blobs
            for _ in range(max(1, n // 400)):
                by, bx = int(np_rng.integers(0, h_px - 3)), int(np_rng.integers(0, w_px - 3))
                arr[by:by + 2, bx:bx + 2] = 30.0
    arr = np.clip(arr, 0, 255).astype(np.uint8)
    return Scan(Image.fromarray(arr, "L"), lines, theta, dpi)


def blank_scan(np_rng, rng, w_px, h_px, dpi, noise, speckle, shade) -> Scan:
    return render_scan(rng, np_rng, w_px, h_px, dpi, "latin", [], 10, (0.1, 0.1, 0.1, 0.1),
                       rng.uniform(-0.5, 0.5), noise, speckle, 0.4, shade)


def stamp_image(rng: random.Random, np_rng: np.random.Generator, w_px: int, h_px: int,
                color: bool) -> Image.Image:
    """A photo/seal-like picture: gradient, concentric rings, a rotated word, grain."""
    yy, xx = np.mgrid[0:h_px, 0:w_px].astype(np.float32)
    base = 150 + 60 * np.sin(xx / max(1, w_px) * rng.uniform(1, 4)) * np.cos(yy / max(1, h_px) * rng.uniform(1, 4))
    img = Image.fromarray(np.clip(base, 0, 255).astype(np.uint8), "L")
    d = ImageDraw.Draw(img)
    cx, cy = w_px // 2, h_px // 2
    r = min(w_px, h_px) // 2 - 3
    for k in range(rng.randint(2, 4)):
        d.ellipse([cx - r + 6 * k, cy - r + 6 * k, cx + r - 6 * k, cy + r - 6 * k], outline=30, width=2)
    f = font(max(8, r // 3))
    d.text((cx, cy), rng.choice(["SAMPLE", "SPECIMEN", "REGD", "COPY"]), font=f, fill=20, anchor="mm")
    arr = np.asarray(img, dtype=np.float32) + np_rng.normal(0, 8, size=(h_px, w_px)).astype(np.float32)
    arr = np.clip(arr, 0, 255).astype(np.uint8)
    out = Image.fromarray(arr, "L")
    if color:
        tint = np.array([rng.uniform(0.75, 1.0), rng.uniform(0.7, 1.0), rng.uniform(0.7, 1.0)], dtype=np.float32)
        rgb = np.clip(arr[..., None].astype(np.float32) * tint[None, None, :] * 1.1, 0, 255).astype(np.uint8)
        out = Image.fromarray(rgb, "RGB")
    return out


def tint_paper(img: Image.Image, rng: random.Random) -> Image.Image:
    """Turn a grey scan into a colour scan with warm or cool paper."""
    arr = np.asarray(img, dtype=np.float32)[..., None]
    tint = np.array([rng.uniform(0.93, 1.0), rng.uniform(0.9, 0.99), rng.uniform(0.78, 0.97)], dtype=np.float32)
    return Image.fromarray(np.clip(arr * tint[None, None, :], 0, 255).astype(np.uint8), "RGB")


# ---- encoders: return (stream dictionary entries, data bytes) -------------------------------------

def _img_dict(w: int, h: int, cs: str, bpc: int) -> str:
    return f"/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /{cs} /BitsPerComponent {bpc}"


def enc_dct(img: Image.Image, quality: int) -> tuple[str, bytes, dict]:
    buf = io.BytesIO()
    kw = {"subsampling": 2} if img.mode == "RGB" else {}
    img.save(buf, "JPEG", quality=quality, optimize=False, progressive=False, **kw)
    cs = "DeviceRGB" if img.mode == "RGB" else "DeviceGray"
    return (_img_dict(img.width, img.height, cs, 8) + " /Filter /DCTDecode", buf.getvalue(),
            {"filter": "DCTDecode", "quality": quality, "mode": img.mode})


def enc_flate(img: Image.Image, level: int, predictor: bool) -> tuple[str, bytes, dict]:
    cs = "DeviceRGB" if img.mode == "RGB" else "DeviceGray"
    ncomp = 3 if img.mode == "RGB" else 1
    raw = np.asarray(img, dtype=np.uint8).reshape(img.height, img.width * ncomp)
    extra = ""
    if predictor:
        # PNG "Up" filter on every row, as many producers write (Predictor 15)
        up = raw.copy()
        up[1:] = (raw[1:].astype(np.int16) - raw[:-1].astype(np.int16)).astype(np.uint8)
        rows = np.concatenate([np.full((img.height, 1), 2, dtype=np.uint8), up], axis=1)
        data = rows.tobytes()
        extra = f" /DecodeParms << /Predictor 15 /Colors {ncomp} /BitsPerComponent 8 /Columns {img.width} >>"
    else:
        data = raw.tobytes()
    comp = zlib.compress(data, level)
    return (_img_dict(img.width, img.height, cs, 8) + " /Filter /FlateDecode" + extra, comp,
            {"filter": "FlateDecode", "predictor": predictor, "mode": img.mode})


def to_bilevel(img: Image.Image, threshold: int) -> Image.Image:
    arr = np.asarray(img.convert("L"), dtype=np.uint8)
    return Image.fromarray(((arr > threshold) * 255).astype(np.uint8), "L").convert("1", dither=Image.Dither.NONE)


def enc_flate_1bit(img1: Image.Image, level: int) -> tuple[str, bytes, dict]:
    comp = zlib.compress(img1.tobytes(), level)
    return (_img_dict(img1.width, img1.height, "DeviceGray", 1) + " /Filter /FlateDecode", comp,
            {"filter": "FlateDecode", "mode": "1"})


def enc_ccitt(img1: Image.Image) -> tuple[str, bytes, dict]:
    buf = io.BytesIO()
    img1.save(buf, "TIFF", compression="group4")
    t = Image.open(io.BytesIO(buf.getvalue()))
    off, cnt = t.tag_v2[273], t.tag_v2[279]
    if isinstance(off, int):
        off, cnt = (off,), (cnt,)
    raw = buf.getvalue()
    data = b"".join(raw[o:o + c] for o, c in zip(off, cnt))
    parms = f"<< /K -1 /Columns {img1.width} /Rows {img1.height} /BlackIs1 true >>"
    return (_img_dict(img1.width, img1.height, "DeviceGray", 1) + f" /Filter /CCITTFaxDecode /DecodeParms {parms}",
            data, {"filter": "CCITTFaxDecode", "mode": "1"})
