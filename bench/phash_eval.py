#!/usr/bin/env python3
"""Evaluate the perceptual hashes (module M12) on the DIBCO page images.

    python -I bench/phash_eval.py --data bench/data --out bench/results/phash_eval.json

Needs the Python binding (`maturin develop --release -m crates/pagetriage-py/Cargo.toml`),
numpy and Pillow, and the DIBCO sets from `bench/fetch.py`. All hashing happens in the Rust core
through `pagetriage_py`; this script only builds perturbed copies of each page and counts bits.

What it measures, for the 136 DIBCO page images (ground-truth images excluded):

  (a) Hamming distance between a page and each perturbed copy of it.
  (b) Hamming distance between distinct pages.
  (c) at the default `max_hamming`: recall on the perturbed copies, and the false-positive rate
      among distinct pairs (a perturbed copy of page i against the original of page j != i).

No threshold is fitted to the DIBCO pages. `max_hamming = 10` comes from the project plan, and
`flat_rms` is a stability guard. The one fitted value, the dHash bound `dhash_max_hamming`, was
chosen on the synthetic "tune" sets (below) by a rule written down in params/phash.toml; the
"test" sets were printed beside them in the same run, so they are not blind, and the "check" sets
were added afterwards with a fresh seed and are the ones that were unseen when the value was
chosen. The DIBCO pages are split by edition into a tuning half and a held-out half only so a
later retune has an untouched half to report on; both halves are shown.

Perturbations 1 to 3 px shifts, JPEG q60 and q85, noise, 0.5 degree rotation, +-10% brightness and
a 90% area crop are the specified set; the rows marked "extra" go beyond it (shifts of 0.5%, 1% and
3% of the page, a 97% crop, half resolution, 2 and 180 degree rotations, a combined "rescan" copy),
because a few pixels are tiny against a 1500 px page.

Everything is seeded, so a rerun with the same numpy and Pillow gives the same numbers. JPEG
output depends on the libjpeg build behind Pillow, so the last digit of a JPEG row can differ
between machines.

A synthetic arm (`--synthetic N`) renders pages of typed text with a shared layout, to ask whether
distinct pages that look alike hash alike; those are made-up pages, not scans.
"""
from __future__ import annotations

import argparse
import io
import json
import math
import sys
import time
import zlib
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageDraw, ImageFont

Image.MAX_IMAGE_PIXELS = 400_000_000

# Directories under bench/data and the glob that picks the page images (not the ground truth);
# see bench/README.md.
SETS = {
    "dibco2009": [
        ("dibco2009/DIBC02009_Test_images-handwritten/DIBC02009_Test_images-handwritten", "*"),
        ("dibco2009/DIBCO2009_Test_images-printed/DIBCO2009_Test_images-printed", "*"),
    ],
    "dibco2010": [("dibco2010/H_DIBCO2010_test_images", "*")],
    "dibco2011": [
        ("dibco2011/DIBCO11-handwritten", "*.png"),
        ("dibco2011/DIBCO11-machine_printed", "*.png"),
    ],
    "dibco2012": [("dibco2012/H-DIBCO2012-dataset/H-DIBCO2012-dataset/H-DIBCO2012-dataset", "*.png")],
    "dibco2013": [("dibco2013/DIBCO2013-dataset/OriginalImages", "*")],
    "dibco2014": [("dibco2014/original_images", "*")],
    "dibco2016": [("dibco2016/DIBCO2016_dataset-original/DIPCO2016_dataset", "*")],
    "dibco2017": [("dibco2017/DIBCO2017_Dataset/Dataset", "*")],
    "dibco2018": [("dibco2018/dibco2018_Dataset/dataset", "*")],
    "dibco2019_track_a": [("dibco2019_track_a/dibco2019_dataset_trackA/Dataset", "*")],
    "dibco2019_track_b": [("dibco2019_track_b/dibco2019_dataset_trackB/Dataset", "*")],
}
IMAGE_SUFFIXES = {".bmp", ".png", ".tif", ".tiff", ".jpg", ".jpeg"}

