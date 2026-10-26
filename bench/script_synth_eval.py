#!/usr/bin/env python3
"""Check the pixel script-family detector (module M4) against synthetic pages of known script.

    python -I bench/script_synth_eval.py                 # dev and test seeds, confusion matrices
    python -I bench/script_synth_eval.py --features      # per-family feature ranges on dev (for tuning)

Pages come from bench/synth: Pillow text in Latin; procedural pseudo-glyphs for the Indic scripts
(with a headline for Devanagari, Bengali and Gurmukhi) and a procedural joined-letter script with
dots for Arabic. The pseudo-glyphs were written by the same author as the detector, so they share
its assumptions (a headline is a bar, Arabic letters join along a baseline). A high score here says
the code does what it was designed to do. It says nothing about real Devanagari, Urdu or Latin print.

Seeds 1-5 are the dev split (thresholds were tuned on these), seeds 11-15 the test split (looked at
once per change). Needs the Python binding and `pip install numpy Pillow`.
"""
from __future__ import annotations

import argparse
import json
import random
import sys
from collections import Counter
from pathlib import Path

import numpy as np
import pagetriage_py

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import scanimg  # noqa: E402
import synthtext  # noqa: E402

W, H, DPI = 1240, 1754, 150
OPTS = json.dumps({"detectors": ["script"], "debug": True})
FAMILY = {"latin": "discrete", "gujarati": "discrete", "tamil": "discrete", "telugu": "discrete",
          "malayalam": "discrete", "devanagari": "headline", "bengali": "headline",
          "gurmukhi": "headline", "arabic": "cursive"}
DEV, TEST = [1, 2, 3, 4, 5], [11, 12, 13, 14, 15]
CONDITIONS = {
    "clean 11pt": dict(pt=11.0),
    "9pt": dict(pt=9.0),
    "14pt": dict(pt=14.0),
    "skew 3deg": dict(pt=11.0, skew=3.0),
    "noise 8": dict(pt=11.0, noise=8.0),
    "speckle 0.3%": dict(pt=11.0, speckle=0.003),
    "blur 1 px": dict(pt=11.0, blur=1.0),
}


def render(script, seed, pt=11.0, noise=2.0, skew=0.0, blur=0.0, speckle=0.0):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, script, 5) for _ in range(6)]
    return scanimg.render_scan(rng, np_rng, W, H, DPI, script, paras, pt, (0.1, 0.08, 0.1, 0.08),
                               skew, noise, speckle, blur, 0.0).image


def analyse(img, params=None):
    opts = {"detectors": ["script"], "debug": True}
    if params:
        opts["params"] = params
    out = json.loads(pagetriage_py.analyse_page(np.asarray(img).tobytes(), img.width, img.height, DPI,
                                                None, None, json.dumps(opts)))
    found = [f["attributes"]["family"] for f in out["findings"] if f["kind"] == "script"]
    return (found[0] if found else "abstain"), out["debug"]["script"]


def run(seeds, params=None, only=None):
    results = []  # (condition, script, truth, got, features)
    for cond, kw in CONDITIONS.items():
        for script in FAMILY:
            for seed in seeds:
                got, feats = analyse(render(script, seed, **kw), params)
                results.append((cond, script, FAMILY[script], got, feats))
    return results


def report(label, results):
    fams = ["headline", "cursive", "discrete", "abstain"]
    print(f"\n== {label}: {len(results)} pages")
    print(f"{'truth \\ got':<12}" + "".join(f"{f:>10}" for f in fams))
    for t in fams[:3]:
        c = Counter(r[3] for r in results if r[2] == t)
        print(f"{t:<12}" + "".join(f"{c[f]:>10}" for f in fams))
    right = sum(r[2] == r[3] for r in results)
    print(f"correct {right}/{len(results)} = {right / len(results):.1%}")
    print(f"{'condition':<14} {'correct':>8}   wrong cases")
    for cond in CONDITIONS:
        rs = [r for r in results if r[0] == cond]
        bad = Counter(f"{r[1]}->{r[3]}" for r in rs if r[2] != r[3])
        print(f"{cond:<14} {sum(r[2] == r[3] for r in rs):>4}/{len(rs):<3}   {dict(bad) if bad else ''}")


def mixed_page(top_script, bottom_script, seed):
    """Upper 45% of one page over the lower 45% of another: a bilingual page, not a real one."""
    from PIL import Image
    a, b = render(top_script, seed), render(bottom_script, seed + 100)
    img = Image.new("L", (W, H), 255)
    img.paste(a.crop((0, 0, W, int(H * 0.45))), (0, 0))
    img.paste(b.crop((0, 0, W, int(H * 0.45))), (0, int(H * 0.5)))
    return img


def sec_mixed():
    print("\n== mixed pages (half and half): the findings reported, primary first")
    for top, bottom in [("latin", "devanagari"), ("devanagari", "latin"), ("latin", "arabic"), ("devanagari", "arabic")]:
        want = {FAMILY[top], FAMILY[bottom]}
        ok, shown = 0, ""
        for seed in DEV + TEST:
            out = json.loads(pagetriage_py.analyse_page(
                np.asarray(mixed_page(top, bottom, seed)).tobytes(), W, H, DPI, None, None, OPTS))
            fams = [f["attributes"]["family"] for f in out["findings"] if f["kind"] == "script"]
            ok += set(fams) == want
            shown = shown or "+".join(fams)
        print(f"{top:>10} over {bottom:<10} both families found on {ok}/{len(DEV + TEST)} pages (e.g. {shown})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--features", action="store_true")
    ap.add_argument("--params", help="TOML override, as the analyser's `params` option")
    args = ap.parse_args()
    dev = run(DEV, args.params)
    if args.features:
        for fam in ["headline", "cursive", "discrete"]:
            rs = [r for r in dev if r[2] == fam and r[1] != "latin" or r[2] == fam and r[1] == "latin"]
            print(f"\n{fam} (dev, {len(rs)} pages)")
            for k in ["top_peak", "low_peak", "comp_density", "small_share"]:
                v = [r[4][k] for r in rs if k in r[4]]
                print(f"  {k:<13} min {min(v):.2f}  median {sorted(v)[len(v) // 2]:.2f}  max {max(v):.2f}")
        return
    report("dev (seeds 1-5, tuned on)", dev)
    report("test (seeds 11-15)", run(TEST, args.params))
    sec_mixed()


if __name__ == "__main__":
    main()
