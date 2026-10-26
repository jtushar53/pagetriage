"""Synthetic rubber stamps, seals and look-alikes on colour text pages, for module M6.

Nothing here is a real stamp. A stamp is drawn as a coloured overlay (ring or border, lines of text,
a date), rotated, with ink dropout, and multiplied onto a page so that the text underneath shows
through, as stamp ink does. Output depends only on the `random.Random` it is given.
"""
from __future__ import annotations

import random

import numpy as np
from PIL import Image, ImageDraw

import scanimg

COLOURS = {"blue": (25, 55, 170), "violet": (110, 40, 150), "red": (185, 30, 40), "green": (20, 110, 60)}
WORDS = ["VERIFIED", "REGISTRAR", "RECEIVED", "SUB-REGISTRAR", "COPY", "CERTIFIED", "OFFICE", "PAID"]
DATES = ["12 MAR 2019", "04-11-2021", "27/08/2016", "30 JUN 2020"]


def _fit_font(size):
    return scanimg.font(max(8, size))


def stamp_layer(rng: random.Random, kind: str, diameter: int, colour, alpha: float):
    """An RGBA layer, square, holding one stamp of the kind ('circle', 'seal', 'rectangle')."""
    s = diameter
    layer = Image.new("RGBA", (s + 8, s + 8), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    a = round(255 * alpha)
    ink = (*colour, a)
    w = max(2, round(s * rng.uniform(0.02, 0.035)))
    m = 4
    text = rng.choice(WORDS)
    f = _fit_font(round(s * 0.13))
    if kind in ("circle", "seal"):
        d.ellipse([m, m, m + s, m + s], outline=ink, width=w)
        if kind == "seal":
            inset = round(s * 0.16)
            d.ellipse([m + inset, m + inset, m + s - inset, m + s - inset], outline=ink, width=max(2, w - 1))
        d.text((m + s / 2, m + s * 0.42), text, font=f, fill=ink, anchor="mm")
        d.text((m + s / 2, m + s * 0.58), rng.choice(DATES), font=_fit_font(round(s * 0.1)), fill=ink, anchor="mm")
    else:
        h = round(s * 0.6)
        d.rectangle([m, m, m + s, m + h], outline=ink, width=w)
        d.text((m + s / 2, m + h * 0.33), text, font=f, fill=ink, anchor="mm")
        d.text((m + s / 2, m + h * 0.68), rng.choice(DATES), font=_fit_font(round(s * 0.1)), fill=ink, anchor="mm")
    arr = np.asarray(layer).copy()
    # Ink dropout: the rubber does not print everywhere.
    drop = np.random.default_rng(rng.randrange(1 << 30)).random(arr.shape[:2]) < rng.uniform(0.05, 0.2)
    arr[drop, 3] = (arr[drop, 3] * 0.3).astype(np.uint8)
    return Image.fromarray(arr, "RGBA")


def multiply(page_rgb: Image.Image, layer: Image.Image, x: int, y: int, tilt: float):
    """Multiply `layer` (rotated by `tilt` degrees) onto the page at its centre (x, y); returns the
    bounding box of the painted ink in page coordinates."""
    rot = layer.rotate(tilt, resample=Image.BICUBIC, expand=True)
    arr = np.asarray(rot).astype(np.float32)
    h, w = arr.shape[:2]
    x0, y0 = x - w // 2, y - h // 2
    page = np.asarray(page_rgb).astype(np.float32)
    H, W = page.shape[:2]
    cx0, cy0, cx1, cy1 = max(0, x0), max(0, y0), min(W, x0 + w), min(H, y0 + h)
    sub = arr[cy0 - y0:cy1 - y0, cx0 - x0:cx1 - x0]
    alpha = sub[..., 3:4] / 255.0
    target = page[cy0:cy1, cx0:cx1]
    page[cy0:cy1, cx0:cx1] = target * (1 - alpha + alpha * sub[..., :3] / 255.0)
    ys, xs = np.nonzero(sub[..., 3] > 40)
    box = (int(xs.min()) + cx0, int(ys.min()) + cy0, int(xs.max()) + cx0, int(ys.max()) + cy0) if len(xs) else None
    return Image.fromarray(np.clip(page, 0, 255).astype(np.uint8), "RGB"), box


def text_page(rng: random.Random, np_rng, script="latin", tint=(255, 252, 244)):
    import synthtext
    paras = [synthtext.paragraph(rng, script, 5) for _ in range(6)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, script, paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, 2.0, 0.0, 0.0, 0.0)
    g = np.asarray(scan.image).astype(np.float32) / 255.0
    rgb = (g[..., None] * np.array(tint, dtype=np.float32)).astype(np.uint8)
    return Image.fromarray(rgb, "RGB")