# Whole editions are held out so the tuning split and the reporting split never share a source
# collection. Chosen by edition parity before looking at any result.
TUNE_SETS = {"dibco2009", "dibco2011", "dibco2013", "dibco2017", "dibco2019_track_a"}

OPTS = '{"detectors":["phash"]}'
OPTS_DEBUG = '{"detectors":["phash"],"debug":true}'
DPI = 150
THRESHOLDS = (4, 6, 8, 10, 12, 16, 20)
DHASH_GATES = (10, 14, 18, 22, 26, 64)


# ---------------------------------------------------------------- pages


def load_gray(path: Path) -> np.ndarray:
    """8-bit gray page, converted exactly as bench/run.py does (the core's own luma for colour)."""
    with Image.open(path) as img:
        img.load()
        width, height = img.size
        if img.mode == "L":
            return np.frombuffer(img.tobytes(), dtype=np.uint8).reshape(height, width)
        rgb = img.convert("RGB").tobytes()
    gray = pagetriage_py.rgb_to_gray(rgb)
    return np.frombuffer(gray, dtype=np.uint8).reshape(height, width)


def discover(data: Path):
    pages = []
    for key, dirs in SETS.items():
        for rel, pattern in dirs:
            root = data / rel
            if not root.is_dir():
                sys.exit(f"missing {root}; fetch the DIBCO sets with bench/fetch.py")
            for path in sorted(root.rglob(pattern)):
                if path.is_file() and path.suffix.lower() in IMAGE_SUFFIXES:
                    pages.append((key, path))
    return pages


# ---------------------------------------------------------------- perturbations


def _shift(g, dx, dy):
    """Content moved right by dx and down by dy; the exposed edge repeats the border pixels."""
    h, w = g.shape
    return np.pad(g, ((dy, 0), (dx, 0)), mode="edge")[:h, :w]


def _jpeg(g, quality):
    buf = io.BytesIO()
    Image.fromarray(g, "L").save(buf, format="JPEG", quality=quality)
    buf.seek(0)
    with Image.open(buf) as im:
        return np.asarray(im.convert("L"), dtype=np.uint8)


def _noise(g, rng, sigma):
    out = g.astype(np.float32) + rng.normal(0.0, sigma, g.shape).astype(np.float32)
    return np.clip(np.rint(out), 0, 255).astype(np.uint8)


def _rotate(g, degrees):
    """Rotate about the centre; the border is padded by repeating edge pixels so no corner is
    filled with an unrelated colour."""
    h, w = g.shape
    margin = int(math.ceil(0.01 * max(h, w))) + 2
    padded = np.pad(g, margin, mode="edge")
    rotated = Image.fromarray(padded, "L").rotate(degrees, resample=Image.BICUBIC)
    return np.asarray(rotated, dtype=np.uint8)[margin : margin + h, margin : margin + w]


def _gain(g, factor):
    return np.clip(np.rint(g.astype(np.float32) * factor), 0, 255).astype(np.uint8)


def _crop_area(g, fraction):
    h, w = g.shape
    side = math.sqrt(fraction)
    ch, cw = max(1, round(h * side)), max(1, round(w * side))
    y0, x0 = (h - ch) // 2, (w - cw) // 2
    return np.ascontiguousarray(g[y0 : y0 + ch, x0 : x0 + cw])


def _shift_fraction(g, fraction):
    h, w = g.shape
    return _shift(g, max(1, round(w * fraction)), max(1, round(h * fraction)))


def _downscale(g, factor):
    h, w = g.shape
    im = Image.fromarray(g, "L").resize((max(1, round(w * factor)), max(1, round(h * factor))), Image.LANCZOS)
    return np.asarray(im, dtype=np.uint8)


