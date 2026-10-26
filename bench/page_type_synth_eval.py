#!/usr/bin/env python3
"""Check the page typing (module M10) against synthetic pages of each type.

    python -I bench/page_type_synth_eval.py

Pages: A4 at 150 dpi, drawn by the author of the page typing, one type per page:
  text         five paragraphs of Latin text (Pillow font)
  annotated    a text page with handwritten lines and a signature below the text
  form         a printed form with handwriting filled in (bench/synth/handwrite.py form_page)
  table        a ruled grid with text in the cells (bench/rulings_synth_eval.py)
  photo        a smooth dark halftone-like picture covering most of the page
  blank        white paper with sensor noise
  cover        a stamp-paper-like cover: a large heading and a QR-like symbol at the top, a few lines
  mixed        text in the top half and a picture in the bottom half
Detectors on: layout, handwriting, rulings, signature, thumb, redaction, qr (not stamp: it needs
colour). Conditions: clean, noise 8, blur 1 px. Seeds 1-6 dev (the coefficients in
params/page_type.toml were adjusted on them), 11-16 test.

The pages are the author's idea of each type; nothing here says how real scanned deeds are typed.
Needs the Python binding, numpy and Pillow.
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
import qrgen  # noqa: E402
import rulings_synth_eval  # noqa: E402
import scanimg  # noqa: E402
import signgen  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["layout", "handwriting", "rulings", "signature", "thumb", "redaction", "qr", "photo"]})
CONDITIONS = {"clean": {}, "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0)}
LABEL = {"text": "text", "annotated": "text_with_annotations", "form": "form", "table": "table", "photo": "photo",
         "blank": "blank", "cover": "cover_stamp_paper", "mixed": "mixed"}
TYPES = list(LABEL)
W, H, EM = 1240, 1754, round(11 * 150 / 72)


def text_page(rng, np_rng, paragraphs=5):
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(paragraphs)]
    scan = scanimg.render_scan(rng, np_rng, W, H, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.08), 0.0, 2.0, 0.0, 0.0, 0.0)
    return scan.image.copy()


def picture(img, rng, np_rng, box):
    """A smooth dark picture: low-frequency noise, gray 40 to 200, over box."""
    x0, y0, x1, y1 = box
    h, w = y1 - y0, x1 - x0
    coarse = np_rng.random((h // 40 + 2, w // 40 + 2)) * 160 + 40
    big = Image.fromarray(coarse.astype(np.uint8), "L").resize((w, h), Image.BICUBIC)
    img.paste(big, (x0, y0))


def page(kind, seed, noise=2.0, blur=0.0):
    rng = random.Random(seed * 31 + len(kind))
    np_rng = np.random.default_rng(seed)
    if kind == "table":
        img, _ = rulings_synth_eval.page("table", seed, noise=noise, blur=blur)
        return img
    if kind == "form":
        img, _, _ = handwrite.form_page(seed, noise=noise, blur=blur)
        return img
    if kind == "text":
        img = text_page(rng, np_rng)
    elif kind == "annotated":
        img = text_page(rng, np_rng, 3)
        d = ImageDraw.Draw(img)
        handwrite.hand_lines(d, rng, int(0.1 * W), int(0.9 * W), int(0.62 * H), EM, 7)
        signgen.signature(d, rng, 700, 1560, 320, 110)
    elif kind == "photo":
        img = Image.new("L", (W, H), 255)
        picture(img, rng, np_rng, (110, 150, 1130, 1600))
    elif kind == "blank":
        img = Image.new("L", (W, H), 252)
    elif kind == "cover":
        img = Image.new("L", (W, H), 255)
        d = ImageDraw.Draw(img)
        d.text((120, 190), "NON-JUDICIAL STAMP PAPER", font=scanimg.font(EM * 3), fill=30, anchor="ls")
        qr = qrgen.qr_like(rng, 33, 5)
        img.paste(qr, (880, 40))
        f = scanimg.font(EM)
        for i in range(5):
            d.text((120, 400 + i * 40), synthtext.sentence(rng, "latin")[:60], font=f, fill=40, anchor="ls")
    elif kind == "mixed":
        img = text_page(rng, np_rng, 3)
        d = ImageDraw.Draw(img)
        d.rectangle([0, 900, W, H], fill=255)
        picture(img, rng, np_rng, (110, 950, 1130, 1650))
    return handwrite.finish(img, rng, noise=noise, blur=blur, skew=0.0)


def typed(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return out["page_type"]["label"]


def main():
    for cond, kw in CONDITIONS.items():
        for split, seeds in (("dev", DEV), ("test", TEST)):
            print(f"\n== {cond}, {split}")
            for kind in TYPES:
                got = {}
                for seed in seeds:
                    lab = typed(page(kind, seed, **kw))
                    got[lab] = got.get(lab, 0) + 1
                right = got.get(LABEL[kind], 0)
                others = ", ".join(f"{k} {v}" for k, v in sorted(got.items()) if k != LABEL[kind])
                print(f"{kind:<10}{right}/{len(seeds)}  {others}")


if __name__ == "__main__":
    main()
