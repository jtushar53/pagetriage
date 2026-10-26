#!/usr/bin/env python3
"""Check the redaction-box detector (module M12, pixel part) against synthetic redactions.

    python -I bench/redaction_synth_eval.py

Pages: A4 at 150 dpi, a Latin text page (bench/synth) with 1 to 4 words blacked out by a solid
rectangle a little bigger than the word, and sometimes a block of 2 to 4 lines blacked out. Look-alikes
that must not be found: filled checkboxes, a solid logo, a thin black rule, a boxed heading whose
text is reversed out (white on black), a table header bar and a plain page. The thick separator bar
(12 px, full width between paragraphs) is reported on its own because it is a known false positive.
Conditions: clean, skew 2 degrees, noise 8, blur 1 px, and "grey" (the box is gray 60, not black).
A box is found when a finding overlaps the drawn one with IoU >= 0.5. Seeds 1-6 dev, 11-16 test.

The boxes are clean synthetic rectangles. Real redactions are often a scan of a marker or a bar drawn
in a PDF editor and then printed, with ragged edges. White-out cannot be found. Needs the Python
binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import random
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402
import scanimg  # noqa: E402
import synthtext  # noqa: E402

DEV, TEST = [1, 2, 3, 4, 5, 6], [11, 12, 13, 14, 15, 16]
OPTS = json.dumps({"detectors": ["redaction"], "debug": True})
CONDITIONS = {"clean": {}, "skew 2deg": dict(skew=2.0), "noise 8": dict(noise=8.0), "blur 1px": dict(blur=1.0),
              "grey box (60)": dict(level=60)}
LOOKALIKES = ["filled checkboxes", "solid logo", "thin rule", "reversed heading", "table header bar", "plain page",
              "thick separator bar"]


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union


def page(kind, seed, skew=0.0, noise=2.0, blur=0.0, level=0):
    rng = random.Random(seed * 17 + len(kind))
    np_rng = np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, "latin", 5) for _ in range(6)]
    scan = scanimg.render_scan(rng, np_rng, 1240, 1754, 150, "latin", paras, 11.0, (0.1, 0.08, 0.1, 0.08),
                               0.0, 2.0, 0.0, 0.0, 0.0)
    img = scan.image.copy()
    d = ImageDraw.Draw(img)
    em = round(11 * 150 / 72)
    truth = []
    lines = [ln for ln in scan.lines if ln]
    if kind == "redaction":
        for _ in range(rng.randint(1, 4)):
            ln = rng.choice(lines)
            # Words under 2.2 x-heights (a letter or two) are not searched for; leave them alone.
            w = rng.choice([x for x in (ln[1:-1] or ln) if x.w >= 36] or ln)
            box = (int(w.x - 3), int(w.y - 0.85 * em), int(w.x + w.w + 3), int(w.y + 0.3 * em))
            d.rectangle(box, fill=level)
            truth.append(box)
        if rng.random() < 0.5:
            i = rng.randint(2, len(lines) - 6)
            n = rng.randint(2, 4)
            x0 = int(lines[i][0].x) - 3
            x1 = int(max(w.x + w.w for ln in lines[i:i + n] for w in ln)) + 3
            box = (x0, int(lines[i][0].y - 0.85 * em), x1, int(lines[i + n - 1][0].y + 0.3 * em))
            d.rectangle(box, fill=level)
            truth.append(box)
    elif kind == "filled checkboxes":
        for i, ln in enumerate(lines[2:9]):
            x = int(ln[0].x) - 40
            d.rectangle([x, int(ln[0].y - em), x + em - 2, int(ln[0].y - 2)], fill=30)
    elif kind == "solid logo":
        d.ellipse([200, 1300, 360, 1460], fill=40)
        d.polygon([(500, 1460), (580, 1300), (660, 1460)], fill=40)
    elif kind == "thin rule":
        d.rectangle([150, 1350, 1090, 1352], fill=30)
    elif kind == "reversed heading":
        d.rectangle([150, 1300, 1090, 1360], fill=30)
        d.text((620, 1345), "SCHEDULE OF PROPERTY", font=scanimg.font(em * 2), fill=255, anchor="ms")
    elif kind == "table header bar":
        d.rectangle([150, 1300, 1090, 1350], fill=40)
        for k in range(4):
            d.text((200 + k * 230, 1335), "Column %d" % (k + 1), font=scanimg.font(em), fill=255, anchor="ls")
    elif kind == "thick separator bar":
        d.rectangle([150, 1300, 1090, 1311], fill=30)
    if skew:
        import math
        t, cx, cy = math.radians(skew), 620, 877
        moved = []
        for x0, y0, x1, y1 in truth:
            pts = [(cx + (x - cx) * math.cos(t) + (y - cy) * math.sin(t), cy - (x - cx) * math.sin(t) + (y - cy) * math.cos(t))
                   for x, y in ((x0, y0), (x1, y0), (x0, y1), (x1, y1))]
            moved.append((int(min(p[0] for p in pts)), int(min(p[1] for p in pts)), int(max(p[0] for p in pts)), int(max(p[1] for p in pts))))
        truth = moved
    return handwrite.finish(img, rng, noise=noise, blur=blur, skew=skew), truth


def found(img):
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, 150, None, None, OPTS))
    return [f for f in out["findings"] if f["kind"] == "redaction_box"]


def main():
    print(f"{'pages':<22}{'cond':<15}{'split':<6}{'found':>8}{'spurious/page':>15}")
    for kind in ["redaction"] + LOOKALIKES:
        for cond, kw in CONDITIONS.items():
            if kind != "redaction" and cond != "clean":
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
                print(f"{kind:<22}{cond:<15}{split:<6}{(f'{hits}/{total}' if total else '-'):>8}{spurious / len(seeds):>15.2f}")


if __name__ == "__main__":
    main()
