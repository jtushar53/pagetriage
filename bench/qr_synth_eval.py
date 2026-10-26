#!/usr/bin/env python3
"""Check the QR finder-pattern detector (module M11) against synthetic QR-like symbols.

    python -I bench/qr_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page (bench/synth) with one symbol of 25 to 57 modules and 3 to 5 px
per module pasted in a margin. The symbols are not decodable (bench/synth/qrgen.py): finder, separator
and timing patterns are drawn, the data area is random. Look-alikes that must not be found: a plain
page, a checkbox grid, a table, a 1-D barcode and a random-module block with no finder patterns.
Conditions: clean, rotated 3 degrees, noise 8, blur 1 px, and a symbol of 2 px per module (the
detector's minimum). A symbol is found when a finding overlaps it with IoU >= 0.5. Seeds 1-6 dev,
11-16 test. Needs the Python binding, numpy and Pillow.

These are clean, flat symbols. A QR printed on a stamp paper and photocopied can be smeared, have
fused modules, and sit on a hatched background, none of which is simulated. Nothing is decoded, so an
e-stamp QR cannot be told from any other QR.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402
import qrgen  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["qr"], "debug": True})
CONDITIONS = {"clean": {}, "rotate 3deg": dict(rot=3.0), "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0),
              "2 px/module": dict(scale=2)}
LOOKALIKES = ["plain page", "checkbox grid", "table", "barcode", "random modules"]


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def page(kind, seed, rot=0.0, noise=2.0, blur=0.0, scale=None):
    rng = random.Random(seed * 17 + len(kind))
    np_rng = np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(5)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, 2.0, 0.0, 0.0, 0.0)
    img = scan.image.copy()
    d = ImageDraw.Draw(img)
    truth = []
    if kind == "qr":
        sc = scale or rng.choice([3, 4, 5])
        sym = qrgen.qr_like(rng, rng.choice([25, 33, 41, 57]) if sc >= 3 else 25, sc)
        if rot:
            sym = sym.rotate(rot, expand=True, fillcolor=255, resample=2)
        x, y = rng.randint(100, 1240 - sym.width - 100), rng.randint(min(1300, 1754 - sym.height - 20), 1754 - sym.height - 20)
        img.paste(sym, (x, y))
        # Truth is the symbol without its quiet zone (4 modules).
        q = 4 * (scale or sc)
        truth.append((x + q, y + q, x + sym.width - q, y + sym.height - q))
    elif kind == "checkbox grid":
        for r in range(6):
            for c in range(8):
                d.rectangle([150 + c * 40, 1300 + r * 40, 150 + c * 40 + 24, 1300 + r * 40 + 24], outline=30, width=2)
    elif kind == "table":
        for r in range(7):
            d.line([(150, 1300 + r * 40), (1090, 1300 + r * 40)], fill=30, width=2)
        for c in range(6):
            d.line([(150 + c * 188, 1300), (150 + c * 188, 1540)], fill=30, width=2)
    elif kind == "barcode":
        x = 200
        while x < 700:
            w = rng.choice([2, 3, 4, 6])
            d.rectangle([x, 1350, x + w - 1, 1450], fill=0)
            x += w + rng.choice([2, 3, 4])
    elif kind == "random modules":
        sym = qrgen.qr_like(rng, 33, 4)
        # Blank the three finder areas by painting the corners white.
        s = 4 * 4
        for ox, oy in ((0, 0), (sym.width - 12 * 4 - s, 0), (0, sym.height - 12 * 4 - s)):
            ImageDraw.Draw(sym).rectangle([ox, oy, ox + 12 * 4 + s - 1, oy + 12 * 4 + s - 1], fill=255)
        img.paste(sym, (200, 1300))
    return handwrite.finish(img, rng, noise=noise, blur=blur, skew=0.0), truth


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "qr_code"]


def main():
    print(f"{'pages':<18}{'cond':<14}{'split':<6}{'found':>8}{'spurious/page':>15}")
    for kind in ["qr"] + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind != "qr" and cond != "clean":
                continue
            for split, seeds in (("dev", DEV), ("test", TEST)):
                hits = total = spurious = 0
                for seed in seeds:
                    img, truth = page(kind, seed, **kw)
                    fs = found(img)
                    matched = set()
                    for t in truth:
                        total += 1
                        for i, f in enumerate(fs):
                            if iou(f["bbox"], t) >= 0.5:
                                hits += 1
                                matched.add(i)
                                break
                    spurious += len(fs) - len(matched)
                print(f"{kind:<18}{cond:<14}{split:<6}{(f'{hits}/{total}' if total else '-'):>8}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
