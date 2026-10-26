#!/usr/bin/env python3
"""Check the continuous-tone picture detector (module M10 region typing) against synthetic pages.

    python -I bench/photo_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page (bench/synth) with, where stated, a picture pasted in. Pictures
(all drawn by numpy, none is a real photograph): `smooth` low-frequency gray noise (gray 40 to 200),
`portrait` a dark oval on a lighter background with texture and a white sky in its top third, and
`passport` a portrait of 1.4 x 1.8 in (a photo on an identity page), `busy` smooth noise with 14 sharp-edged blobs and grain (steep edges inside the picture), `dithered` the portrait as a photocopier prints it (ordered 4 x 4 dither, black and white only). Look-alikes that must not be found:
a plain text page, a ruled table, a solid black disc, an inked ring stamp, dense 7 pt text, a fingerprint-
like ridge oval (the thumb fixture's kind) and a blank page. Conditions: clean, noise 8, blur 1 px. A
picture is found when a finding overlaps it with IoU >= 0.5. Seeds 1-6 dev, 11-16 test.

Real photographs on scanned deeds are often printed at low contrast, dithered, or photocopied to near
bilevel; none of that is simulated. Needs the Python binding, numpy and Pillow.
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
sys.path.insert(0, str(Path(__file__).resolve().parent))
import handwrite  # noqa: E402
import rulings_synth_eval  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["photo"], "debug": True})
CONDITIONS = {"clean": {}, "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0)}
POSITIVES = ["smooth", "portrait", "passport", "busy", "dithered"]
LOOKALIKES = ["text page", "table", "solid disc", "inked ring", "dense 7pt text", "ridge oval", "blank"]
W, H = 1240, 1754


def text_page(rng, np_rng, paragraphs=4, pt=11.0):
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(paragraphs)]
    return scanimg.render_scan(rng, np_rng, W, H, 150, "latin", paras, pt, (0.1, 0.08, 0.1, 0.08), 0.0, 2.0, 0.0, 0.0, 0.0).image.copy()


def smooth(np_rng, w, h):
    coarse = np_rng.random((h // 30 + 2, w // 30 + 2)) * 160 + 40
    return np.asarray(Image.fromarray(coarse.astype(np.uint8), "L").resize((w, h), Image.BICUBIC), dtype=np.float32)


def portrait(np_rng, w, h):
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    bg = 150 + 40 * (yy / h)
    oval = (((xx - w / 2) / (0.32 * w)) ** 2 + ((yy - 0.55 * h) / (0.38 * h)) ** 2) < 1
    out = np.where(oval, 70 + 50 * np.sin(xx / 17.0) * np.cos(yy / 23.0), bg)
    out[: h // 3] = np.where(oval[: h // 3], out[: h // 3], 250)   # sky
    out += np_rng.normal(0, 6, out.shape)
    return out.astype(np.float32)


def busy(np_rng, w, h):
    """A smooth picture with sharp-edged blobs and grain: steep edges inside the picture."""
    out = smooth(np_rng, w, h)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    for _ in range(14):
        cx, cy, r = np_rng.random() * w, np_rng.random() * h, 12 + np_rng.random() * 40
        out = np.where(np.hypot(xx - cx, yy - cy) < r, np.float32(40 + np_rng.random() * 170), out)
    return (out + np_rng.normal(0, 10, out.shape)).astype(np.float32)


def dithered(np_rng, w, h):
    """The `portrait` picture as a photocopier would print it: ordered 4 x 4 dither, black and white only."""
    pic = portrait(np_rng, w, h)
    bayer = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) / 16.0 * 255
    t = np.tile(bayer, (h // 4 + 1, w // 4 + 1))[:h, :w]
    return np.where(pic > t, 255.0, 30.0).astype(np.float32)


def page(kind, seed, noise=2.0, blur=0.0):
    rng = random.Random(seed * 31 + len(kind))
    np_rng = np.random.default_rng(seed)
    truth = []
    if kind == "table":
        img, _ = rulings_synth_eval.page("table", seed, noise=noise, blur=blur)
        return img, truth
    img = text_page(rng, np_rng, 6 if kind == "dense 7pt text" else 3, 7.0 if kind == "dense 7pt text" else 11.0)
    if kind in POSITIVES:
        w, h = {"smooth": (rng.randint(330, 700), rng.randint(300, 600)), "portrait": (rng.randint(300, 520), rng.randint(380, 640)), "busy": (rng.randint(330, 600), rng.randint(300, 560)), "dithered": (rng.randint(300, 520), rng.randint(380, 640)),
                "passport": (210, 270)}[kind]
        x, y = rng.randint(100, W - w - 100), rng.randint(900, H - h - 80)
        pic = {"smooth": smooth, "busy": busy, "dithered": dithered}.get(kind, portrait)(np_rng, w, h)
        a = np.asarray(img, dtype=np.float32).copy()
        a[y:y + h, x:x + w] = np.clip(pic, 0, 255)
        img = Image.fromarray(a.astype(np.uint8), "L")
        truth.append((x, y, x + w - 1, y + h - 1))
    elif kind == "solid disc":
        ImageDraw.Draw(img).ellipse([300, 1000, 560, 1260], fill=40)
    elif kind == "inked ring":
        d = ImageDraw.Draw(img)
        d.ellipse([300, 1000, 560, 1260], outline=60, width=4)
        d.ellipse([320, 1020, 540, 1240], outline=60, width=2)
    elif kind == "ridge oval":
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
        r = np.hypot((xx - 400) / 1.0, (yy - 1100) / 1.25)
        ridge = (np.sin(r * 2 * np.pi / 6.0) > 0) & (r < 110)
        a = np.asarray(img, dtype=np.float32).copy()
        a[ridge] = 50
        img = Image.fromarray(a.astype(np.uint8), "L")
    elif kind == "blank":
        img = Image.new("L", (W, H), 252)
    return handwrite.finish(img, rng, noise=noise, blur=blur, skew=0.0), truth


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "photo"]


def main():
    print(f"{'pages':<16}{'cond':<10}{'split':<6}{'found':>7}{'spurious/page':>15}")
    for kind in POSITIVES + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind in LOOKALIKES and cond != "clean":
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
                print(f"{kind:<16}{cond if kind in POSITIVES else '-':<10}{split:<6}{(f'{hits}/{total}' if total else '-'):>7}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
