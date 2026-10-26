"""Blind-embossed seals for the embossed-seal detector: a height field (rings and a few random arcs
standing in for the lettering) lit from one side, so the page shows only faint shading, no ink."""
from __future__ import annotations

import math
import random

import numpy as np


def _blur(a: np.ndarray, sigma: float) -> np.ndarray:
    r = max(1, int(3 * sigma))
    k = np.exp(-0.5 * (np.arange(-r, r + 1) / sigma) ** 2)
    k /= k.sum()
    a = np.apply_along_axis(lambda m: np.convolve(m, k, mode="same"), 0, a)
    return np.apply_along_axis(lambda m: np.convolve(m, k, mode="same"), 1, a)


def shading(rng: random.Random, diameter: int, amplitude: float, light_deg: float = 45.0) -> np.ndarray:
    """Float array (diameter + margin) squared, gray-level offsets from the paper: positive on the
    lit side of every edge, negative on the shaded side."""
    m = diameter // 8
    n = diameter + 2 * m
    yy, xx = np.mgrid[0:n, 0:n].astype(np.float32)
    r = np.hypot(xx - n / 2, yy - n / 2) / (diameter / 2)
    h = np.zeros((n, n), np.float32)
    h += ((r > 0.88) & (r < 1.0)).astype(np.float32)              # outer rim
    h += ((r > 0.62) & (r < 0.68)).astype(np.float32) * 0.8       # inner ring
    ang = np.arctan2(yy - n / 2, xx - n / 2)
    for _ in range(rng.randint(10, 18)):                          # "lettering": short arcs between the rings
        a0, w = rng.uniform(-math.pi, math.pi), rng.uniform(0.1, 0.3)
        rr = rng.uniform(0.72, 0.84)
        h += (((ang - a0 + math.pi) % (2 * math.pi) - math.pi).__abs__() < w).astype(np.float32) * ((abs(r - rr) < 0.025).astype(np.float32)) * 0.7
    h = _blur(h, max(1.2, diameter / 90))
    gy, gx = np.gradient(h)
    t = math.radians(light_deg)
    s = -(gx * math.cos(t) + gy * math.sin(t))
    s = s / (np.abs(s).max() + 1e-9) * amplitude
    return s
