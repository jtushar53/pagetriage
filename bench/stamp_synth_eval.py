#!/usr/bin/env python3
"""Check the stamp and seal detector (module M6) against synthetic stamps with known truth.

    python -I bench/stamp_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page on cream paper with one stamp multiplied onto it (blue,
violet, red or green ink; a circle, a double-ringed seal or a rectangle; 1.0 to 2.4 in; tilted up
to 20 degrees; ink dropout; opacity 0.55 to 0.95), or no stamp. Look-alikes that must not be found:
a blue heading block, blue ballpoint handwriting, a red bare table frame, a yellow highlighter bar,
and a multi-colour logo. Conditions: colour, faint (opacity 0.4), blur 1 px, and gray only (the same
pages with colour thrown away: the detector needs colour and reports nothing, which is the expected
0 here).

A stamp is found when a finding of kind stamp or seal has a box overlapping the drawn box with
IoU >= 0.5. Seeds 1-6 are the dev split (the interior and frame thresholds were set looking at
them), seeds 11-16 the test split.

The stamps are drawn by the author of the detector; real stamps smear, overlap text and fade.
Needs the Python binding, numpy and Pillow.
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
import handwrite  # noqa: E402
import scanimg  # noqa: E402
import stampgen  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["stamp"], "debug": True})
KINDS = ["circle", "seal", "rectangle"]


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def make(seed, kind, opacity=None, blur=0.0):
    rng = random.Random(seed * 101 + KINDS.index(kind) * 7 if kind in KINDS else seed * 101)
    np_rng = np.random.default_rng(seed)
    page = stampgen.text_page(rng, np_rng)
    colour = stampgen.COLOURS[rng.choice(list(stampgen.COLOURS))]
    name = [k for k, v in stampgen.COLOURS.items() if v == colour][0]
    alpha = opacity if opacity is not None else rng.uniform(0.55, 0.95)
    d = round(rng.uniform(1.0, 2.4) * 150)
    layer = stampgen.stamp_layer(rng, kind, d, colour, alpha)
    cx, cy = rng.randint(400, 840), rng.randint(900, 1450)
    page, box = stampgen.multiply(page, layer, cx, cy, rng.uniform(-20, 20))
    if blur:
        page = page.filter(ImageFilter.GaussianBlur(blur))
    return page, {"box": box, "kind": "seal" if kind == "seal" else "stamp", "shape": "rectangle" if kind == "rectangle" else "circle", "colour": name}


def lookalike(seed, what):
    rng = random.Random(seed * 211)
    np_rng = np.random.default_rng(seed)
    page = stampgen.text_page(rng, np_rng)
    d = ImageDraw.Draw(page)
    x, y = rng.randint(300, 600), rng.randint(900, 1200)
    f = scanimg.font(round(11 * 150 / 72))
    if what == "blue heading":
        for i in range(5):
            d.text((x, y + i * 40), "THIS DEED OF SALE IS MADE ON THE DAY", font=f, fill=(25, 55, 170), anchor="ls")
    elif what == "blue handwriting":
        hand = Image.new("L", (1240, 1754), 255)
        handwrite.hand_lines(ImageDraw.Draw(hand), rng, x, x + 500, y, 23, 3, ink=40)
        a = np.asarray(hand).astype(np.float32) / 255.0
        p = np.asarray(page).astype(np.float32)
        ink = np.array([25, 55, 170], dtype=np.float32) / 255.0
        page = Image.fromarray(np.clip(p * (a[..., None] + (1 - a[..., None]) * ink), 0, 255).astype(np.uint8), "RGB")
    elif what == "red bare frame":
        d.rectangle([x, y, x + 330, y + 200], outline=(185, 30, 40), width=4)
    elif what == "yellow bar":
        d.rectangle([x, y, x + 400, y + 28], fill=(255, 230, 90))
    elif what == "multicolour logo":
        for k, c in enumerate([(200, 40, 40), (40, 160, 60), (40, 70, 190), (230, 160, 20)]):
            d.rectangle([x + k * 60, y, x + k * 60 + 55, y + 120], fill=c)
    return page


def analyse(img, gray_only=False):
    gray = np.asarray(img.convert("L"))
    rgb = None if gray_only else np.asarray(img).tobytes()
    out = json.loads(pagetriage_py.analyse_page(gray.tobytes(), img.width, img.height, 150, rgb, None, OPTS))
    return [f for f in out["findings"] if f["kind"] in ("stamp", "seal")]


def main():
    print(f"{'condition':<22}{'split':<6}{'found':>8}{'kind ok':>9}{'shape ok':>10}{'colour ok':>10}")
    for cond, kw, gray_only in [("colour", {}, False), ("faint (opacity 0.4)", dict(opacity=0.4), False),
                                ("blur 1 px", dict(blur=1.0), False), ("gray only (unsupported)", {}, True)]:
        for split, seeds in (("dev", DEV), ("test", TEST)):
            n = found = kind_ok = shape_ok = colour_ok = 0
            for seed in seeds:
                for kind in KINDS:
                    if gray_only and kind == "rectangle":
                        continue
                    img, truth = make(seed, kind, **kw)
                    fs = analyse(img, gray_only)
                    n += 1
                    hit = [f for f in fs if truth["box"] and iou(f["bbox"], truth["box"]) >= 0.5]
                    if hit:
                        f = hit[0]
                        found += 1
                        kind_ok += f["kind"] == truth["kind"]
                        shape_ok += f["attributes"]["shape"] == truth["shape"]
                        colour_ok += f["attributes"].get("colour") == {"blue": "blue", "violet": "violet", "red": "red", "green": "green"}[truth["colour"]]
            print(f"{cond:<22}{split:<6}{found:>5}/{n:<3}{kind_ok:>6}/{n:<3}{shape_ok:>7}/{n:<3}{colour_ok:>7}/{n:<3}")
    print("\nfalse stamps (pages with any stamp or seal finding / pages tried)")
    for what in ["no stamp", "blue heading", "blue handwriting", "red bare frame", "yellow bar", "multicolour logo"]:
        row = []
        for gray_only in (False, True):
            hits = 0
            for seed in DEV + TEST:
                img = stampgen.text_page(random.Random(seed), np.random.default_rng(seed)) if what == "no stamp" else lookalike(seed, what)
                hits += bool(analyse(img, gray_only))
            row.append(hits)
        print(f"{what:<18} colour {row[0]:>2}/12   gray only {row[1]:>2}/12")


if __name__ == "__main__":
    main()
