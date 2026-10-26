#!/usr/bin/env python3
"""Check the rulings, table and checkbox detector (module M9) against synthetic pages with known truth.

    python -I bench/rulings_synth_eval.py

Pages: A4 at 150 dpi (Pillow font, so Latin text only).
  table     a ruled grid of 2-8 rows and 2-5 columns, 1-3 px lines, text in the cells
  checks    a list of 6-14 items, each a hollow square (1-2 px edge) and a label, some ticked
  mixed     a table, a checkbox list and an underline on one page
  text      paragraphs only, plus a line of text with digits and capital O/D/0: nothing should be found
Degradations: clean, skew 2 degrees, noise 8, blur 1 px, and a 100 dpi rendering (smaller text and
thinner lines). Seeds 1-5 are the dev split (the checkbox and contrast thresholds were looked at on
them), seeds 11-15 the test split.

The pages are clean renderings drawn by the author of the detector: this says the code does what it
was designed to, nothing about real scans of forms.
Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
import zlib
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5], [11, 12, 13, 14, 15]
OPTS = json.dumps({"detectors": ["rulings"], "debug": True})
CONDITIONS = {"clean": {}, "skew 2deg": dict(skew=2.0), "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0),
              "100 dpi": dict(dpi=100)}


def _setup(dpi):
    return (round(1240 * dpi / 150), round(1754 * dpi / 150), round(11.0 * dpi / 72))


def table_on(draw, rng, x0, y0, rows, cols, lw, em, f, width):
    rh, cw = round(em * 2.3), (width - 2 * x0) // cols
    for r in range(rows + 1):
        draw.rectangle([x0, y0 + r * rh, x0 + cols * cw, y0 + r * rh + lw - 1], fill=30)
    for c in range(cols + 1):
        draw.rectangle([x0 + c * cw, y0, x0 + c * cw + lw - 1, y0 + rows * rh + lw - 1], fill=30)
    for r in range(rows):
        for c in range(cols):
            draw.text((x0 + c * cw + em // 2, y0 + r * rh + round(em * 1.6)), rng.choice(synthtext.sentence(rng, "latin").split())[:8], font=f,
                      fill=40, anchor="ls")
    return y0 + rows * rh + em * 2


def checks_on(draw, rng, x0, y0, n, em, f, edge):
    size = round(em * 0.95)
    checked = 0
    for i in range(n):
        y = y0 + i * round(em * 2.0)
        draw.rectangle([x0, y, x0 + size, y + size], outline=30, width=edge)
        if rng.random() < 0.4:
            checked += 1
            if rng.random() < 0.5:
                draw.line([(x0 + 2, y + 2), (x0 + size - 2, y + size - 2)], fill=30, width=edge)
                draw.line([(x0 + size - 2, y + 2), (x0 + 2, y + size - 2)], fill=30, width=edge)
            else:
                draw.line([(x0 + 2, y + size // 2), (x0 + size // 3, y + size - 2), (x0 + size, y - 2)], fill=30, width=edge)
        draw.text((x0 + size + em, y + size), synthtext.sentence(rng, "latin")[:40], font=f, fill=40, anchor="ls")
    return y0 + n * round(em * 2.0) + em, checked


def page(kind, seed, dpi=150, **degrade):
    W, H, em = _setup(dpi)
    rng = random.Random(seed * 7919 + zlib.crc32(kind.encode()) % 1000)
    img = Image.new("L", (W, H), 255)
    draw = ImageDraw.Draw(img)
    f = scanimg.font(em)
    truth = {"tables": [], "boxes": 0, "checked": 0}
    x0, y = int(0.1 * W), int(0.08 * H)
    lw = rng.randint(1, 3) if dpi == 150 else rng.randint(1, 2)
    if kind in ("table", "mixed"):
        rows, cols = rng.randint(2, 8), rng.randint(2, 5)
        y = table_on(draw, rng, x0, y, rows, cols, lw, em, f, W)
        truth["tables"].append((rows, cols))
    if kind in ("checks", "mixed"):
        n = rng.randint(6, 14)
        y, checked = checks_on(draw, rng, x0, y, n, em, f, rng.randint(1, 2))
        truth["boxes"], truth["checked"] = n, checked
    if kind == "mixed":
        draw.rectangle([x0, y + em, x0 + W // 3, y + em + lw - 1], fill=30)
    if kind == "text":
        for i in range(24):
            draw.text((x0, y + i * round(em * 1.4)), synthtext.sentence(rng, "latin")[:75], font=f, fill=40, anchor="ls")
        draw.text((x0, y + 25 * round(em * 1.4)), "Ref. 0 O D Q 10 100 OOO DDD 000 [ ] ( )", font=f, fill=40, anchor="ls")
    return handwrite.finish(img, rng, **{k: v for k, v in degrade.items()}), truth


def analyse(img, dpi):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, dpi, None, None, OPTS))
    return out["findings"]


def run(split, seeds):
    print(f"\n== {split}")
    print(f"{'kind':<8}{'condition':<11}{'tables exact':>13}{'boxes exact':>12}{'ticks exact':>12}{'spurious/page':>14}")
    for kind in ["table", "checks", "mixed", "text"]:
        for cond, kw in CONDITIONS.items():
            kw = dict(kw)
            dpi = kw.pop("dpi", 150)
            t_ok = b_ok = c_ok = spurious = 0
            for seed in seeds:
                img, truth = page(kind, seed, dpi=dpi, **kw)
                fs = analyse(img, dpi)
                tabs = [f for f in fs if f["kind"] == "table"]
                grids = [f for f in fs if f["kind"] == "checkbox_grid"]
                want_t = truth["tables"]
                got_t = sorted((f["attributes"]["rows"], f["attributes"]["cols"]) for f in tabs)
                t_ok += got_t == sorted(want_t)
                got_b = sum(g["attributes"]["boxes"] for g in grids)
                got_c = sum(g["attributes"]["checked"] for g in grids)
                b_ok += got_b == truth["boxes"]
                c_ok += got_c == truth["checked"]
                if kind == "text":
                    spurious += len([f for f in fs if f["kind"] in ("table", "checkbox_grid", "fold_line", "ruling_lines")])
                if kind in ("table", "checks"):  # nothing of the other sort should appear
                    other = "checkbox_grid" if kind == "table" else "table"
                    spurious += len([f for f in fs if f["kind"] == other])
            n = len(seeds)
            print(f"{kind:<8}{cond:<11}{t_ok:>10}/{n}{b_ok:>9}/{n}{c_ok:>9}/{n}{spurious / n:>14.1f}")


if __name__ == "__main__":
    run("dev", DEV)
    run("test", TEST)
