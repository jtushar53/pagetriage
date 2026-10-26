#!/usr/bin/env python3
"""Check the page-orientation detector against synthetic pages turned by 0, 90, 180 and 270 degrees.

    python -I bench/orientation_synth_eval.py

Pages: A4 at 150 dpi, six paragraphs in one script (bench/synth): Latin (Pillow font, real letter shapes),
and the pseudo-glyph scripts Devanagari, Bengali, Gurmukhi (headline scripts), Gujarati, Tamil, Telugu,
Kannada, Malayalam and Arabic (drawn by bench/synth/scanimg.py, not real letterforms). Each page is
turned by 0, 90, 180 and 270 degrees clockwise and the detector must say how far to turn it back. Conditions: clean,
tilted 3 degrees, noise 8, blur 1.5 px. Seeds 1-3 dev, 11-13 test. Reported: pages out of 12
(4 turns x 3 seeds) per script and condition for which a caller acting on the `misoriented` finding (and
doing nothing when there is none) ends with an upright page, and the mean confidence of the wrong ones.

The pseudo-glyph scripts only imitate where ink sits (a headline, ascenders, descenders, joins); real
Indic and Arabic text has other shapes. Nothing here says how real scanned deeds behave.
Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3], [11, 12, 13]
SCRIPTS = ["latin", "devanagari", "bengali", "gurmukhi", "gujarati", "tamil", "telugu", "kannada", "malayalam", "arabic"]
CONDITIONS = {"clean": {}, "tilt 3deg": dict(skew=3.0), "noise 8": dict(noise=8.0), "blur 1.5px": dict(blur=1.5)}
OPTS = json.dumps({"detectors": ["orientation"], "debug": True})
W, H = 1240, 1754


def page(script, seed, skew=0.0, noise=2.0, blur=0.0):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, script, 6) for _ in range(6)]
    return scanimg.render_scan(rng, np_rng, W, H, 150, script, paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               skew, noise, 0.0, blur, 0.0).image


def turn(img, deg_cw):
    # PIL rotates counter-clockwise; transpose constants give exact quarter turns.
    return {0: img, 90: img.transpose(Image.ROTATE_270), 180: img.transpose(Image.ROTATE_180),
            270: img.transpose(Image.ROTATE_90)}[deg_cw]


def read(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    d = out["debug"]["orientation"]
    flagged = [f["attributes"]["rotate_clockwise_deg"] for f in out["findings"] if f["kind"] == "misoriented"]
    # What a caller acts on: the finding's angle, or 0 when the detector abstains or says "upright".
    return (flagged[0] if flagged else 0), d["confidence"], bool(flagged)


def main():
    for split, seeds in (("dev", DEV), ("test", TEST)):
        print(f"\n== {split}: correct / 12 per script and condition (wrong ones: mean confidence)")
        print(f"{'script':<12}" + "".join(f"{c:>16}" for c in CONDITIONS))
        for script in SCRIPTS:
            cells = []
            for kw in CONDITIONS.values():
                right, wrong_conf = 0, []
                for seed in seeds:
                    base = page(script, seed, **kw)
                    for t in (0, 90, 180, 270):
                        rot, conf, _ = read(turn(base, t))
                        # The image was turned t degrees clockwise; turning it (360 - t) % 360 restores it.
                        if rot == (360 - t) % 360:
                            right += 1
                        else:
                            wrong_conf.append(conf)
                mean = f"{sum(wrong_conf) / len(wrong_conf):.2f}" if wrong_conf else "-"
                cells.append(f"{right}/12 ({mean})")
            print(f"{script:<12}" + "".join(f"{c:>16}" for c in cells))


if __name__ == "__main__":
    main()
