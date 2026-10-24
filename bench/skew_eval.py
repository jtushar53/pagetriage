#!/usr/bin/env python3
"""Skew-estimator stress test on DIBCO page images (module M3, acceptance criterion 1).

For every DIBCO page image (originals, not ground truth) we estimate the skew of the page as it is,
then rotate the page by each angle in a fixed list and estimate again. The originals have their own
intrinsic skew, so the error of one measurement is

    error = est(rotated) - est(original) - theta

Positive angles are counter-clockwise, the same sense as Pillow's `Image.rotate` and as the
`quality.skew_deg` field. This measures how reproducibly the estimator follows a known rotation, not
how close it is to the "true" skew of a handwritten manuscript (which has no single true skew).

DIBCO is NOT Indian-deed material: it is ink on aged paper, mostly handwritten or machine-printed
Latin text, cropped to a few lines. Treat the numbers as a regression and sanity check only.

    python -I bench/skew_eval.py --data bench/data
    python -I bench/skew_eval.py --data bench/data --sets tune --fill paper
    python -I bench/skew_eval.py --data bench/data --augraphy

Needs the Python binding (`maturin develop --release -m crates/pagetriage-py/Cargo.toml`) and
Pillow. `--augraphy` additionally needs `pip install augraphy`.

Tuning split. Parameters were tuned on the "tune" sets only; the "heldout" sets were not looked at
until the final run (see docs/detectors/quality.md for the history).
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
from pathlib import Path

import pagetriage_py
from PIL import Image

Image.MAX_IMAGE_PIXELS = 400_000_000

# Fixed, deterministic. Spread over [-10, 10] and deliberately off the 0.5-degree coarse grid.
ANGLES = [-10.0, -8.3, -6.1, -4.4, -2.7, -1.2, 0.6, 2.2, 3.9, 5.8, 7.4, 10.0]

# Augraphy's rotation (Geometric(rotate_range=...)) takes whole degrees only, so its variant uses these.
ANGLES_AUGRAPHY = [-10, -8, -6, -4, -3, -2, 1, 2, 3, 5, 7, 10]

# Folder layout is in bench/README.md (the organisers' folder names keep their typos).
SETS = {
    "dibco2009": [
        "dibco2009/DIBC02009_Test_images-handwritten/DIBC02009_Test_images-handwritten",
        "dibco2009/DIBCO2009_Test_images-printed/DIBCO2009_Test_images-printed",
    ],
    "dibco2010": ["dibco2010/H_DIBCO2010_test_images"],
    "dibco2011": ["dibco2011/DIBCO11-handwritten", "dibco2011/DIBCO11-machine_printed"],
    "dibco2012": ["dibco2012/H-DIBCO2012-dataset/H-DIBCO2012-dataset/H-DIBCO2012-dataset"],
    "dibco2013": ["dibco2013/DIBCO2013-dataset/OriginalImages"],
    "dibco2014": ["dibco2014/original_images"],
    "dibco2016": ["dibco2016/DIBCO2016_dataset-original/DIPCO2016_dataset"],
    "dibco2017": ["dibco2017/DIBCO2017_Dataset/Dataset"],
    "dibco2018": ["dibco2018/dibco2018_Dataset/dataset"],
    "dibco2019_track_a": ["dibco2019_track_a/dibco2019_dataset_trackA/Dataset"],
    "dibco2019_track_b": ["dibco2019_track_b/dibco2019_dataset_trackB/Dataset"],
}
IMAGE_SUFFIXES = {".png", ".bmp", ".tif", ".tiff", ".jpg", ".jpeg"}

# Split by edition, fixed before any tuning.
TUNE = ["dibco2009", "dibco2012", "dibco2016", "dibco2018", "dibco2019_track_a"]
HELDOUT = [s for s in SETS if s not in TUNE]

TOLERANCE_DEG = 0.3  # acceptance: median |error| <= 0.3
GROSS_DEG = 1.0  # "failed" measurement: the estimator lost the rotation entirely


def list_pages(data: Path, key: str):
    pages = []
    for rel in SETS[key]:
        folder = data / rel
        if not folder.is_dir():
            sys.exit(f"missing {folder}: is --data pointing at bench/data?")
        for path in sorted(folder.rglob("*")):
            if path.suffix.lower() in IMAGE_SUFFIXES and "_GT" not in path.name and path.is_file():
                pages.append(path)
    return pages


def to_gray(img: Image.Image) -> Image.Image:
    """Gray plane through the core's own fixed-point luma (as bench/run.py does)."""
    if img.mode == "L":
        return img.copy()
    rgb = img.convert("RGB")
    gray = pagetriage_py.rgb_to_gray(rgb.tobytes())
    return Image.frombytes("L", rgb.size, bytes(gray))


def paper_level(gray: Image.Image) -> int:
    """Median gray of the page: a stand-in for the paper colour, used by --fill paper."""
    hist = gray.histogram()
    half = sum(hist) / 2
    acc = 0
    for level, n in enumerate(hist):
        acc += n
        if acc >= half:
            return level
    return 255


def estimate(gray: Image.Image, params: str | None) -> dict:
    opts = {"detectors": [], "debug": True}
    if params:
        opts["params"] = params
    report = json.loads(
        pagetriage_py.analyse_page(
            gray.tobytes(), gray.width, gray.height, 150, None, None, json.dumps(opts)
        )
    )
    q = report["quality"]
    feats = report["debug"]["quality"]
    return {"skew": q["skew_deg"], "ratio": feats.get("skew_peak_ratio", 0.0)}


