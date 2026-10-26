"""Synthetic wet signatures and look-alikes, for module M7.

Nothing here is a real signature. A signature is a smooth flourish through random control points, with
loops, a long underswing and a pen that changes width; some have a dot or a rule. The look-alikes are
the things a signature detector must not take: lines of handwriting, circles and stamps in black, solid
logos, box frames, cross-outs. Output depends only on the `random.Random` it is given.
"""
from __future__ import annotations

import math
import random

from PIL import Image, ImageDraw

import handwrite


def _loop_points(rng: random.Random, x0: float, y0: float, w: float, h: float):
    """Control points of a flourish: a run of loops that travels left to right, then an underswing."""
    pts = []
    n = rng.randint(3, 6)
    for i in range(n):
        cx = x0 + w * (0.08 + 0.7 * i / n)
        a, b = rng.uniform(0.15, 0.4) * h, rng.uniform(0.2, 0.5) * h
        phase = rng.uniform(0, 2 * math.pi)
        for k in range(8):
            t = phase + 2 * math.pi * k / 8 * rng.choice([1.0, 1.3])
            pts.append((cx + a * 0.6 * math.cos(t) + w * 0.03 * k, y0 + h * 0.5 + b * math.sin(t)))
    # the long underswing
    pts.append((x0 + w * rng.uniform(0.7, 0.95), y0 + h * rng.uniform(0.6, 0.95)))
    pts.append((x0 + w * rng.uniform(0.2, 0.5), y0 + h * rng.uniform(0.75, 1.0)))
    pts.append((x0 + w * rng.uniform(0.85, 1.0), y0 + h * rng.uniform(0.55, 0.8)))
    return pts


def signature(draw: ImageDraw.ImageDraw, rng: random.Random, x: int, y: int, w: int, h: int, ink: int = 40):
    """Draw one signature in the box (x, y, w, h); returns its tight box."""
    pts = _loop_points(rng, x, y, w, h)
    curve = handwrite._spline([(px + rng.gauss(0, 0.01 * w), py + rng.gauss(0, 0.01 * h)) for px, py in pts], steps=8)
    pen = max(2, round(h * rng.uniform(0.03, 0.055)))
    xs, ys = [], []
    for a, b in zip(curve, curve[1:]):
        draw.line([a, b], fill=ink, width=max(1, round(pen * rng.uniform(0.6, 1.5))))
        xs += [a[0], b[0]]
        ys += [a[1], b[1]]
    if rng.random() < 0.4:  # a dot
        dx, dy = x + w * rng.uniform(0.7, 1.0), y + h * rng.uniform(0.1, 0.4)
        draw.ellipse([dx - pen, dy - pen, dx + pen, dy + pen], fill=ink)
        xs.append(dx)
        ys.append(dy)
    return (int(min(xs) - pen), int(min(ys) - pen), int(max(xs) + pen), int(max(ys) + pen))