def _rescan(g, rng):
    """One composite 'rescan': half a percent of shift, 0.3 degrees, 5% dimmer, noise, JPEG q85."""
    out = _shift_fraction(g, 0.005)
    out = _rotate(out, 0.3)
    out = _gain(out, 0.95)
    out = _noise(out, rng, 6.0)
    return _jpeg(out, 85)


# name -> (function(gray, rng), in the specified set?)
PERTURBATIONS = {
    "shift_1px": (lambda g, r: _shift(g, 1, 1), True),
    "shift_2px": (lambda g, r: _shift(g, 2, 2), True),
    "shift_3px": (lambda g, r: _shift(g, 3, 3), True),
    "jpeg_q60": (lambda g, r: _jpeg(g, 60), True),
    "jpeg_q85": (lambda g, r: _jpeg(g, 85), True),
    "noise_sigma10": (lambda g, r: _noise(g, r, 10.0), True),
    "rotate_0.5deg": (lambda g, r: _rotate(g, 0.5), True),
    "brightness_-10%": (lambda g, r: _gain(g, 0.9), True),
    "brightness_+10%": (lambda g, r: _gain(g, 1.1), True),
    "crop_90%_area": (lambda g, r: _crop_area(g, 0.9), True),
    "shift_0.5%_extra": (lambda g, r: _shift_fraction(g, 0.005), False),
    "shift_1%_extra": (lambda g, r: _shift_fraction(g, 0.01), False),
    "shift_3%_extra": (lambda g, r: _shift_fraction(g, 0.03), False),
    "crop_97%_area_extra": (lambda g, r: _crop_area(g, 0.97), False),
    "rotate_2deg_extra": (lambda g, r: _rotate(g, 2.0), False),
    "rotate_180deg_extra": (lambda g, r: np.ascontiguousarray(g[::-1, ::-1]), False),
    "downscale_50%_extra": (lambda g, r: _downscale(g, 0.5), False),
    "rescan_combo_extra": (_rescan, False),
}


# ---------------------------------------------------------------- hashing and distances


def hash_page(gray: np.ndarray, debug=False):
    h, w = gray.shape
    report = json.loads(
        pagetriage_py.analyse_page(gray.tobytes(), w, h, DPI, None, None, OPTS_DEBUG if debug else OPTS)
    )
    return report


def to_u64(hex_string: str) -> np.uint64:
    return np.uint64(int(hex_string, 16))