def augraphy_rotate(gray: Image.Image, angle: int):
    """Rotate with Augraphy's Geometric augmentation instead of Pillow (counter-clockwise for
    positive angles, canvas enlarged, exposed corners black: OpenCV's default border). Whole
    degrees only. Returns None if augraphy is not installed."""
    try:
        import numpy as np
        from augraphy import Geometric
    except Exception:
        return None
    out = Geometric(rotate_range=(angle, angle), p=1.0)(np.asarray(gray))
    return Image.fromarray(out).convert("L")


def stats(errors: list[float]) -> dict:
    if not errors:
        return {}
    a = sorted(abs(e) for e in errors)
    p90 = a[min(len(a) - 1, int(round(0.9 * (len(a) - 1))))]
    return {
        "n": len(a),
        "median_abs": statistics.median(a),
        "p90_abs": p90,
        "max_abs": a[-1],
        "median_signed": statistics.median(errors),
        "over_tolerance": sum(1 for e in a if e > TOLERANCE_DEG),
        "over_gross": sum(1 for e in a if e > GROSS_DEG),
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--data", type=Path, default=Path("bench/data"), help="DIBCO root (bench/data)")
    ap.add_argument("--sets", default="all", help="all, tune, heldout, or comma-separated set keys")
    ap.add_argument("--fill", choices=["white", "paper"], default="white",
                    help="colour of the corners a rotation exposes (default white; paper = page median)")
    ap.add_argument("--params", help="TOML override for the analyser, e.g. '[quality]\\nskew_criterion=\"gradient\"'")
    ap.add_argument("--augraphy", action="store_true",
                    help="use augraphy's rotation (whole degrees, black corners) instead of Pillow's")
    ap.add_argument("--out", type=Path, help="write one JSON line per measurement here")
    ap.add_argument("--limit", type=int, default=0, help="at most this many pages per set (smoke test)")
    ap.add_argument("--show-worst", type=int, default=8, help="list this many worst measurements")
    args = ap.parse_args(argv)

    if args.sets == "all":
        keys = list(SETS)
    elif args.sets == "tune":
        keys = TUNE
    elif args.sets == "heldout":
        keys = HELDOUT
    else:
        keys = args.sets.split(",")
        for k in keys:
            if k not in SETS:
                sys.exit(f"unknown set {k}; known: {', '.join(SETS)}")

    rows = []  # one per measurement
    originals = {}
    for key in keys:
        pages = list_pages(args.data, key)
        if args.limit:
            pages = pages[: args.limit]
        for path in pages:
            with Image.open(path) as img:
                img.load()
                gray = to_gray(img)
            base = estimate(gray, args.params)
            originals[f"{key}/{path.name}"] = base
            fill = 255 if args.fill == "white" else paper_level(gray)
            for theta in (ANGLES_AUGRAPHY if args.augraphy else ANGLES):
                if args.augraphy:
                    rot = augraphy_rotate(gray, theta)
                    if rot is None:
                        sys.exit("augraphy is not usable here (import failed); rerun without --augraphy")
                else:
                    rot = gray.rotate(theta, resample=Image.BICUBIC, expand=True, fillcolor=fill)
                est = estimate(rot, args.params)
                rows.append({
                    "set": key, "page": path.name, "theta": theta,
                    "est_original": base["skew"], "est_rotated": est["skew"],
                    "error": est["skew"] - base["skew"] - theta,
                    "ratio_original": base["ratio"], "ratio_rotated": est["ratio"],
                })
            print(f"{key}/{path.name}: original skew {base['skew']:+.2f}", file=sys.stderr)

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, sort_keys=True) + "\n")

    def show(label, subset):
        s = stats([r["error"] for r in subset])
        if not s:
            return
        pages_failed = len({(r["set"], r["page"]) for r in subset if abs(r["error"]) > GROSS_DEG})
        pages = len({(r["set"], r["page"]) for r in subset})
        print(f"{label:<22} {pages:>4} {s['n']:>5} {s['median_abs']:>8.3f} {s['p90_abs']:>8.3f} "
              f"{s['max_abs']:>8.2f} {s['median_signed']:>+8.3f} {s['over_tolerance']:>6} "
              f"{s['over_gross']:>6} {pages_failed:>9}")

    fill = "black (augraphy)" if args.augraphy else args.fill
    print(f"\nfill={fill} rotation={'augraphy' if args.augraphy else 'pillow-bicubic'} "
          f"angles={len(ANGLES_AUGRAPHY if args.augraphy else ANGLES)} tolerance={TOLERANCE_DEG} gross={GROSS_DEG}")
    print(f"{'set':<22} {'pages':>4} {'meas':>5} {'med|e|':>8} {'p90|e|':>8} {'max|e|':>8} "
          f"{'bias':>8} {'>0.3':>6} {'>1.0':>6} {'pg>1.0':>9}")
    for key in keys:
        show(key, [r for r in rows if r["set"] == key])
    if args.sets == "all":
        show("TUNE sets", [r for r in rows if r["set"] in TUNE])
        show("HELD-OUT sets", [r for r in rows if r["set"] in HELDOUT])
    show("OVERALL", rows)

    if args.show_worst:
        print("\nworst measurements:")
        for r in sorted(rows, key=lambda r: -abs(r["error"]))[: args.show_worst]:
            print(f"  {r['set']}/{r['page']} theta={r['theta']:+.1f} "
                  f"orig={r['est_original']:+.2f} rot={r['est_rotated']:+.2f} err={r['error']:+.2f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
