"""Synthetic handwriting and forms, for the handwriting-vs-print evaluation (module M5).

Nothing here is a real document and nothing is real handwriting. A "handwritten" word is a chain of
wobbly strokes: each letter is a smooth random curve through five control points with a slant that is
steady within a word, a pen width that changes from segment to segment, a baseline that wanders, and
letter heights that vary (some tall, some with descenders). It has the properties the detector looks
at, because it was written by the same author; real handwriting is more varied. Output depends only
on the `random.Random` it is given.
"""
from __future__ import annotations

import math
import random

import numpy as np
from PIL import Image, ImageDraw

import scanimg


def _spline(points, steps=10):
    """Catmull-Rom through `points`, ends repeated."""
    pts = [points[0]] + list(points) + [points[-1]]
    out = []
    for i in range(1, len(pts) - 2):
        p0, p1, p2, p3 = pts[i - 1], pts[i], pts[i + 1], pts[i + 2]
        for s in range(steps):
            t = s / steps
            t2, t3 = t * t, t * t * t
            out.append(tuple(0.5 * ((2 * p1[k]) + (-p0[k] + p2[k]) * t + (2 * p0[k] - 5 * p1[k] + 4 * p2[k] - p3[k]) * t2
                                    + (-p0[k] + 3 * p1[k] - 3 * p2[k] + p3[k]) * t3) for k in (0, 1)))
    out.append(points[-1])
    return out


def _letter_path(rng, kind, cx, base, cw, h, em):
    """Control points of one letter, in an upright frame (slant is applied by the caller)."""
    j = lambda: rng.gauss(0, 0.015 * em)  # noqa: E731 - tiny hand tremor
    if kind == "loop":  # o, a, e, c: a closed round stroke
        pts = []
        turns = rng.choice([1.0, 1.25, 1.5])
        n = 9
        for i in range(n + 1):
            t = 2 * math.pi * turns * i / n
            pts.append((cx + cw * (0.5 + 0.5 * math.sin(t)) + j(), base - h * (0.5 - 0.5 * math.cos(t)) + j()))
        return pts
    if kind == "arch":  # n, m, u, v: one or two humps
        humps = rng.choice([1, 2])
        pts = [(cx, base - h * 0.8)]
        for k in range(humps):
            x1 = cx + cw * (k + 0.5) / humps
            x2 = cx + cw * (k + 1.0) / humps
            pts += [(x1 + j(), base - h * 1.0 + j()), (x2 + j(), base + j())]
        return pts
    if kind == "ascender":  # l, b, h, k: a tall stroke and a loop back down
        top = base - h * rng.uniform(1.9, 2.4)
        return [(cx + 0.1 * cw, base + j()), (cx + 0.35 * cw, top + j()), (cx + 0.5 * cw, base - h * 0.6),
                (cx + 0.85 * cw, base - h * 0.9 + j()), (cx + cw, base + j())]
    # descender: g, y, p, j: down below the baseline and back
    low = base + h * rng.uniform(0.8, 1.3)
    return [(cx + 0.2 * cw, base - h * 0.9 + j()), (cx + 0.5 * cw, base - h * 0.3), (cx + 0.55 * cw, low + j()),
            (cx + 0.2 * cw, low + 0.2 * h), (cx + 0.1 * cw, low - 0.1 * h), (cx + cw, base - 0.2 * h + j())]


def hand_word(draw: ImageDraw.ImageDraw, rng: random.Random, letters: int, x: float, yb: float, em: float,
              ink: int = 40, pen: float | None = None) -> tuple[float, tuple[int, int, int, int]]:
    """Draw one word; returns (end x, bounding box)."""
    pen = pen if pen is not None else max(1.0, em * 0.09)
    slant = rng.uniform(0.1, 0.45)
    wander = 0.0
    x0, ys, cx, prev = x, [], x, None
    for _ in range(letters):
        wander = max(-0.2 * em, min(0.2 * em, wander + rng.gauss(0, 0.05 * em)))
        base = yb + wander
        cw = em * rng.uniform(0.3, 0.55)
        h = em * rng.uniform(0.28, 0.5)
        kind = rng.choices(["loop", "arch", "ascender", "descender"], [4, 4, 1.5, 1])[0]
        pts = [(px + slant * (base - py), py) for px, py in _letter_path(rng, kind, cx, base, cw, h, em)]
        curve = _spline(pts)
        if prev is not None and rng.random() < 0.7:
            curve = [prev] + curve
        pressure = rng.uniform(0.8, 1.4)  # pen pressure changes letter to letter
        for a, b in zip(curve, curve[1:]):
            draw.line([a, b], fill=ink, width=max(1, round(pen * pressure)))
            ys.extend([a[1], b[1]])
        prev = curve[-1]
        cx += cw * rng.uniform(0.8, 1.1)
    return cx, (int(x0), int(min(ys) - pen), int(cx + slant * em * 0.5), int(max(ys) + pen))