def popcount_matrix(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """Hamming distances between every element of `a` and every element of `b` (uint64 arrays)."""
    x = (a[:, None] ^ b[None, :]).astype(np.uint64)
    bits = np.unpackbits(x.view(np.uint8).reshape(x.shape + (8,)), axis=-1)
    return bits.sum(axis=-1).astype(np.int32)


def percentile(values, q):
    return float(np.percentile(np.asarray(values, dtype=np.float64), q))


# ---------------------------------------------------------------- synthetic same-template pages

WORDS = (
    "the said party hereinafter referred to as vendor purchaser agrees to sell and convey all that "
    "piece and parcel of land situate lying at village survey number measuring approximately square "
    "feet together with all rights easements appurtenances thereto free from encumbrances whatsoever "
    "consideration sum rupees paid receipt whereof is hereby acknowledged witness executed on this day "
    "of year between registered office deed schedule boundaries north south east west property "
    "possession title mortgage lease agreement clause covenant indemnify"
).split()


def _font(size):
    try:
        return ImageFont.load_default(size=size)
    except TypeError:  # Pillow without FreeType size support
        return ImageFont.load_default()


def render_text_page(rng: np.random.Generator, variant: str, width=827, height=1169) -> np.ndarray:
    """A made-up typed page at 100 dpi A4. 'dense': every page has the same margins and fills all
    lines, ragged right. 'varied': random heading, paragraph breaks, line count and signature block."""
    img = Image.new("L", (width, height), 245)
    draw = ImageDraw.Draw(img)
    font = _font(17)
    left, right, top, bottom, step = 95, width - 95, 110, height - 110, 27
    max_lines = (bottom - top) // step

    def line(target_fraction):
        words, text = [], ""
        while True:
            word = WORDS[int(rng.integers(len(WORDS)))]
            trial = (text + " " + word).strip()
            if font.getlength(trial) > (right - left) * target_fraction and text:
                return text
            text = trial

    rows = []  # (text, centered)
    if variant == "dense":
        rows = [(line(1.0), False) for _ in range(max_lines)]
    else:
        if rng.random() < 0.5:
            rows.append((" ".join(WORDS[int(rng.integers(len(WORDS)))].upper() for _ in range(3)), True))
            rows.append(("", False))
        target = int(rng.integers(14, max_lines + 1))
        while len(rows) < target:
            for _ in range(int(rng.integers(3, 10))):
                rows.append((line(1.0), False))
            rows.append((line(float(rng.uniform(0.15, 0.9))), False))
            rows.append(("", False))
        rows = rows[:target]
    for i, (text, centered) in enumerate(rows):
        x = left + ((right - left) - font.getlength(text)) / 2 if centered else left
        draw.text((x, top + i * step), text, fill=25, font=font)
    if variant == "varied" and rng.random() < 0.7:
        y = int(rng.integers(bottom - 120, bottom))
        x0 = int(rng.integers(left, left + 200))
        draw.line([(x0, y), (x0 + 180, y)], fill=30, width=2)
        draw.text((x0, y + 6), "Signature", fill=25, font=font)
    arr = np.asarray(img, dtype=np.float32) + rng.normal(0.0, 3.0, (height, width)).astype(np.float32)
    return np.clip(np.rint(arr), 0, 255).astype(np.uint8)


# ---------------------------------------------------------------- evaluation


def pair_stats(distances: np.ndarray) -> dict:
    d = distances.astype(np.float64)
    return {
        "n": int(d.size),
        "min": float(d.min()),
        "p1": percentile(d, 1),
        "p5": percentile(d, 5),
        "p25": percentile(d, 25),
        "median": percentile(d, 50),
        "p75": percentile(d, 75),
        "p99": percentile(d, 99),
        "max": float(d.max()),
        "within": {str(t): int((d <= t).sum()) for t in THRESHOLDS},
    }


def evaluate(names, split, ph0, dh0, ph_pert, dh_pert, max_hamming):
    """Metrics for the pages selected by the boolean array `split`."""
    idx = np.flatnonzero(split)
    out = {"pages": int(idx.size)}

    # (b) distinct originals
    full = popcount_matrix(ph0[idx], ph0[idx])
    iu = np.triu_indices(idx.size, k=1)
    distinct = full[iu]
    out["distinct"] = pair_stats(distinct)
    close = [
        (names[idx[i]], names[idx[j]], int(full[i, j]))
        for i, j in zip(*iu)
        if full[i, j] <= max_hamming
    ]
    out["distinct_pairs_within_max_hamming"] = close
    order = sorted(zip(full[iu].tolist(), iu[0].tolist(), iu[1].tolist()))[:6]
    out["closest_distinct_pairs"] = [(names[idx[i]], names[idx[j]], int(d)) for d, i, j in order]

    out["perturbations"] = {}
    for name in ph_pert:
        own = np.array(
            [popcount_matrix(ph0[i : i + 1], ph_pert[name][i : i + 1])[0, 0] for i in idx], dtype=np.int32
        )
        cross = popcount_matrix(ph_pert[name][idx], ph0[idx])
        off_diag = cross[~np.eye(idx.size, dtype=bool)]
        own_d = np.array(
            [popcount_matrix(dh0[i : i + 1], dh_pert[name][i : i + 1])[0, 0] for i in idx], dtype=np.int32
        )
        cross_d = popcount_matrix(dh_pert[name][idx], dh0[idx])
        off_d = cross_d[~np.eye(idx.size, dtype=bool)]
        row = {
            "misses_at_max": [names[i] for i, d in zip(idx, own) if d > max_hamming],
            "worst": sorted(((names[i], int(d)) for i, d in zip(idx, own)), key=lambda t: -t[1])[:4],
            "phash_own": {
                "median": percentile(own, 50),
                "mean": float(own.mean()),
                "p95": percentile(own, 95),
                "max": int(own.max()),
            },
            "dhash_own": {"median": percentile(own_d, 50), "p95": percentile(own_d, 95), "max": int(own_d.max())},
            "recall": {str(t): float((own <= t).mean()) for t in THRESHOLDS},
            "false_positive": {str(t): float((off_diag <= t).mean()) for t in THRESHOLDS},
            "false_positive_count_at_max": int((off_diag <= max_hamming).sum()),
            "cross_pairs": int(off_diag.size),
            "dhash_recall": {str(t): float((own_d <= t).mean()) for t in THRESHOLDS},
            "dhash_false_positive": {str(t): float((off_d <= t).mean()) for t in THRESHOLDS},
            # gate: pHash <= max_hamming AND dHash <= g
            "gate": {
                str(g): {
                    "recall": float(((own <= max_hamming) & (own_d <= g)).mean()),
                    "false_positive": float(((off_diag <= max_hamming) & (off_d <= g)).mean()),
                }
                for g in DHASH_GATES
            },
        }
        out["perturbations"][name] = row
    return out


def document_check(ph0_hex, dh0_hex, pert_hex, params_toml=None):
    """End to end through the Rust `near_duplicates`: a document of all originals followed by one
    perturbed copy per page. A copy is right when it points at its own original."""
    n = len(ph0_hex)
    result = {}
    for name, (ph, dh) in pert_hex.items():
        found = pagetriage_py.near_duplicates(ph0_hex + ph, dh0_hex + dh, params_toml)
        copies = found[n:]
        right = sum(1 for k, f in enumerate(copies) if f == k)
        missed = sum(1 for f in copies if f is None)
        wrong = n - right - missed
        result[name] = {"right": right, "missed": missed, "wrong_target": wrong}
    originals_flagged = sum(1 for f in found[:n] if f is not None)
    return result, originals_flagged


SYNTHETIC_COPIES = ("rescan_combo_extra", "shift_1%_extra", "shift_3px", "crop_90%_area")


def synthetic_arm(n, variant, seed, max_hamming):
    """Hash `n` made-up typed pages and a few perturbed copies of each; distinct-pair distances,
    recall of the copies, and the effect of the optional dHash gate."""
    rng = np.random.Generator(np.random.PCG64(seed))
    pages = [render_text_page(rng, variant) for _ in range(n)]
    ph0 = np.array([to_u64(hash_page(p)["phash"]) for p in pages], dtype=np.uint64)
    dh0 = np.array([to_u64(hash_page(p)["dhash"]) for p in pages], dtype=np.uint64)
    full = popcount_matrix(ph0, ph0)
    full_d = popcount_matrix(dh0, dh0)
    iu = np.triu_indices(n, k=1)
    out = {"pages": n, "distinct": pair_stats(full[iu]), "copies": {}}
    out["gate_distinct"] = {
        str(g): int(((full[iu] <= max_hamming) & (full_d[iu] <= g)).sum()) for g in DHASH_GATES
    }
    for name in SYNTHETIC_COPIES:
        fn = PERTURBATIONS[name][0]
        ph, dh = [], []
        for i, page in enumerate(pages):
            rep = hash_page(np.ascontiguousarray(fn(page, np.random.Generator(np.random.PCG64(seed + i)))))
            ph.append(to_u64(rep["phash"]))
            dh.append(to_u64(rep["dhash"]))
        ph, dh = np.array(ph, dtype=np.uint64), np.array(dh, dtype=np.uint64)
        own = np.array([popcount_matrix(ph0[i : i + 1], ph[i : i + 1])[0, 0] for i in range(n)])
        own_d = np.array([popcount_matrix(dh0[i : i + 1], dh[i : i + 1])[0, 0] for i in range(n)])
        cross = popcount_matrix(ph, ph0)
        cross_d = popcount_matrix(dh, dh0)
        off = ~np.eye(n, dtype=bool)
        out["copies"][name] = {
            "own_median": percentile(own, 50),
            "own_p95": percentile(own, 95),
            "recall": {str(t): float((own <= t).mean()) for t in THRESHOLDS},
            "false_positive": {str(t): float((cross[off] <= t).mean()) for t in THRESHOLDS},
            "gate": {
                str(g): {
                    "recall": float(((own <= max_hamming) & (own_d <= g)).mean()),
                    "false_positive": float(((cross[off] <= max_hamming) & (cross_d[off] <= g)).mean()),
                }
                for g in DHASH_GATES
            },
        }
    return out


def print_synthetic(synthetic, max_hamming):
    print("\n### Synthetic typed pages (made-up A4 text at 100 dpi; not scans)\n")
    for variant, roles in synthetic.items():
        for role, r in roles.items():
            d = r["distinct"]
            print(
                f"\n**{variant}, {role} set** ({r['pages']} pages, {d['n']} distinct pairs): pHash distance min {d['min']:.0f}, "
                f"p1 {d['p1']:.0f}, p5 {d['p5']:.0f}, median {d['median']:.0f}, p99 {d['p99']:.0f}, max {d['max']:.0f}\n"
            )
            rows = [
                [f"<={t}", d["within"][str(t)], f"{100 * d['within'][str(t)] / d['n']:.2f}%"]
                + [f"{100 * r['copies'][c]['recall'][str(t)]:.0f}%" for c in SYNTHETIC_COPIES]
                for t in THRESHOLDS
            ]
            print(md_table(["threshold", "distinct pairs within", "false-positive rate"] + [f"recall {c}" for c in SYNTHETIC_COPIES], rows))
            rows = []
            for g in DHASH_GATES:
                rows.append(
                    [f"pHash<={max_hamming} and dHash<={g}", f"{100 * r['gate_distinct'][str(g)] / d['n']:.2f}%"]
                    + [f"{100 * r['copies'][c]['gate'][str(g)]['recall']:.0f}%" for c in SYNTHETIC_COPIES]
                )
            print()
            print(md_table(["rule", "false-positive rate"] + [f"recall {c}" for c in SYNTHETIC_COPIES], rows))


def md_table(header, rows):
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    lines += ["| " + " | ".join(str(c) for c in row) + " |" for row in rows]
    return "\n".join(lines)


def report_markdown(results, max_hamming, dhash_gate):
    out = []
    for split_name in ("held_out", "tuning", "all"):
        r = results["splits"][split_name]
        out.append(f"\n### Split: {split_name} ({r['pages']} pages)\n")
        d = r["distinct"]
        out.append(
            f"Distinct pages: {d['n']} pairs; pHash distance min {d['min']:.0f}, p1 {d['p1']:.1f}, p5 {d['p5']:.1f}, "
            f"median {d['median']:.1f}, p99 {d['p99']:.1f}, max {d['max']:.0f}; pairs within {max_hamming} bits: "
            f"{d['within'][str(max_hamming)]}"
        )
        out.append("Closest distinct pairs: " + "; ".join(f"{a} ~ {b} ({k})" for a, b, k in r["closest_distinct_pairs"]))
        if r["distinct_pairs_within_max_hamming"]:
            out.append("Pairs within the threshold: " + "; ".join(f"{a} ~ {b} ({k})" for a, b, k in r["distinct_pairs_within_max_hamming"]))
        rows = []
        for name, p in r["perturbations"].items():
            rows.append(
                [
                    name,
                    f"{p['phash_own']['median']:.0f}",
                    f"{p['phash_own']['mean']:.1f}",
                    f"{p['phash_own']['p95']:.0f}",
                    p["phash_own"]["max"],
                    f"{100 * p['recall'][str(max_hamming)]:.1f}%",
                    f"{100 * p['false_positive'][str(max_hamming)]:.2f}% ({p['false_positive_count_at_max']}/{p['cross_pairs']})",
                    f"{100 * p['gate'][str(dhash_gate)]['recall']:.1f}%",
                    f"{p['dhash_own']['median']:.0f}",
                ]
            )
        out.append(
            md_table(
                ["perturbation", "pHash median", "mean", "p95", "max", f"recall @{max_hamming}", f"false positive @{max_hamming}", f"recall with dHash<={dhash_gate}", "dHash median"],
                rows,
            )
        )
        rows = []
        for name, p in r["perturbations"].items():
            rows.append([name] + [f"{100 * p['recall'][str(t)]:.0f}" for t in THRESHOLDS])
        out.append("\nRecall (%) by threshold:\n")
        out.append(md_table(["perturbation"] + [f"<={t}" for t in THRESHOLDS], rows))
        out.append("\nDistinct-page pairs within threshold: " + ", ".join(f"<={t}: {r['distinct']['within'][str(t)]}" for t in THRESHOLDS))
    return "\n".join(out)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--data", type=Path, default=Path("bench/data"), help="folder holding the DIBCO sets")
    ap.add_argument("--out", type=Path, help="write all raw numbers here as JSON")
    ap.add_argument("--limit", type=int, help="use only the first N pages (smoke test)")
    ap.add_argument("--synthetic", type=int, default=0, metavar="N", help="also hash N synthetic typed pages per variant")
    ap.add_argument("--seed", type=int, default=20261024)
    args = ap.parse_args(argv)

    phash_section = pagetriage_py.params_toml().split("[phash]")[1]

    def default_param(key):
        return next(int(line.split("=")[1]) for line in phash_section.splitlines() if line.startswith(key))

    max_hamming = default_param("max_hamming")
    dhash_gate = default_param("dhash_max_hamming")
    params_toml = None

    pages = discover(args.data)
    if args.limit:
        pages = pages[: args.limit]
    names = [f"{key}/{path.name}" for key, path in pages]
    tune = np.array([key in TUNE_SETS for key, _ in pages])
    print(
        f"{len(pages)} page images, {int(tune.sum())} in the tuning split, "
        f"max_hamming {max_hamming}, dhash_max_hamming {dhash_gate}",
        file=sys.stderr,
    )

    ph0, dh0 = [], []
    ph0_hex, dh0_hex = [], []
    sizes, hash_ms, lowfreq = [], [], []
    ph_pert = {k: [] for k in PERTURBATIONS}
    dh_pert = {k: [] for k in PERTURBATIONS}
    pert_hex = {k: ([], []) for k in PERTURBATIONS}
    flat_pages = 0
    for i, (key, path) in enumerate(pages):
        gray = load_gray(path)
        sizes.append(gray.shape[::-1])
        t0 = time.perf_counter()
        report = hash_page(gray, debug=True)
        hash_ms.append((time.perf_counter() - t0) * 1000.0)
        lowfreq.append(report["debug"]["phash"]["lowfreq_rms"])
        flat_pages += int(report["debug"]["phash"]["flat"])
        ph0_hex.append(report["phash"])
        dh0_hex.append(report["dhash"])
        ph0.append(to_u64(report["phash"]))
        dh0.append(to_u64(report["dhash"]))
        for name, (fn, _) in PERTURBATIONS.items():
            # Seeded by (run seed, page, perturbation name), so adding a perturbation changes no other row.
            rng = np.random.Generator(np.random.PCG64(np.random.SeedSequence([args.seed, i, zlib.crc32(name.encode())])))
            rep = hash_page(np.ascontiguousarray(fn(gray, rng)))
            ph_pert[name].append(to_u64(rep["phash"]))
            dh_pert[name].append(to_u64(rep["dhash"]))
            pert_hex[name][0].append(rep["phash"])
            pert_hex[name][1].append(rep["dhash"])
        print(f"  {i + 1}/{len(pages)} {names[i]}", file=sys.stderr, end="\r")
    print(file=sys.stderr)

    ph0 = np.array(ph0, dtype=np.uint64)
    dh0 = np.array(dh0, dtype=np.uint64)
    ph_pert = {k: np.array(v, dtype=np.uint64) for k, v in ph_pert.items()}
    dh_pert = {k: np.array(v, dtype=np.uint64) for k, v in dh_pert.items()}

    everyone = np.ones(len(pages), dtype=bool)
    results = {
        "pages": len(pages),
        "max_hamming": max_hamming,
        "dhash_max_hamming": dhash_gate,
        "page_sizes": {"min_w": int(min(s[0] for s in sizes)), "max_w": int(max(s[0] for s in sizes)),
                       "min_h": int(min(s[1] for s in sizes)), "max_h": int(max(s[1] for s in sizes)),
                       "median_w": float(np.median([s[0] for s in sizes])), "median_h": float(np.median([s[1] for s in sizes]))},
        "flat_pages": flat_pages,
        "lowfreq_rms_min": float(min(lowfreq)),
        "hash_ms_per_page": {"median": percentile(hash_ms, 50), "max": float(max(hash_ms))},
        "hash_ms_per_megapixel": percentile([ms / (w * h / 1e6) for ms, (w, h) in zip(hash_ms, sizes)], 50),
        "splits": {
            "held_out": evaluate(names, ~tune, ph0, dh0, ph_pert, dh_pert, max_hamming),
            "tuning": evaluate(names, tune, ph0, dh0, ph_pert, dh_pert, max_hamming),
            "all": evaluate(names, everyone, ph0, dh0, ph_pert, dh_pert, max_hamming),
        },
    }
    doc, originals_flagged = document_check(ph0_hex, dh0_hex, pert_hex)
    results["document_check"] = {"originals_flagged_as_duplicates": originals_flagged, "copies": doc}

    # The pure-python reading of the same rule must agree with the Rust function on the originals.
    py_rule = []
    for i in range(len(pages)):
        hit = next(
            (
                j
                for j in range(i)
                if bin(int(ph0[i]) ^ int(ph0[j])).count("1") <= max_hamming
                and bin(int(dh0[i]) ^ int(dh0[j])).count("1") <= dhash_gate
            ),
            None,
        )
        py_rule.append(hit)
    results["rust_matches_python_rule_on_originals"] = py_rule == pagetriage_py.near_duplicates(ph0_hex, dh0_hex)

    if args.synthetic:
        results["synthetic"] = {
            variant: {
                role: synthetic_arm(args.synthetic, variant, args.seed + 7919 * (1 + k) + (0 if variant == "dense" else 101), max_hamming)
                for k, role in enumerate(("tune", "test", "check"))
            }
            for variant in ("dense", "varied")
        }

    print(report_markdown(results, max_hamming, dhash_gate))
    print("\n### Document-level check (originals then one copy each; right / missed / wrong target)\n")
    print(md_table(["perturbation", "right", "missed", "wrong target"],
                   [[k, v["right"], v["missed"], v["wrong_target"]] for k, v in doc.items()]))
    print(f"\nOriginals flagged as duplicates of an earlier original: {originals_flagged}")
    print(f"Rust near_duplicates agrees with the Python rule on the originals: {results['rust_matches_python_rule_on_originals']}")
    print(f"Pages: {results['pages']}, flat pages: {flat_pages}, smallest low-frequency rms: {results['lowfreq_rms_min']:.3f}")
    print(f"Sizes: {results['page_sizes']}")
    print(f"Hash time via Python: median {results['hash_ms_per_page']['median']:.1f} ms/page, "
          f"{results['hash_ms_per_megapixel']:.2f} ms/MP (median), worst page {results['hash_ms_per_page']['max']:.0f} ms")
    if "synthetic" in results:
        print_synthetic(results["synthetic"], max_hamming)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(results, indent=1, sort_keys=True))
        print(f"\nraw numbers: {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
