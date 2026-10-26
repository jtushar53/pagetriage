"""Synthetic fingerprint-like impressions and look-alikes, for module M8.

Nothing here is a real fingerprint. A print is a field of curved ridges (the level sets of a phase
that grows with an elliptical distance from an off-centre core, with a low-frequency warp), inked at
about half duty, inside an elliptical envelope with pressure that varies across it. The look-alikes
are textures a ridge detector must not take: a block of text, hatching, a barcode, a halftone, a solid
ink blob. Output depends only on the generators it is given.
"""
from __future__ import annotations

import numpy as np
from PIL import Image


def _smooth_noise(np_rng, h, w, cell):
    small = np_rng.random((h // cell + 2, w // cell + 2)).astype(np.float32)
    img = Image.fromarray((small * 255).astype(np.uint8), "L").resize((w + cell, h + cell), Image.BICUBIC)
    return np.asarray(img, dtype=np.float32)[:h, :w] / 255.0


def fingerprint_alpha(np_rng, rng, width_px: int, height_px: int, period: float, pressure: float = 0.95,
                      blur_px: float = 0.0):
    """Ink alpha (0..1) of a print filling a width x height box."""
    h, w = height_px, width_px
    ys, xs = np.mgrid[0:h, 0:w].astype(np.float32)
    a, b = w / 2.0, h / 2.0
    cx0, cy0 = a * rng.uniform(-0.2, 0.2), b * rng.uniform(-0.25, 0.1)
    x, y = xs - a, ys - b
    ex, ey = rng.uniform(0.55, 0.8), rng.uniform(0.9, 1.3)
    r = np.sqrt(((x - cx0) * ex) ** 2 + ((y - cy0) * ey) ** 2)
    warp = 0.6 * period * np.sin(y / (0.5 * b) + rng.uniform(0, 6.28)) * np.cos(x / (0.7 * a) + rng.uniform(0, 6.28))
    # An angular term turns the concentric rings into loops and whorls.
    ang = np.arctan2(y - cy0, x - cx0)
    phase = 2 * np.pi * (r + warp + rng.choice([0.0, 0.25, 0.5]) * period * ang / (2 * np.pi)) / period
    ridge = np.clip(2.2 * np.cos(phase) + 0.3, 0, 1)
    env = np.clip((1.0 - np.sqrt((x / a) ** 2 + (y / b) ** 2)) * 7.0, 0, 1)
    press = pressure * (0.65 + 0.35 * _smooth_noise(np_rng, h, w, 18))
    alpha = ridge * env * press
    if blur_px > 0:
        img = Image.fromarray((alpha * 255).astype(np.uint8), "L")
        from PIL import ImageFilter
        alpha = np.asarray(img.filter(ImageFilter.GaussianBlur(blur_px)), dtype=np.float32) / 255.0
    return alpha


def paste_alpha(page: Image.Image, alpha, cx: int, cy: int, ink_gray: int = 55):
    """Multiply-blend an ink alpha into a gray page at its centre; returns the box of the ink."""
    arr = np.asarray(page, dtype=np.float32)
    h, w = alpha.shape
    x0, y0 = cx - w // 2, cy - h // 2
    sub = arr[y0:y0 + h, x0:x0 + w]
    arr[y0:y0 + h, x0:x0 + w] = sub * (1 - alpha * (1 - ink_gray / 255.0))
    ys, xs = np.nonzero(alpha > 0.15)
    box = (int(xs.min()) + x0, int(ys.min()) + y0, int(xs.max()) + x0, int(ys.max()) + y0)
    return Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8), "L"), box
