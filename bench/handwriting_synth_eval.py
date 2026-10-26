#!/usr/bin/env python3
"""Check the handwriting-vs-print detector (module M5) against synthetic pages with known truth.

    python -I bench/handwriting_synth_eval.py

Pages: A4 at 150 dpi.
  printed   Latin (Pillow font) and Devanagari (pseudo-glyphs): there should be no handwriting
  confuser  pseudo-Arabic, drawn as joined round letters on a baseline, which looks like cursive
            handwriting to the features: reported on its own
  hand      pages of synthetic handwriting (bench/synth/handwrite.py): all of it should be found.
            Three styles: "cursive" (round joined loops: the style the thresholds were tuned on),
            "scribble" (random straight segments) and "block" (hand-printed capitals), the last two
            never tuned on and reported as they come
  form      a printed form with synthetic handwriting filled in after some labels, on a thin rule
            (and printed paragraph lines): the handwriting should be found and the print not

Recall is the share of truth handwriting boxes whose centre lies inside a reported region; precision
is the share of the reported area that lies inside a truth handwriting box. Seeds 1-5 are the dev
split (thresholds were tuned on seeds 1-3), seeds 11-15 the test split.

The handwriting is drawn by the same author as the detector, so this says the code does what it was
designed to do and nothing about real handwriting. Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

W, H, DPI = 1240, 1754, 150
OPTS = json.dumps({"detectors": ["handwriting"], "debug": True})
DEV, TEST = [1, 2, 3, 4, 5], [11, 12, 13, 14, 15]
CONDITIONS = {"clean": {}, "noise 8": dict(noise=8.0), "blur 1 px": dict(blur=1.0)}


PARAMS = None  # a TOML override, set by --params


def regions(img):
    opts = {"detectors": ["handwriting"], "debug": True}
    if PARAMS:
        opts["params"] = PARAMS
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, DPI,
                                                None, None, json.dumps(opts)))
    return [f["bbox"] for f in out["findings"] if f["kind"] == "handwriting"], out["debug"]["handwriting"]


def printed_page(script, seed, **kw):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, script, 5) for _ in range(6)]
    noise = kw.get("noise", 2.0)
    return scanimg.render_scan(rng, np_rng, W, H, DPI, script, paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, noise, 0.0, kw.get("blur", 0.0), 0.0).image


def mask(boxes, scale=4):
    m = np.zeros((H // scale + 1, W // scale + 1), bool)
    for x0, y0, x1, y1 in boxes:
        m[y0 // scale:y1 // scale + 1, max(0, x0) // scale:x1 // scale + 1] = True
    return m


def score(found, truth):
    """(truth boxes hit, truth boxes, reported area inside truth, reported area). A region merges
    neighbouring words, so the truth for precision is each box widened by the gap it may bridge."""
    fm = mask(found)
    tm = mask([(x0 - 30, max(0, y0 - 4), x1 + 30, y1 + 4) for x0, y0, x1, y1 in truth])
    hit = sum(fm[(y0 + y1) // 8, (x0 + x1) // 8] for x0, y0, x1, y1 in truth)
    return int(hit), len(truth), int((fm & tm).sum()), int(fm.sum())


def main():
    global PARAMS
    if "--params" in sys.argv:
        PARAMS = sys.argv[sys.argv.index("--params") + 1]
    print(f"{'pages':<22}{'cond':<10}{'split':<6}{'pages w/ any region':>20}{'recall':>9}{'precision':>11}")
    for kind in ["latin print", "devanagari print", "arabic (confuser)", "handwriting", "scribble (untuned)",
                 "block (untuned)", "form"]:
        for cond, kw in CONDITIONS.items():
            for split, seeds in (("dev", DEV), ("test", TEST)):
                flagged, hit, n_truth, inside, area = 0, 0, 0, 0, 0
                for seed in seeds:
                    truth = []
                    if kind.endswith("print") or kind.startswith("arabic"):
                        img = printed_page(kind.split()[0], seed, **kw)
                    elif kind == "handwriting":
                        img, truth = handwrite.hand_page(seed, **kw)
                    elif kind.startswith(("scribble", "block")):
                        img, truth = handwrite.hand_page(seed, style=kind.split()[0], **kw)
                    else:
                        img, truth, _ = handwrite.form_page(seed, **kw)
                    found, _ = regions(img)
                    flagged += bool(found)
                    h, n, i, a = score(found, truth)
                    hit, n_truth, inside, area = hit + h, n_truth + n, inside + i, area + a
                rec = f"{hit / n_truth:.0%}" if n_truth else "-"
                prec = f"{inside / area:.0%}" if area else "-"
                print(f"{kind:<22}{cond:<10}{split:<6}{flagged:>14}/{len(seeds)}{rec:>11}{prec:>11}")


if __name__ == "__main__":
    main()
