"""Evaluation metrics for the benchmark harness. Pure numpy; no algorithm used by the analyser lives
here, only the scoring of its output.

Binarisation (DIBCO) works on boolean ink masks (True = ink). DIBCO ground truth stores ink as black,
so convert with `ink_mask(gray)`.

Detection metrics (F1, IoU) work on counts and `[x0, y0, x1, y1]` boxes. Calibration metrics (ECE,
Brier) take confidences in [0, 1] and boolean outcomes.

`pseudo_f_measure` and `drd` are not implemented yet: pFM needs a skeletonised ground truth with
the DIBCO weight maps, and DRD needs the 5x5 reciprocal-distance weight matrix and non-uniform
block selection. Both arrive with the Howe binariser in Phase 1, where they are needed to check our
port against the published DIBCO numbers.
"""
from __future__ import annotations

import math

import numpy as np


def ink_mask(gray: np.ndarray, threshold: int = 128) -> np.ndarray:
    """Boolean ink mask from a DIBCO-style image where ink is dark."""
    return np.asarray(gray) < threshold


def _check_masks(pred: np.ndarray, gt: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    pred, gt = np.asarray(pred, dtype=bool), np.asarray(gt, dtype=bool)
    if pred.shape != gt.shape:
        raise ValueError(f"shape mismatch: prediction {pred.shape} vs ground truth {gt.shape}")
    return pred, gt


def precision_recall(pred: np.ndarray, gt: np.ndarray) -> tuple[float, float]:
    pred, gt = _check_masks(pred, gt)
    tp = int(np.count_nonzero(pred & gt))
    fp = int(np.count_nonzero(pred & ~gt))
    fn = int(np.count_nonzero(~pred & gt))
    return _safe_div(tp, tp + fp), _safe_div(tp, tp + fn)


def f_measure(pred: np.ndarray, gt: np.ndarray) -> float:
    """DIBCO F-measure, in percent: harmonic mean of ink precision and ink recall."""
    p, r = precision_recall(pred, gt)
    return 100.0 * _safe_div(2 * p * r, p + r)


def psnr(pred: np.ndarray, gt: np.ndarray) -> float:
    """DIBCO PSNR in dB for binary images with C = 1: 10 log10(1 / MSE). Infinite if identical."""
    pred, gt = _check_masks(pred, gt)
    mse = float(np.mean(pred != gt))
    return math.inf if mse == 0.0 else 10.0 * math.log10(1.0 / mse)


def pseudo_f_measure(pred, gt_skeleton, gt_weights):
    raise NotImplementedError("pFM arrives with the Howe binariser in Phase 1")


def drd(pred, gt):
    raise NotImplementedError("DRD arrives with the Howe binariser in Phase 1")


def f1_from_counts(tp: int, fp: int, fn: int) -> float:
    return _safe_div(2 * tp, 2 * tp + fp + fn)


def iou(a, b) -> float:
    """Intersection over union of two `[x0, y0, x1, y1]` boxes (x1, y1 exclusive)."""
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    union = _area(a) + _area(b) - inter
    return _safe_div(inter, union)


def _area(box) -> int:
    return max(0, box[2] - box[0]) * max(0, box[3] - box[1])


def match_boxes(pred_boxes, gt_boxes, iou_threshold: float = 0.5) -> tuple[int, int, int]:
    """Greedy one-to-one matching by descending IoU. Returns (tp, fp, fn)."""
    pairs = sorted(
        ((iou(p, g), i, j) for i, p in enumerate(pred_boxes) for j, g in enumerate(gt_boxes)),
        key=lambda t: (-t[0], t[1], t[2]),
    )
    used_p, used_g, tp = set(), set(), 0
    for score, i, j in pairs:
        if score < iou_threshold:
            break
        if i in used_p or j in used_g:
            continue
        used_p.add(i)
        used_g.add(j)
        tp += 1
    return tp, len(pred_boxes) - tp, len(gt_boxes) - tp


def brier(confidence, outcome) -> float:
    """Mean squared error between confidence and the 0/1 outcome."""
    c, o = _calibration_inputs(confidence, outcome)
    return float(np.mean((c - o) ** 2))


def ece(confidence, outcome, bins: int = 15) -> float:
    """Expected calibration error with equal-width bins: sum over bins of
    (bin share) * |mean outcome - mean confidence|."""
    c, o = _calibration_inputs(confidence, outcome)
    if c.size == 0:
        raise ValueError("ece needs at least one sample")
    # Bin index in [0, bins-1]; confidence 1.0 belongs to the last bin.
    idx = np.minimum((c * bins).astype(int), bins - 1)
    total = 0.0
    for b in range(bins):
        sel = idx == b
        n = int(np.count_nonzero(sel))
        if n:
            total += (n / c.size) * abs(float(o[sel].mean()) - float(c[sel].mean()))
    return total


def _calibration_inputs(confidence, outcome) -> tuple[np.ndarray, np.ndarray]:
    c = np.asarray(confidence, dtype=float)
    o = np.asarray(outcome, dtype=float)
    if c.shape != o.shape:
        raise ValueError("confidence and outcome must have the same shape")
    if c.size and (c.min() < 0.0 or c.max() > 1.0 or np.isnan(c).any()):
        raise ValueError("confidence must be in [0, 1]")
    return c, o


def _safe_div(n: float, d: float) -> float:
    return n / d if d else 0.0
