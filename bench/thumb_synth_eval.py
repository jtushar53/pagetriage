#!/usr/bin/env python3
"""Check the thumb-impression detector (module M8) against synthetic prints and look-alikes.

    python -I bench/thumb_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page with one fingerprint-like impression (0.7 to 1.1 in wide,
0.85 to 1.3 in tall, ridge period 3 to 5 px, bench/synth/printgen.py) in the lower half. Look-alikes
that must not be found: a block of text, hatching (parallel lines), a barcode, a halftone dot grid,
a solid ink blob, diagonal hatching, and a plain page. Conditions: clean, blur 0.8 px, faint (pressure
0.45), and smudged (ridges blurred by 1.8 px, which fuses them into a blob). A print is found when a
finding overlaps the drawn box with IoU >= 0.5. Seeds 1-6 are the dev split (thresholds were set looking
at them), seeds 11-16 the test split.

The prints are drawn by the author of the detector from the properties it measures. Real inked
impressions smear, are partial, and are often scanned at 150 dpi where a ridge is 3 pixels. Needs the
Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageDraw, ImageFilter

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import printgen  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["thumb"], "debug": True})
CONDITIONS = {"clean": {}, "blur 0.8px": dict(blur=0.8), "faint (0.45)": dict(pressure=0.45),
              "smudged (1.8px)": dict(smudge=1.8)}
LOOKALIKES = ["text block", "hatching", "barcode", "halftone", "solid blob", "diagonal hatching", "plain page"]


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def base(seed):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(4)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.45),
                               0.0, 2.0, 0.0, 0.0, 0.0)
    return scan.image.copy(), rng, np_rng


def page(kind, seed, blur=0.0, pressure=0.95, smudge=0.0):
    img, rng, np_rng = base(seed)
    d = ImageDraw.Draw(img)
    f = scanimg.font(round(11 * 150 / 72))
    cx, cy = rng.randint(400, 800), rng.randint(1250, 1500)
    truth = None
    if kind == "print":
        w, h = rng.randint(105, 165), rng.randint(130, 195)
        alpha = printgen.fingerprint_alpha(np_rng, rng, w, h, rng.uniform(3.0, 5.0), pressure, smudge)
        img, truth = printgen.paste_alpha(img, alpha, cx, cy)
    elif kind == "text block":
        for i in range(8):
            d.text((cx - 80, cy - 90 + i * 24), synthtext.sentence(rng, "latin")[:14], font=f, fill=40, anchor="ls")
    elif kind == "hatching":
        for k in range(30):
            d.line([(cx - 70, cy - 90 + k * 6), (cx + 70, cy - 90 + k * 6)], fill=50, width=3)
    elif kind == "barcode":
        x = cx - 90
        while x < cx + 90:
            wbar = rng.choice([3, 4, 6])
            d.rectangle([x, cy - 55, x + wbar - 1, cy + 55], fill=40)
            x += wbar + rng.choice([3, 4, 6])
    elif kind == "halftone":
        for yy in range(cy - 80, cy + 80, 5):
            for xx in range(cx - 70, cx + 70, 5):
                d.ellipse([xx, yy, xx + 3, yy + 3], fill=60)
    elif kind == "solid blob":
        d.ellipse([cx - 60, cy - 75, cx + 60, cy + 75], fill=60)
    elif kind == "diagonal hatching":
        for k in range(-30, 30):
            d.line([(cx - 70 + k * 6, cy - 90), (cx + 70 + k * 6, cy + 90)], fill=50, width=3)
    out = img.filter(ImageFilter.GaussianBlur(blur)) if blur else img
    return out, truth


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "thumb_impression"]


def main():
    print(f"{'pages':<19}{'cond':<17}{'split':<6}{'found':>8}{'spurious/page':>15}")
    for kind in ["print"] + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind != "print" and cond != "clean":
                continue
            for split, seeds in (("dev", DEV), ("test", TEST)):
                hits = total = spurious = 0
                for seed in seeds:
                    img, truth = page(kind, seed, **kw)
                    fs = found(img)
                    ok = 0
                    if truth:
                        total += 1
                        if any(iou(f["bbox"], truth) >= 0.5 for f in fs):
                            hits += 1
                            ok = 1
                    spurious += len(fs) - ok
                print(f"{kind:<19}{cond:<17}{split:<6}{(f'{hits}/{total}' if total else '-'):>8}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
