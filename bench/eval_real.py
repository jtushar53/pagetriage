#!/usr/bin/env python3
"""Score the detectors on a folder of real, labelled pages, and write the calibration CSV.

    python -I bench/eval_real.py PAGES_DIR [--detectors a,b,...] [--calibration-csv out.csv]
    python -I bench/eval_real.py --selftest

PAGES_DIR holds page images (PNG, PGM, TIFF, JPEG) and a `labels.json`:

    {"scan_001.png": {"dpi": 150,
                      "page_type": "form",
                      "findings": [{"kind": "stamp", "bbox": [x0, y0, x1, y1]},
                                   {"kind": "thumb_impression", "bbox": [...]},
                                   {"kind": "blank_page"}]}}

`bbox` is in pixels of the image as stored; a label without a `bbox` is a page-level label (the page
has such a finding). A finding of the same kind counts as right when its bbox overlaps a label's with
IoU >= --iou (0.5), or, for a page-level label, when there is one. Reported per kind: labels, findings,
true positives, false positives, false negatives, precision and recall; and the page-type accuracy. With
--calibration-csv, every finding is written as `kind,confidence,label` (label 1 when it matched), which
`bench/fit_calibration.py` turns into calibration maps.

Real pages are private. Never commit them: they contain names, Aadhaar fragments and addresses. Put them
under bench/data/ (git-ignored) or anywhere outside the repository. `--selftest` builds a tiny labelled
set from the synthetic generators in a temporary folder and checks this script on it; it says nothing
about real pages. Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image

IMAGE_SUFFIXES = {".png", ".pgm", ".tif", ".tiff", ".jpg", ".jpeg", ".bmp"}


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]) + 1)
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]) + 1)
    inter = ix * iy
    union = (a[2] - a[0] + 1) * (a[3] - a[1] + 1) + (b[2] - b[0] + 1) * (b[3] - b[1] + 1) - inter
    return inter / union if union else 0.0


def analyse(path, dpi, detectors):
    img = Image.open(path)
    rgb = None
    if img.mode not in ("L", "LA"):
        rgb_img = img.convert("RGB")
        rgb = np.asarray(rgb_img).tobytes()
        gray = pagetriage_py.rgb_to_gray(rgb)
    else:
        gray = np.asarray(img.convert("L")).tobytes()
    opts = {"detectors": detectors}
    out = pagetriage_py.analyse_page(gray, img.width, img.height, dpi, rgb, None, json.dumps(opts))
    return json.loads(out)


def score(pages_dir, detectors, min_iou, rows_out=None):
    labels = json.loads((pages_dir / "labels.json").read_text(encoding="utf-8"))
    stats = defaultdict(lambda: dict(labels=0, findings=0, tp=0, fp=0))
    type_total = type_right = 0
    for name, entry in sorted(labels.items()):
        path = pages_dir / name
        if not path.exists() or path.suffix.lower() not in IMAGE_SUFFIXES:
            print(f"skipped {name}: not an image in {pages_dir}", file=sys.stderr)
            continue
        report = analyse(path, int(entry.get("dpi", 150)), detectors)
        truth = entry.get("findings", [])
        for t in truth:
            stats[t["kind"]]["labels"] += 1
        used = set()
        for f in sorted(report["findings"], key=lambda f: -f["confidence"]):
            k = f["kind"]
            stats[k]["findings"] += 1
            hit = None
            for i, t in enumerate(truth):
                if i in used or t["kind"] != k:
                    continue
                if "bbox" not in t or (f.get("bbox") and iou(f["bbox"], t["bbox"]) >= min_iou):
                    hit = i
                    break
            if hit is None:
                stats[k]["fp"] += 1
            else:
                used.add(hit)
                stats[k]["tp"] += 1
            if rows_out is not None:
                rows_out.append((k, f["confidence"], int(hit is not None)))
        if "page_type" in entry and report.get("page_type"):
            type_total += 1
            type_right += report["page_type"]["label"] == entry["page_type"]
    return stats, (type_right, type_total)


def print_table(stats, page_type):
    print(f"{'kind':<22}{'labels':>7}{'found':>7}{'TP':>5}{'FP':>5}{'FN':>5}{'precision':>10}{'recall':>8}")
    for kind in sorted(stats):
        s = stats[kind]
        fn = max(0, s["labels"] - s["tp"])
        prec = s["tp"] / s["findings"] if s["findings"] else float("nan")
        rec = s["tp"] / s["labels"] if s["labels"] else float("nan")
        print(f"{kind:<22}{s['labels']:>7}{s['findings']:>7}{s['tp']:>5}{s['fp']:>5}{fn:>5}{prec:>10.2f}{rec:>8.2f}")
    if page_type[1]:
        print(f"\npage type: {page_type[0]} / {page_type[1]} correct")


def selftest():
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import qr_synth_eval as q

    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp)
        labels = {}
        for seed in (1, 2, 3):
            img, truth = q.page("real qr", seed)
            name = f"qr_{seed}.png"
            img.save(d / name)
            labels[name] = {"dpi": 150, "findings": [{"kind": "qr_code", "bbox": list(t)} for t in truth]}
        img, _ = q.page("plain page", 1)
        img.save(d / "plain.png")
        labels["plain.png"] = {"dpi": 150, "findings": []}
        (d / "labels.json").write_text(json.dumps(labels), encoding="utf-8")
        rows = []
        stats, pt = score(d, ["qr"], 0.5, rows)
        print_table(stats, pt)
        s = stats["qr_code"]
        assert (s["labels"], s["tp"], s["fp"]) == (3, 3, 0), s
        assert len(rows) == 3 and all(r[2] == 1 for r in rows)
        csv_path = d / "cal.csv"
        with open(csv_path, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["kind", "confidence", "label"])
            w.writerows(rows)
        assert csv_path.read_text().splitlines()[0] == "kind,confidence,label"
    print("selftest ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("pages", nargs="?")
    ap.add_argument("--detectors", default=",".join(d for d in pagetriage_py_detectors()))
    ap.add_argument("--iou", type=float, default=0.5)
    ap.add_argument("--calibration-csv")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return
    if not a.pages:
        ap.error("a folder of labelled pages or --selftest")
    rows = [] if a.calibration_csv else None
    stats, pt = score(Path(a.pages), a.detectors.split(","), a.iou, rows)
    print_table(stats, pt)
    if rows is not None:
        with open(a.calibration_csv, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["kind", "confidence", "label"])
            w.writerows(rows)
        print(f"wrote {len(rows)} rows to {a.calibration_csv}")


def pagetriage_py_detectors():
    # Every opt-in detector that reads a single gray page; stamp needs colour and is skipped for gray
    # images by the core itself (it returns nothing), so it is safe to list.
    return ["blank_page", "coverage", "emboss", "handwriting", "layout", "photo", "qr", "redaction", "rulings",
            "script", "signature", "stamp", "thumb"]


if __name__ == "__main__":
    main()
