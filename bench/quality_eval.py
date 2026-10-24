#!/usr/bin/env python3
"""Run the quality block (module M3) over the DIBCO page images and score what can be scored.

    python -I bench/quality_eval.py --data bench/data --out bench/results/quality.jsonl
    python -I bench/quality_eval.py --data bench/data --sets tune

DIBCO is not Indian-deed material, and DIBCO carries no ground truth for degradation. The only
labels used here are the author's own, from looking at the page images, and are therefore
subjective: `STRONG_BLEED` lists pages where reverse-side text is plainly visible through the paper,
`CLEAN_BLEED` pages where none is. Pages in neither list are not scored. The labels were made before
the bleed-through thresholds were tuned and are kept in this file so the evaluation is repeatable.

Tuning split: parameters were tuned on the "tune" sets only (same split as bench/skew_eval.py), and
the "heldout" sets were scored once at the end.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import pagetriage_py
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from skew_eval import HELDOUT, SETS, TUNE, list_pages, to_gray  # noqa: E402

# Pages where the reverse side is plainly visible through the paper (author's eyeball).
STRONG_BLEED = {
    "dibco2009": ["H03"],
    "dibco2010": ["H05"],
    "dibco2011": ["HW7", "PR1", "PR2", "PR3", "PR4", "PR8"],
    "dibco2012": ["H01", "H06", "H14"],
    "dibco2013": ["HW04", "HW05", "HW06", "PR06"],
    "dibco2016": ["2", "3", "5", "6"],
    "dibco2017": ["1", "3", "11", "12", "13", "14", "15", "18", "19", "20"],
    "dibco2019_track_a": ["2", "3"],
}
# Pages with no visible show-through.
CLEAN_BLEED = {
    "dibco2009": ["P01", "P02", "P03", "P05"],
    "dibco2010": ["H04", "H06", "H09", "H10"],
    "dibco2011": ["HW2", "HW5", "HW8"],
    "dibco2012": ["H03", "H04", "H07", "H08", "H10", "H12", "H13"],
    "dibco2013": ["HW01", "HW02", "HW03", "HW07", "PR03", "PR04", "PR08"],
    "dibco2014": ["H01", "H02", "H03", "H04", "H05", "H06", "H08", "H10"],
    "dibco2016": ["1", "4", "8", "9", "10"],
    "dibco2017": ["6", "7", "8", "9", "10"],
    "dibco2018": ["2", "3", "4", "6", "7", "8", "9"],
    "dibco2019_track_a": ["1", "4", "7", "8", "9", "10"],
}


def label_of(key: str, name: str):
    stem = Path(name).stem
    if stem in STRONG_BLEED.get(key, []):
        return 1
    if stem in CLEAN_BLEED.get(key, []):
        return 0
    return None


def auc(scores_pos, scores_neg):
    """Probability that a random positive outscores a random negative (ties count half)."""
    if not scores_pos or not scores_neg:
        return float("nan")
    wins = sum((p > n) + 0.5 * (p == n) for p in scores_pos for n in scores_neg)
    return wins / (len(scores_pos) * len(scores_neg))


def analyse(gray: Image.Image, params: str | None):
    opts = {"detectors": [], "debug": True}
    if params:
        opts["params"] = params
    t = time.perf_counter()
    out = pagetriage_py.analyse_page(
        gray.tobytes(), gray.width, gray.height, 150, None, None, json.dumps(opts)
    )
    ms = (time.perf_counter() - t) * 1000
    return json.loads(out), ms


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--data", type=Path, default=Path("bench/data"))
    ap.add_argument("--sets", default="all", help="all, tune, heldout, or comma-separated set keys")
    ap.add_argument("--params", help="TOML override for the analyser")
    ap.add_argument("--out", type=Path, help="write one JSON line per page here")
    ap.add_argument("--table", action="store_true", help="print every page, not just the summary")
    args = ap.parse_args(argv)

    keys = {"all": list(SETS), "tune": TUNE, "heldout": HELDOUT}.get(args.sets) or args.sets.split(",")
    rows = []
    for key in keys:
        for path in list_pages(args.data, key):
            with Image.open(path) as img:
                img.load()
                gray = to_gray(img)
            report, ms = analyse(gray, args.params)
            q, f = report["quality"], report["debug"]["quality"]
            rows.append({
                "set": key, "page": path.name, "w": gray.width, "h": gray.height, "ms": ms,
                "label_bleed": label_of(key, path.name), "quality": q, "features": f,
            })

    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as fh:
            for r in rows:
                fh.write(json.dumps(r, sort_keys=True) + "\n")

    if args.table:
        print(f"{'page':<26} {'skew':>6} {'pk':>5} {'bleed':>6} {'ghost':>6} {'sig_mm':>6} {'rms':>5} "
              f"{'gap':>5} {'speck':>7} {'fold':>4} {'gen':>3} {'bin':>7} {'gr':>2} {'bad':>5} {'lab':>3}")
        for r in rows:
            q, f = r["quality"], r["features"]
            gen = q["photocopy_generation_est"]
            print(f"{r['set'][5:] + '/' + r['page']:<26} {q['skew_deg']:6.2f} {f['skew_peak_ratio']:5.2f} "
                  f"{q['bleed_through']:6.3f} {f['bleed_ghost_stroke_ratio']:6.3f} "
                  f"{f['blur_sigma_mm']:6.3f} {q['contrast_rms']:5.3f} {f['peak_gap']:5.2f} "
                  f"{q['speckle_density']:7.5f} {q['fold_lines']:4d} "
                  f"{(gen['value'] if gen else -1):3d} {q['recommended_binarizer']:>7} {q['grade']:>2} "
                  f"{f['grade_badness']:5.2f} {'' if r['label_bleed'] is None else r['label_bleed']:>3}")

    def split_auc(name, subset, field):
        pos = [field(r) for r in subset if r["label_bleed"] == 1]
        neg = [field(r) for r in subset if r["label_bleed"] == 0]
        print(f"  {name:<24} strong={len(pos):>2} clean={len(neg):>2} "
              f"AUC={auc(pos, neg):.3f}")

    print("\nbleed-through vs the author's labels (AUC: 1.0 perfect, 0.5 chance)")
    for name, field in [
        ("bleed_through", lambda r: r["quality"]["bleed_through"]),
        ("mid_mass", lambda r: r["features"]["bleed_mid_mass"]),
        ("ghost_stroke_ratio", lambda r: r["features"]["bleed_ghost_stroke_ratio"]),
        ("faint_ink_ratio", lambda r: r["features"]["bleed_faint_ink_ratio"]),
    ]:
        print(f" {name}")
        split_auc("tune sets", [r for r in rows if r["set"] in TUNE], field)
        split_auc("held-out sets", [r for r in rows if r["set"] in HELDOUT], field)
        split_auc("all", rows, field)

    times = sorted(r["ms"] for r in rows)
    if times:
        print(f"\nanalyse_page wall time over {len(times)} pages (includes the Python call): "
              f"median {times[len(times) // 2]:.1f} ms, max {times[-1]:.1f} ms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