def scribble_word(draw, rng, letters, x, yb, em, ink=40):
    """A second, different kind of handwriting, for testing how well a detector tuned on `hand_word`
    carries over: each letter is six random points in a cell joined by straight segments of 1-2
    pixels (no smooth curves, no slant structure), the baseline wandering by up to a fifth of the
    text height. Returns (end x, box)."""
    cell = max(4, round(0.45 * em))
    tall = max(5, round(0.75 * em))
    wander, prev, ys, x0 = 0, None, [], x
    for _ in range(letters):
        wander = max(-round(0.2 * em), min(round(0.2 * em), wander + rng.randint(-1, 1)))
        pts = sorted((x + rng.randint(0, cell), yb + wander - rng.randint(0, tall)) for _ in range(6))
        if prev is not None:
            pts.insert(0, prev)
        for a, b in zip(pts, pts[1:]):
            draw.line([a, b], fill=ink, width=rng.randint(1, 2))
            ys.extend([a[1], b[1]])
        prev = pts[-1]
        x += cell + rng.randint(0, 2)
    return x, (int(x0), int(min(ys)) - 1, int(x), int(max(ys)) + 1)


_BLOCK = {  # strokes of block capitals on a 0..1 x 0..1 cell (y down), as (x0, y0, x1, y1)
    "L": [(0, 0, 0, 1), (0, 1, 1, 1)], "T": [(0, 0, 1, 0), (0.5, 0, 0.5, 1)], "E": [(0, 0, 0, 1), (0, 0, 1, 0), (0, 0.5, 0.8, 0.5), (0, 1, 1, 1)],
    "H": [(0, 0, 0, 1), (1, 0, 1, 1), (0, 0.5, 1, 0.5)], "A": [(0, 1, 0.5, 0), (0.5, 0, 1, 1), (0.2, 0.6, 0.8, 0.6)],
    "N": [(0, 1, 0, 0), (0, 0, 1, 1), (1, 1, 1, 0)], "O": [(0, 0, 1, 0), (1, 0, 1, 1), (1, 1, 0, 1), (0, 1, 0, 0)],
    "I": [(0.5, 0, 0.5, 1)], "R": [(0, 1, 0, 0), (0, 0, 1, 0), (1, 0, 1, 0.5), (1, 0.5, 0, 0.5), (0.3, 0.5, 1, 1)],
    "K": [(0, 0, 0, 1), (1, 0, 0, 0.55), (0.3, 0.45, 1, 1)], "M": [(0, 1, 0, 0), (0, 0, 0.5, 0.6), (0.5, 0.6, 1, 0), (1, 0, 1, 1)],
    "S": [(1, 0, 0, 0), (0, 0, 0, 0.5), (0, 0.5, 1, 0.5), (1, 0.5, 1, 1), (1, 1, 0, 1)],
}


def block_word(draw, rng, letters, x, yb, em, ink=40):
    """Hand-printed block capitals: every letter is drawn from straight strokes with its own size,
    slant, spacing and jitter, as someone filling in a form in capitals does. A third style, used
    only to test (the detector was never tuned on it). Returns (end x, box)."""
    wander, ys, x0 = 0.0, [], x
    pen = max(1, round(em * rng.uniform(0.07, 0.12)))
    for _ in range(letters):
        wander = max(-0.12 * em, min(0.12 * em, wander + rng.gauss(0, 0.04 * em)))
        ch = rng.choice(list(_BLOCK))
        h = em * rng.uniform(0.62, 0.9)
        w = em * rng.uniform(0.35, 0.6)
        slant = rng.uniform(-0.1, 0.25)
        for x0s, y0s, x1s, y1s in _BLOCK[ch]:
            pts = []
            for fx, fy in ((x0s, y0s), (x1s, y1s)):
                py = yb + wander - (1 - fy) * h + rng.gauss(0, 0.02 * em)
                pts.append((x + fx * w + slant * (yb - py) + rng.gauss(0, 0.03 * em), py))
            draw.line(pts, fill=ink, width=pen)
            ys.extend([pts[0][1], pts[1][1]])
        x += w + em * rng.uniform(0.12, 0.3)
    return x, (int(x0), int(min(ys) - pen), int(x), int(max(ys) + pen))


