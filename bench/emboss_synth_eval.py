#!/usr/bin/env python3
"""Check the embossed-seal detector (module M11) against synthetic blind embossings.

    python -I bench/emboss_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page (bench/synth) with one embossed seal of 30 to 50 mm (bench/synth/
embossgen.py: a rim, an inner ring and some arcs as a height field lit from a random side, drawn as
gray-level shading of +-A around the paper) in the lower half. A: amplitude of the shading, in gray
levels. Conditions: A 12, A 8, A 5, A 8 with noise 4, A 8 with blur 1 px. Look-alikes that must not be
found: a plain page, an inked ring stamp, a solid disc, a faint dark-ring stain, a dome (a round hill
of shading, not a ring) and a linear lighting gradient over the page. A seal is found when a finding
overlaps it with IoU >= 0.5. Seeds 1-6 dev, 11-16 test.

The relief is a rendering of the author's idea of embossing. Real blind embossings are subtler, are
often under the text, are creased or photocopied flat, and depend on the scanner's lighting. Needs the
Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import math
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import embossgen  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["emboss"], "debug": True})
CONDITIONS = {"A 12": dict(amp=12), "A 8": dict(amp=8), "A 5": dict(amp=5), "A 8 noise 4": dict(amp=8, noise=4.0),
              "A 8 blur 1px": dict(amp=8, blur=1.0)}
LOOKALIKES = ["plain page", "inked ring", "solid disc", "stain ring", "dome", "lighting gradient"]


DECOY = {}  # where the look-alike (or the seal) was drawn, for debugging only


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def page(kind, seed, amp=8, noise=1.0, blur=0.0):
    rng = random.Random(seed * 13 + len(kind))
    np_rng = np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(4)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, 0.0, 0.0, 0.0, 0.0)
    arr = np.asarray(scan.image, dtype=np.float32).copy()
    arr = np.minimum(arr, 252.0)
    dia = rng.randint(177, 295)
    x0, y0 = rng.randint(120, 1240 - dia - 200), rng.randint(900, 1754 - dia - 160)
    truth = []
    DECOY[(kind, seed)] = (x0, y0, x0 + dia, y0 + dia)
    if kind == "emboss":
        s = embossgen.shading(rng, dia, amp, light_deg=rng.uniform(0, 360))
        m = (s.shape[0] - dia) // 2
        arr[y0 - m:y0 - m + s.shape[0], x0 - m:x0 - m + s.shape[1]] += s * (arr[y0 - m:y0 - m + s.shape[0], x0 - m:x0 - m + s.shape[1]] > 200)
        truth.append((x0, y0, x0 + dia, y0 + dia))
    elif kind == "inked ring":
        img = Image.fromarray(arr.astype(np.uint8), "L")
        d = ImageDraw.Draw(img)
        d.ellipse([x0, y0, x0 + dia, y0 + dia], outline=60, width=4)
        d.ellipse([x0 + 14, y0 + 14, x0 + dia - 14, y0 + dia - 14], outline=60, width=2)
        arr = np.asarray(img, dtype=np.float32).copy()
    elif kind == "solid disc":
        img = Image.fromarray(arr.astype(np.uint8), "L")
        ImageDraw.Draw(img).ellipse([x0, y0, x0 + dia, y0 + dia], fill=40)
        arr = np.asarray(img, dtype=np.float32).copy()
    elif kind == "stain ring":
        yy, xx = np.mgrid[0:arr.shape[0], 0:arr.shape[1]].astype(np.float32)
        r = np.hypot(xx - (x0 + dia / 2), yy - (y0 + dia / 2)) / (dia / 2)
        arr -= 14.0 * np.exp(-0.5 * ((r - 0.95) / 0.04) ** 2) * (arr > 200)
    elif kind == "dome":
        yy, xx = np.mgrid[0:arr.shape[0], 0:arr.shape[1]].astype(np.float32)
        r = np.hypot(xx - (x0 + dia / 2), yy - (y0 + dia / 2)) / (dia / 2)
        arr -= 16.0 * np.exp(-0.5 * (r / 0.6) ** 2) * (arr > 200)
    elif kind == "lighting gradient":
        arr -= np.linspace(0, 25, arr.shape[1], dtype=np.float32)[None, :] * (arr > 200)
    if blur > 0:
        from PIL import ImageFilter
        arr = np.asarray(Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8), "L").filter(ImageFilter.GaussianBlur(blur)), dtype=np.float32)
    if noise > 0:
        arr = arr + np.random.default_rng(seed + 99).normal(0, noise, arr.shape).astype(np.float32)
    return Image.fromarray(np.clip(np.round(arr), 0, 255).astype(np.uint8), "L"), truth


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "embossed_seal"]


def main():
    print(f"{'pages':<18}{'cond':<14}{'split':<6}{'found':>8}{'spurious/page':>15}")
    for kind in ["emboss"] + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind != "emboss" and cond != "A 8":
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
                print(f"{kind:<18}{cond if kind == 'emboss' else '-':<14}{split:<6}{(f'{hits}/{total}' if total else '-'):>8}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
