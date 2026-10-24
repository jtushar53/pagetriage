"""Score the binarisers on the DIBCO benchmark pages and compare with published numbers.

    python -I bench/eval_binarize.py --sets dibco2009 dibco2010 --method howe
    python -I bench/eval_binarize.py --sets dibco2011 --method howe --params-file my_overrides.toml

Per image it prints the F-measure and PSNR (definitions in `metrics.py`), and next to it the value
Howe published for the same image, when there is one. The images are the page images of the DIBCO
test sets as fetched by `fetch.py`; the layouts are in `bench/README.md`. Nothing here is an
algorithm: it only decodes, calls `pagetriage_py.binarize`, and scores.

Published numbers: N. R. Howe, "Document binarization with automatic parameter tuning", IJDAR
16(3):247-258, 2013 (author's manuscript), Tables 1 and 3. They were copied by hand from that text.
The 2009 training images (H2009-T1/T2, P2009-T1/T2) are not part of the downloaded test sets, so
the paper's 2009/2010 averages cover 24 images and ours cover 20.
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import metrics  # noqa: E402

DATA = Path(__file__).resolve().parent / "data"

# (key, image, ground truth) per image, under bench/data. Keys follow the paper's naming.
def _manifest() -> dict[str, list[tuple[str, Path, Path]]]:
    d09 = DATA / "dibco2009"
    h09 = d09 / "DIBC02009_Test_images-handwritten" / "DIBC02009_Test_images-handwritten"
    p09 = d09 / "DIBCO2009_Test_images-printed" / "DIBCO2009_Test_images-printed"
    g09h = d09 / "DIBCO2009-GT-Test-images_handwritten" / "DIBCO2009-GT-Test-images_handwritten"
    g09p = d09 / "DIBCO2009-GT-Test-images_printed" / "DIBCO2009-GT-Test-images_printed"
    d10 = DATA / "dibco2010"
    d11 = DATA / "dibco2011"
    out: dict[str, list[tuple[str, Path, Path]]] = {"dibco2009": [], "dibco2010": [], "dibco2011": []}
    for n in range(1, 6):
        out["dibco2009"].append((f"H2009-{n}", h09 / f"H0{n}.bmp", g09h / f"H0{n}.tiff"))
    for n in range(1, 6):
        out["dibco2009"].append((f"P2009-{n}", p09 / f"P0{n}.bmp", g09p / f"P0{n}.tiff"))
    for n in range(1, 11):
        stem = f"H{n:02d}"
        image = next((d10 / "H_DIBCO2010_test_images").glob(stem + ".*"), d10 / f"{stem}.missing")
        out["dibco2010"].append((f"H2010-{n}", image, d10 / "H_DIBCO2010_GT" / f"{stem}_estGT.tiff"))
    for n in range(1, 9):
        out["dibco2011"].append(
            (f"H2011-{n}", d11 / "DIBCO11-handwritten" / f"HW{n}.png", d11 / "DIBCO11-handwritten" / f"HW{n}_GT.tiff")
        )
    for n in range(1, 9):
        out["dibco2011"].append(
            (f"P2011-{n}", d11 / "DIBCO11-machine_printed" / f"PR{n}.png", d11 / "DIBCO11-machine_printed" / f"PR{n}_GT.tiff")
        )
    return out


# Howe 2013, Table 1 (DIBCO 2009 + H-DIBCO 2010): F-measure per image for
# (static, oracle, algorithm 1 = tune c, algorithm 2 = tune c and t_hi, algorithm 3 = dual).
TABLE1 = {
    "H2009-1": (92.7, 95.9, 95.4, 95.8, 95.8), "H2009-2": (90.1, 96.4, 88.8, 96.2, 95.7),
    "H2009-3": (94.7, 95.6, 92.0, 94.8, 95.1), "H2009-4": (92.1, 94.7, 93.9, 94.6, 94.5),
    "H2009-5": (86.5, 92.7, 91.9, 92.3, 92.6),
    "H2010-1": (93.7, 96.2, 95.0, 95.4, 96.0), "H2010-2": (89.0, 96.1, 95.4, 95.7, 95.4),
    "H2010-3": (94.4, 94.7, 93.3, 94.7, 94.7), "H2010-4": (92.9, 94.4, 93.5, 93.8, 93.9),
    "H2010-5": (92.9, 96.5, 96.1, 96.5, 96.4), "H2010-6": (90.9, 91.2, 90.4, 90.9, 90.9),
    "H2010-7": (95.0, 95.2, 94.8, 95.0, 95.1), "H2010-8": (92.6, 93.7, 92.2, 93.5, 93.4),
    "H2010-9": (92.6, 93.8, 92.9, 92.1, 93.5), "H2010-10": (88.8, 92.7, 90.8, 92.5, 87.2),
    "P2009-1": (93.9, 94.3, 90.4, 94.0, 94.2), "P2009-2": (96.8, 96.9, 96.8, 96.8, 96.9),
    "P2009-3": (97.6, 98.4, 98.2, 98.2, 98.3), "P2009-4": (93.5, 93.8, 92.8, 92.9, 92.8),
    "P2009-5": (88.6, 91.5, 84.7, 91.2, 84.3),
}
# Howe 2013, Table 3 (DIBCO 2011): (static, oracle, tune c, tune t_hi, dual).
TABLE3 = {
    "H2011-1": (77.3, 93.9, 88.2, 90.2, 90.0), "H2011-2": (97.4, 97.5, 96.6, 97.3, 97.4),
    "H2011-3": (93.2, 95.2, 94.9, 94.5, 93.5), "H2011-4": (92.5, 92.7, 92.5, 92.0, 92.0),
    "H2011-5": (92.4, 97.1, 96.2, 97.1, 96.2), "H2011-6": (88.2, 94.4, 91.1, 60.6, 92.1),
    "H2011-7": (87.6, 92.5, 92.3, 89.6, 89.8), "H2011-8": (95.3, 95.7, 94.9, 95.3, 95.4),
    "P2011-1": (93.0, 95.3, 94.3, 95.1, 94.7), "P2011-2": (77.6, 90.8, 73.5, 71.5, 72.3),
    "P2011-3": (95.6, 96.6, 96.3, 96.4, 96.4), "P2011-4": (95.0, 95.2, 94.5, 95.2, 95.1),
    "P2011-5": (94.8, 94.9, 93.7, 94.2, 94.6), "P2011-6": (66.7, 93.1, 89.9, 86.0, 86.3),
    "P2011-7": (90.2, 93.3, 89.7, 93.1, 93.1), "P2011-8": (90.3, 92.2, 88.5, 91.3, 91.5),
}
# Column of the tables above that corresponds to each of our Howe modes.
PUBLISHED_COLUMN = {"static": 0, "tune_c": 2, "dual": 4}


def load_gray(path: Path) -> np.ndarray:
    import pagetriage_py as pt

    with Image.open(path) as im:
        rgb = im.convert("RGB")
        width, height = rgb.size
        gray = pt.rgb_to_gray(rgb.tobytes())
    return np.frombuffer(gray, dtype=np.uint8).reshape(height, width)


def load_ink(path: Path) -> np.ndarray:
    with Image.open(path) as im:
        return metrics.ink_mask(np.asarray(im.convert("L")))


def published(key: str, mode: str, method: str) -> float | None:
    if method != "howe":
        return None
    table = TABLE3 if "2011" in key else TABLE1
    row = table.get(key)
    if row is None:
        return None
    return row[PUBLISHED_COLUMN[mode]]


def with_howe_mode(overrides: str, mode: str) -> str:
    """TOML overrides that certainly run Howe in `mode`. A `--params` string that already has a
    `[howe]` table must not silently win over `--howe-mode` (it used to: the flag was dropped and the
    embedded default mode ran instead), so the mode is written into that table, and a conflicting
    `mode` key is an error."""
    import re

    given = re.search(r'^\s*mode\s*=\s*"([a-z_]+)"', overrides, flags=re.M)
    if given:
        if given.group(1) != mode:
            raise SystemExit(f'--params sets mode = "{given.group(1)}" but --howe-mode is "{mode}"')
        return overrides
    if re.search(r"^\s*\[howe\]\s*$", overrides, flags=re.M):
        return re.sub(r"^(\s*\[howe\]\s*)$", rf'\1\nmode = "{mode}"', overrides, count=1, flags=re.M)
    return overrides + f'\n[howe]\nmode = "{mode}"\n'


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sets", nargs="+", default=["dibco2009", "dibco2010", "dibco2011"], choices=["dibco2009", "dibco2010", "dibco2011"])
    ap.add_argument("--method", default="howe", choices=["otsu", "sauvola", "wolf", "howe"])
    ap.add_argument("--howe-mode", default="dual", choices=["static", "tune_c", "dual"])
    ap.add_argument("--params", default=None, help="TOML override text, e.g. '[howe]\\ncost_scale = 1.0'")
    ap.add_argument("--params-file", default=None, help="file with TOML overrides")
    ap.add_argument("--only", nargs="*", default=None, help="image keys, e.g. H2011-1 P2011-6")
    ap.add_argument("--json", default=None, help="write per-image results here")
    args = ap.parse_args()

    import pagetriage_py as pt

    overrides = args.params or ""
    if args.params_file:
        overrides += "\n" + Path(args.params_file).read_text()
    if args.method == "howe":
        overrides = with_howe_mode(overrides, args.howe_mode)
    manifest = _manifest()
    rows = []
    for name in args.sets:
        for key, image_path, gt_path in manifest[name]:
            if args.only and key not in args.only:
                continue
            if not image_path.exists() or not gt_path.exists():
                print(f"missing data for {key}: run bench/fetch.py", file=sys.stderr)
                return 2
            gray = load_gray(image_path)
            truth = load_ink(gt_path)
            height, width = gray.shape
            if truth.shape != gray.shape:
                print(f"{key}: GT shape {truth.shape} != image shape {gray.shape}", file=sys.stderr)
                return 2
            started = time.perf_counter()
            extra = {}
            if args.method == "howe":
                ink, c, t_hi = pt.binarize_howe(gray.tobytes(), width, height, overrides or None)
                extra = {"c": round(c, 1), "t_hi": round(t_hi, 3)}
            else:
                ink = pt.binarize(gray.tobytes(), width, height, args.method, overrides or None)
            seconds = time.perf_counter() - started
            pred = np.frombuffer(ink, dtype=np.uint8).reshape(height, width).astype(bool)
            row = {
                "key": key, "set": name, "pixels": width * height,
                "f": metrics.f_measure(pred, truth), "psnr": metrics.psnr(pred, truth),
                "seconds": seconds, "published": published(key, args.howe_mode, args.method), **extra,
            }
            rows.append(row)
            pub = f"{row['published']:6.1f}" if row["published"] is not None else "     -"
            print(f"{key:9s} F={row['f']:6.2f}  published={pub}  PSNR={row['psnr']:5.2f}  {seconds:6.1f}s  {extra}", flush=True)

    def mean(values):
        return statistics.fmean(values) if values else float("nan")

    print()
    for label, pick in (("handwritten", lambda r: r["key"].startswith("H")), ("printed", lambda r: r["key"].startswith("P")), ("all", lambda r: True)):
        chosen = [r for r in rows if pick(r)]
        if not chosen:
            continue
        pubs = [r["published"] for r in chosen if r["published"] is not None]
        paired = [r for r in chosen if r["published"] is not None]
        print(
            f"{label:12s} n={len(chosen):2d}  mean F={mean([r['f'] for r in chosen]):6.2f}"
            + (f"  published mean over the same images={mean(pubs):6.2f}  (ours - published = {mean([r['f'] for r in paired]) - mean(pubs):+.2f})" if pubs else "")
        )
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(rows, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