def hand_lines(draw, rng, x0, x1, yb, em, lines, lead=2.1, ink=40):
    """`lines` lines of handwriting between x0 and x1; returns the word boxes."""
    boxes = []
    for _ in range(lines):
        x = x0
        drift = rng.gauss(0, 0.15 * em)
        while x < x1 - 4 * em:
            n = rng.randint(3, 8)
            x, box = hand_word(draw, rng, n, x, yb + drift, em, ink)
            boxes.append(box)
            x += em * rng.uniform(0.35, 0.7)
        yb += lead * em
    return boxes


def finish(img: Image.Image, rng: random.Random, noise=2.0, blur=0.0, skew=0.0):
    from PIL import ImageFilter
    if abs(skew) > 1e-6:
        img = img.rotate(skew, resample=Image.BICUBIC, fillcolor=255)
    if blur > 0:
        img = img.filter(ImageFilter.GaussianBlur(blur))
    arr = np.asarray(img, dtype=np.float32)
    if noise > 0:
        arr = arr + np.random.default_rng(rng.randrange(1 << 30)).normal(0, noise, arr.shape).astype(np.float32)
    return Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8), "L")


def hand_page(seed, w=1240, h=1754, dpi=150, pt=11.0, style="cursive", **degrade):
    """A page of handwriting; every word box returned. `style` is "cursive" (`hand_word`, the style
    the detector was tuned on) or "scribble" (`scribble_word`) or "block" (`block_word`)."""
    rng = random.Random(seed)
    em = round(pt * dpi / 72)
    img = Image.new("L", (w, h), 255)
    draw = ImageDraw.Draw(img)
    if style == "cursive":
        boxes = hand_lines(draw, rng, int(0.1 * w), int(0.9 * w), int(0.1 * h), em, 22)
    else:
        word = scribble_word if style == "scribble" else block_word
        boxes, yb = [], int(0.1 * h)
        for _ in range(22):
            x = int(0.1 * w)
            while x < int(0.9 * w) - 4 * em:
                x, box = word(draw, rng, rng.randint(3, 8), x, yb, em)
                boxes.append(box)
                x += int(em * rng.uniform(0.35, 0.7))
            yb += round(2.1 * em)
    return finish(img, rng, **degrade), boxes


LABELS = ["Name of the vendor", "Father's name", "Plot number", "Registration number", "Date of execution",
          "Sub-registrar office", "Village", "Consideration amount", "Witness", "Address of purchaser"]


def form_page(seed, w=1240, h=1754, dpi=150, pt=11.0, **degrade):
    """A printed form (Latin labels, Pillow font) with handwriting filled in after some labels, on a
    thin rule. Returns (image, [hand word boxes], [printed word boxes])."""
    import synthtext
    rng = random.Random(seed)
    em = round(pt * dpi / 72)
    img = Image.new("L", (w, h), 255)
    draw = ImageDraw.Draw(img)
    f = scanimg.font(em)
    hand_boxes, print_boxes = [], []
    y = int(0.1 * h)
    for row in range(18):
        yb = y + row * int(2.3 * em)
        if rng.random() < 0.3:  # a printed paragraph line instead of a field
            text = synthtext.sentence(rng, "latin")[:70]
            draw.text((int(0.1 * w), yb), text, font=f, fill=40, anchor="ls")
            print_boxes.append((int(0.1 * w), yb - em, int(0.1 * w + f.getlength(text)), yb + em // 3))
            continue
        label = rng.choice(LABELS) + ":"
        lx = int(0.1 * w)
        draw.text((lx, yb), label, font=f, fill=40, anchor="ls")
        end = lx + int(f.getlength(label))
        print_boxes.append((lx, yb - em, end, yb + em // 3))
        fx = end + em
        fill_end = int(0.85 * w)
        draw.line([(fx, yb + 4), (fill_end, yb + 4)], fill=60, width=1)
        if rng.random() < 0.65:
            x = fx + em * 0.3
            for _ in range(rng.randint(1, 3)):
                x, box = hand_word(draw, rng, rng.randint(3, 8), x, yb - 2, em)
                hand_boxes.append(box)
                x += em * rng.uniform(0.4, 0.8)
    return finish(img, rng, **degrade), hand_boxes, print_boxes
