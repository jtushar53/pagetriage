#!/usr/bin/env python3
"""Check the wet-signature detector (module M7) against synthetic signatures and look-alikes.

    python -I bench/signature_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page with a signature (a flourish of 0.9 to 2.6 in, drawn by
`bench/synth/signgen.py`) below the text, sometimes over a signature rule and with a printed name
beneath; 1 to 2 per page. Look-alikes that must not be found: a page of handwriting (the style M5 is
tuned on), a black circular stamp, solid logo shapes, a ruled box frame, a zigzag cross-out of a line
of text, a list of checkboxes, and a plain text page. Conditions: clean, skew 2 degrees, noise 8,
blur 1 px. A signature is found when a finding's box overlaps the drawn one with IoU >= 0.5. Seeds 1-6
are the dev split (thresholds were set looking at them), seeds 11-16 the test split.

The signatures are drawn by the author of the detector, from curves that have the properties it
measures; real ones range from a clean initial to a near-straight line. Needs the Python binding,
numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402
import scanimg  # noqa: E402
import signgen  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["signature"], "debug": True})
CONDITIONS = {"clean": {}, "skew 2deg": dict(skew=2.0), "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0)}
LOOKALIKES = ["handwriting", "black stamp", "solid logo", "box frame", "cross-out", "checkboxes", "text only"]


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def text_base(rng, np_rng):
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(4)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.45), 0.0, 2.0, 0.0, 0.0, 0.0)
    return scan.image.copy()


def page(kind, seed, skew=0.0, noise=2.0, blur=0.0):
    rng = random.Random(seed * 131 + len(kind))
    np_rng = np.random.default_rng(seed)
    img = text_base(rng, np_rng)
    d = ImageDraw.Draw(img)
    f = scanimg.font(round(11 * 150 / 72))
    truth = []
    if kind == "signature":
        for i in range(rng.randint(1, 2)):
            w, h = rng.randint(150, 400), rng.randint(60, 150)
            x, y = rng.randint(150, 700) + i * 0, 1300 + i * 190
            if rng.random() < 0.6:
                d.line([(x - 10, y + h + 12), (x + w + 20, y + h + 12)], fill=60, width=2)
                d.text((x, y + h + 40), "Signature of the Vendor", font=f, fill=40, anchor="ls")
            truth.append(signgen.signature(d, rng, x, y, w, h))
    elif kind == "handwriting":
        handwrite.hand_lines(d, rng, 150, 1000, 1300, 23, 4)
    elif kind == "black stamp":
        cx, cy, r = 600, 1400, rng.randint(70, 130)
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=40, width=5)
        d.ellipse([cx - r + 25, cy - r + 25, cx + r - 25, cy + r - 25], outline=40, width=3)
        d.text((cx, cy), "REGISTRAR", font=f, fill=40, anchor="mm")
    elif kind == "solid logo":
        for k in range(3):
            d.rectangle([200 + k * 150, 1300, 320 + k * 150, 1420], fill=50)
        d.ellipse([700, 1300, 820, 1420], fill=60)
    elif kind == "box frame":
        for r in range(4):
            d.rectangle([200, 1300 + r * 60, 900, 1360 + r * 60], outline=40, width=2)
    elif kind == "cross-out":
        x = 200
        while x < 900:
            d.line([(x, 1290), (x + 14, 1340), (x + 28, 1290)], fill=40, width=2)
            x += 28
    elif kind == "checkboxes":
        for i in range(6):
            d.rectangle([200, 1300 + i * 45, 224, 1324 + i * 45], outline=40, width=2)
            d.text((250, 1322 + i * 45), "Clause accepted", font=f, fill=40, anchor="ls")
    return handwrite.finish(img, rng, noise=noise, blur=blur, skew=skew), truth


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "signature"]


def main():
    print(f"{'pages':<14}{'cond':<11}{'split':<6}{'signatures found':>18}{'spurious/page':>15}")
    for kind in ["signature"] + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind != "signature" and cond in ("noise 8",):
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
                tot = f"{hits}/{total}" if total else "-"
                print(f"{kind:<14}{cond:<11}{split:<6}{tot:>18}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
