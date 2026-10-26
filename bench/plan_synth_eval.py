#!/usr/bin/env python3
"""Check the page plan (module M14) against synthetic forms with known printed and handwritten words.

    python -I bench/plan_synth_eval.py

Pages: A4 at 150 dpi, a printed form with handwriting filled in after some labels
(bench/synth/handwrite.py form_page), clean, tilted 3 degrees, noise 8 and blur 1 px, seeds 1-6 dev and
11-16 test. For every drawn word the plan must send it to the right reader: a handwritten word into a
region routed to `handwriting`, a printed word into a region routed to `ocr` (a word counts as inside
when 60% of its box is). Also reported: how much of the page the regions cover (the share of pixels a
downstream model would be sent instead of the whole page), the scale factor chosen, and, for a second
run on pages turned by 90, 180 and 270 degrees, whether the plan names the turn that restores them (right, abstained, or wrong).

The pages are the author's idea of a filled-in form, with the handwriting detector tuned on the same
generator; nothing here says how a real form behaves. Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
CONDITIONS = {"clean": {}, "tilt 3deg": dict(skew=3.0), "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0)}
OPTS = json.dumps({"detectors": ["plan", "handwriting"]})


def plan_of(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return out["plan"]


def inside(word, region, share=0.6):
    ix = min(word[2], region[2]) - max(word[0], region[0]) + 1
    iy = min(word[3], region[3]) - max(word[1], region[1]) + 1
    if ix <= 0 or iy <= 0:
        return False
    return ix * iy >= share * (word[2] - word[0] + 1) * (word[3] - word[1] + 1)


def covered_share(regions, w, h):
    mask = np.zeros((h, w), bool)
    for r in regions:
        b = r["bbox"]
        mask[b[1]:b[3] + 1, b[0]:b[2] + 1] = True
    return mask.mean()


def main():
    print(f"{'condition':<11}{'split':<6}{'hand words':>12}{'printed words':>15}{'page sent':>11}{'scale':>7}")
    for cond, kw in CONDITIONS.items():
        for split, seeds in (("dev", DEV), ("test", TEST)):
            hand_ok = hand_n = print_ok = print_n = 0
            sent, scale = [], []
            for seed in seeds:
                img, hand, printed = handwrite.form_page(seed, **kw)
                plan = plan_of(img)
                regions = plan["regions"]
                for b in hand:
                    hand_n += 1
                    hand_ok += any(r["route"] == "handwriting" and inside(b, r["bbox"]) for r in regions)
                for b in printed:
                    print_n += 1
                    print_ok += any(r["route"] == "ocr" and inside(b, r["bbox"]) for r in regions)
                sent.append(covered_share(regions, img.width, img.height))
                scale.append(plan["scale"]["factor"])
            print(f"{cond:<11}{split:<6}{f'{hand_ok}/{hand_n}':>12}{f'{print_ok}/{print_n}':>15}{np.mean(sent):>11.2f}{np.mean(scale):>7.2f}")
    print("\nturned pages (3 forms x 3 turns per split): the turn the plan names")
    for split, seeds in (("dev", DEV), ("test", TEST)):
        right = abstain = wrong = 0
        for seed in seeds[:3]:
            img, _, _ = handwrite.form_page(seed)
            for deg, op in ((90, Image.ROTATE_270), (180, Image.ROTATE_180), (270, Image.ROTATE_90)):
                p = plan_of(img.transpose(op))
                if p["rotate_clockwise_deg"] == 0:
                    abstain += 1
                elif p["rotate_clockwise_deg"] == 360 - deg and p["needs_rerun"]:
                    right += 1
                else:
                    wrong += 1
        print(f"{split}: right {right}, abstained {abstain}, wrong {wrong}")

if __name__ == "__main__":
    main()
