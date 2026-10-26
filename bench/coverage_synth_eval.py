#!/usr/bin/env python3
"""Check the extraction-coverage detector against synthetic pages with known text boxes.

    python -I bench/coverage_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page (bench/synth) of 24 to 30 lines. The text boxes are the true
word boxes of a chosen share of the lines (the lines are chosen at random): all lines, 70%, 40%, none,
and "all lines, boxes shifted 1/3 of a line height down" (an OCR layer that sits low). A gap is expected
when the share of uncovered lines is over 20%. Reported: the detector's `ink_covered_share` against the
true share of lines covered (lines have different amounts of ink, so they differ a little), whether a
gap was found, and for the full-coverage case whether a spurious gap was. Seeds 1-6 dev, 11-16 test.

The boxes are exact; a PDF's boxes use a flat glyph width, which this does not simulate. Needs the
Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
CASES = {"all lines": (1.0, 0), "70% of lines": (0.7, 0), "40% of lines": (0.4, 0), "no boxes": (0.0, 0),
         "all, shifted down": (1.0, 1)}
EM = round(11 * 150 / 72)


def run(seed, share, shift):
    rng = random.Random(seed)
    np_rng = np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(6)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, 2.0, 0.0, 0.0, 0.0)
    lines = [ln for ln in scan.lines if ln]
    chosen = set(rng.sample(range(len(lines)), round(share * len(lines))))
    boxes = []
    for i, ln in enumerate(lines):
        if i in chosen:
            y = int(ln[0].y)
            for w in ln:
                boxes.append([int(w.x), y - int(0.8 * EM) + shift * EM // 3, int(w.x + w.w), y + int(0.25 * EM) + shift * EM // 3])
    opts = json.dumps({"detectors": ["coverage"], "debug": True, "text_boxes": boxes})
    out = json.loads(pagetriage_py.analyse_page(np.asarray(scan.image).tobytes(), 1240, 1754, 150, None, None, opts))
    gap = any(f["kind"] == "extraction_gap" for f in out["findings"])
    return out["debug"]["coverage"]["ink_covered_share"], len(chosen) / len(lines), gap


def main():
    print(f"{'case':<20}{'split':<6}{'mean share':>11}{'true share':>11}{'max |diff|':>11}{'gap found':>10}")
    for name, (share, shift) in CASES.items():
        for split, seeds in (("dev", DEV), ("test", TEST)):
            rows = [run(s, share, shift) for s in seeds]
            est = sum(r[0] for r in rows) / len(rows)
            true = sum(r[1] for r in rows) / len(rows)
            diff = max(abs(r[0] - r[1]) for r in rows)
            gaps = sum(r[2] for r in rows)
            print(f"{name:<20}{split:<6}{est:>11.2f}{true:>11.2f}{diff:>11.2f}{gaps:>7}/{len(rows)}")


if __name__ == "__main__":
    main()
