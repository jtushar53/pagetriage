#!/usr/bin/env python3
"""Fit per-kind confidence calibration maps (module M13) by isotonic regression.

    python -I bench/fit_calibration.py labelled.csv > calibration_override.toml
    python -I bench/fit_calibration.py --selftest

`labelled.csv` has one row per finding: `kind,confidence,label` (`kind` as in the report, for example
`stamp`; `confidence` the finding's raw confidence; `label` 1 if a person judged the finding right,
else 0). The output is a TOML override for `[calibration]` that `analyse_page`'s `params` option (or
`pagetriage analyse --params`) accepts: a monotone piecewise-linear map per kind.

Nothing in this repository is labelled on real documents, so no map ships: this is the tool for when
there is such a set. Fitting needs at least `--min-rows` (default 100) rows per kind with both labels
present; kinds with fewer are skipped with a note on stderr. The fit is pool-adjacent-violators on
(confidence, label), thinned to at most `--max-knots` knots; the first and last knots are where the
data begin and end, and values outside are held flat, as the core does.

`--selftest` fits synthetic data drawn from a known miscalibration (the true rate is the square of
the confidence) and checks that the result is monotone, close to the truth, and accepted by the core.
It says nothing about real findings.
"""
from __future__ import annotations

import argparse
import csv
import json
import random
import sys


def pav(points):
    """Pool adjacent violators on (x, y) sorted by x. Returns blocks (x_lo, x_hi, mean, weight)."""
    blocks = []
    for x, y in points:
        blocks.append([x, x, y, 1.0])
        while len(blocks) > 1 and blocks[-2][2] >= blocks[-1][2]:
            b = blocks.pop()
            a = blocks[-1]
            w = a[3] + b[3]
            a[2] = (a[2] * a[3] + b[2] * b[3]) / w
            a[1] = b[1]
            a[3] = w
    return blocks


def knots_from(blocks, max_knots):
    ks = []
    for lo, hi, mean, _ in blocks:
        ks.append((lo, mean))
        if hi > lo:
            ks.append((hi, mean))
    # Strictly increasing x: drop a repeated x, keeping the larger y (monotone anyway).
    out = []
    for x, y in ks:
        if out and x <= out[-1][0]:
            out[-1] = (out[-1][0], max(out[-1][1], y))
        else:
            out.append((x, y))
    while len(out) > max_knots:
        # Remove the interior knot whose removal changes the curve least.
        best, at = None, None
        for i in range(1, len(out) - 1):
            (x0, y0), (x1, y1), (x2, y2) = out[i - 1], out[i], out[i + 1]
            err = abs(y1 - (y0 + (y2 - y0) * (x1 - x0) / (x2 - x0)))
            if best is None or err < best:
                best, at = err, i
        del out[at]
    return out


def fit(rows, min_rows, max_knots):
    by_kind = {}
    for kind, conf, label in rows:
        by_kind.setdefault(kind, []).append((conf, label))
    maps = {}
    for kind in sorted(by_kind):
        pts = sorted(by_kind[kind])
        labels = {y for _, y in pts}
        if len(pts) < min_rows or labels != {0, 1}:
            print(f"skipped {kind}: {len(pts)} rows, labels {sorted(labels)}", file=sys.stderr)
            continue
        ks = knots_from(pav(pts), max_knots)
        if len(ks) >= 2:
            maps[kind] = [[round(x, 6), round(y, 6)] for x, y in ks]
    return maps


def to_toml(maps):
    if not maps:
        return "[calibration]\nmaps = {}\n"
    items = ", ".join(f"{k} = {json.dumps(v)}" for k, v in maps.items())
    return f"[calibration]\nmaps = {{ {items} }}\n"


def read_rows(path):
    rows = []
    with open(path, newline="") as f:
        for r in csv.DictReader(f):
            rows.append((r["kind"].strip(), float(r["confidence"]), int(r["label"])))
    return rows


def selftest():
    rng = random.Random(7)
    rows = []
    for _ in range(4000):
        c = rng.random() * 0.5 + 0.5          # raw confidences are all above 0.5 (they are emitted)
        rows.append(("stamp", c, 1 if rng.random() < c * c else 0))
    maps = fit(rows, 100, 12)
    ks = maps["stamp"]
    assert all(b[0] > a[0] and b[1] >= a[1] for a, b in zip(ks, ks[1:])), ks
    for x in (0.6, 0.75, 0.9):
        y = interpolate(ks, x)
        assert abs(y - x * x) < 0.08, (x, y, x * x)
    toml = to_toml(maps)
    # The core must accept it.
    import numpy as np
    import pagetriage_py
    gray = np.full(64 * 64, 255, np.uint8).tobytes()
    opts = json.dumps({"params": toml, "detectors": ["blank_page"]})
    out = json.loads(pagetriage_py.analyse_page(gray, 64, 64, 150, None, None, opts))
    assert out["findings"], "blank page expected"
    bad = json.dumps({"params": "[calibration]\nmaps = { stamp = [[0.9, 0.5], [0.5, 0.6]] }\n"})
    try:
        pagetriage_py.analyse_page(gray, 64, 64, 150, None, None, bad)
    except ValueError:
        pass
    else:
        raise AssertionError("a non-increasing map must be rejected")
    print("selftest ok:", toml.strip().splitlines()[1][:100], "...")


def interpolate(ks, x):
    if x <= ks[0][0]:
        return ks[0][1]
    if x >= ks[-1][0]:
        return ks[-1][1]
    for (x0, y0), (x1, y1) in zip(ks, ks[1:]):
        if x <= x1:
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("csv", nargs="?")
    ap.add_argument("--min-rows", type=int, default=100)
    ap.add_argument("--max-knots", type=int, default=12)
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return
    if not a.csv:
        ap.error("a CSV file or --selftest")
    sys.stdout.write(to_toml(fit(read_rows(a.csv), a.min_rows, a.max_knots)))


if __name__ == "__main__":
    main()
