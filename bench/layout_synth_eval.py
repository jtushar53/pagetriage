#!/usr/bin/env python3
"""Check layout segmentation against synthetic pages whose lines, paragraphs and columns are known.

    python -I bench/layout_synth_eval.py
    python -I bench/layout_synth_eval.py --seeds 5

Pages come from bench/synth (Pillow text in Latin; procedural pseudo-glyphs with a headline for
Devanagari and Bengali), A4 at 150 dpi. Each row renders `--seeds` pages and compares what the
`layout` detector counted with what was drawn: text lines, paragraphs (blocks) and columns. The
pages are clean renderings with controlled damage, so these numbers say whether the segmentation
responds correctly to the structure, not how it behaves on real scans. Also times the pass.

Needs the Python binding and `pip install numpy Pillow`.
"""
from __future__ import annotations

import argparse
import json
import random
import statistics
import sys
import time
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DPI = 150
W, H = 1240, 1754  # A4 at 150 dpi
OPTS = json.dumps({"detectors": ["layout"], "debug": True})


def paragraphs_drawn(lines, em=round(11.0 * DPI / 72)):
    """Paragraphs actually on the page: a baseline gap well above the 1.38 em leading starts a new
    one. (Text that does not fit is dropped, so the number asked for can be too high.)"""
    ys = [ln[0].y for ln in lines if ln]
    return 1 + sum(b - a > 1.6 * 1.38 * em for a, b in zip(ys, ys[1:]))


def one_column(script, seed, skew=0.0, noise=2.0, speckle=0.0, blur=0.0, w=W):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, script, 5) for _ in range(6)]
    scan = scanimg.render_scan(rng, np_rng, w, H, DPI, script, paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               skew, noise, speckle, blur, 0.0)
    return scan, paragraphs_drawn(scan.lines)


def two_columns(script, seed):
    """Two independently rendered half-width pages side by side, with a gutter between."""
    left, np_ = one_column(script, seed, w=W // 2)
    right, nr = one_column(script, seed + 1000, w=W // 2)
    img = Image.new("L", (W, H), 255)
    img.paste(left.image, (0, 0))
    img.paste(right.image, (W // 2, 0))
    lines = len(left.lines) + len(right.lines)
    return img, lines, np_ + nr


def run(img):
    t = time.perf_counter()
    out = pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, DPI, None, None, OPTS)
    ms = (time.perf_counter() - t) * 1000
    return json.loads(out)["debug"]["layout"], ms


def row(label, cases):
    """`cases` is a list of (image, true_lines, true_blocks, true_columns)."""
    feats, times = [], []
    for img, *truth in cases:
        f, ms = run(img)
        feats.append((f, truth))
        times.append(ms)
    n = len(cases)
    exact_lines = sum(f["lines"] == t[0] for f, t in feats)
    exact_blocks = sum(f["blocks"] == t[1] for f, t in feats)
    exact_cols = sum(f["columns"] == t[2] for f, t in feats)
    line_err = statistics.mean(abs(f["lines"] - t[0]) / t[0] for f, t in feats)
    print(f"{label:<26} {exact_lines:>2}/{n} {line_err * 100:>6.1f}%   {exact_blocks:>2}/{n}   {exact_cols:>2}/{n}"
          f"   {statistics.median(times):>7.0f}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=3)
    args = ap.parse_args()
    seeds = range(1, args.seeds + 1)
    print(f"{'page':<26} {'lines':>5} {'mean |err|':>10}   {'blocks':>6}   {'cols':>5}   {'ms (med)':>8}")
    for script in ["latin", "devanagari", "bengali"]:
        cases = []
        for s in seeds:
            scan, paras = one_column(script, s)
            cases.append((scan.image, len(scan.lines), paras, 1))
        row(f"{script}, 1 column", cases)
    for script in ["latin", "devanagari"]:
        cases = []
        for s in seeds:
            img, lines, paras = two_columns(script, s)
            cases.append((img, lines, paras, 2))
        row(f"{script}, 2 columns", cases)
    for label, kw in [("latin, skew 1.5 deg", dict(skew=1.5)), ("latin, skew 4 deg", dict(skew=4.0)),
                      ("latin, speckle 0.2%", dict(speckle=0.002)), ("latin, blur 1.2 px", dict(blur=1.2))]:
        cases = []
        for s in seeds:
            scan, paras = one_column("latin", s, **kw)
            cases.append((scan.image, len(scan.lines), paras, 1))
        row(label, cases)


if __name__ == "__main__":
    main()
