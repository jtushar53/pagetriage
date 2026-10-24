#!/usr/bin/env python3
"""Check the quality block (module M3) against synthetic pages whose degradation is known.

    python -I bench/quality_synth_eval.py
    python -I bench/quality_synth_eval.py --only skew,blur

Pages come from bench/synth (Pillow text in Latin; procedural pseudo-glyphs with a headline for
Devanagari and Bengali), so nothing here is a real document and nothing depends on installed fonts.
Each section sweeps one degradation and prints what the analyser measured next to the truth. The
pages are clean renderings with controlled damage, so these numbers say whether a measure responds
in the right direction and roughly the right amount. They do not say how it behaves on real scans:
see bench/skew_eval.py and bench/quality_eval.py for the DIBCO runs.

Needs the Python binding and `pip install numpy Pillow`.
"""
from __future__ import annotations

import argparse
import json
import random
import statistics
import sys
import time
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image, ImageFilter

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import scanimg  # noqa: E402
import synthtext  # noqa: E402

W, H, DPI = 1240, 1754, 150  # A4 at 150 dpi


def render(script="latin", skew=0.0, noise=2.0, speckle=0.0, blur=0.0, seed=1, pt=11.0):
    rng, np_rng = random.Random(seed), np.random.default_rng(seed)
    paras = [synthtext.paragraph(rng, script, 6) for _ in range(8)]
    scan = scanimg.render_scan(rng, np_rng, W, H, DPI, script, paras, pt, (0.1, 0.08, 0.1, 0.08),
                               skew, noise, speckle, blur, 0.0)
    return scan.image


def analyse(img, params=None):
    opts = {"detectors": [], "debug": True}
    if params:
        opts["params"] = params
    t = time.perf_counter()
    out = pagetriage_py.analyse_page(img.tobytes(), img.width, img.height, DPI, None, None, json.dumps(opts))
    ms = (time.perf_counter() - t) * 1000
    rep = json.loads(out)
    return rep["quality"], rep["debug"]["quality"], ms


def sec_skew():
    print("== skew: known rotation, three scripts (error = estimate - truth, degrees)")
    angles = [-9.0, -4.3, -1.1, 0.0, 0.7, 3.2, 8.6]
    print(f"{'script':<11} " + " ".join(f"{a:>6}" for a in angles) + "   max|err|")
    for script in ["latin", "devanagari", "bengali"]:
        errs = []
        for a in angles:
            q, f, _ = analyse(render(script, skew=a, seed=3))
            errs.append(q["skew_deg"] - a)
        print(f"{script:<11} " + " ".join(f"{e:+6.2f}" for e in errs) + f"   {max(abs(e) for e in errs):.2f}")


def sec_blur():
    print("\n== blur: Gaussian blur radius applied after rendering (pixels)")
    print(f"{'blur':>5} {'est sigma px':>13} {'sigma mm':>9} {'lap var':>10} {'grade':>5} {'badness':>8}")
    for b in [0.0, 0.5, 1.0, 1.5, 2.0, 3.0]:
        q, f, _ = analyse(render("latin", blur=b, seed=4))
        print(f"{b:5.1f} {f['blur_sigma_px']:13.2f} {f['blur_sigma_mm']:9.3f} {q['blur_laplacian_var']:10.0f} "
              f"{q['grade']:>5} {f['grade_badness']:8.2f}")


def sec_speckle():
    print("\n== speckle: random dark pixels, density = fraction of page pixels set")
    print(f"{'set':>9} {'known /px':>10} {'measured /px':>13} {'ratio':>6} {'grade':>5}")
    for s in [0.0, 0.0002, 0.0005, 0.001, 0.003]:
        q, f, _ = analyse(render("latin", speckle=s, seed=5))
        known = s
        ratio = q["speckle_density"] / known if known else float("nan")
        print(f"{s:9.4f} {known:10.5f} {q['speckle_density']:13.5f} {ratio:6.2f} {q['grade']:>5}")


def composite_bleed(front: Image.Image, back: Image.Image, alpha: float) -> Image.Image:
    """Darken the front by the mirrored, blurred ink of the back, as show-through does."""
    mirrored = back.transpose(Image.FLIP_LEFT_RIGHT).filter(ImageFilter.GaussianBlur(0.8))
    f = np.asarray(front, dtype=np.float32)
    b = np.asarray(mirrored, dtype=np.float32)
    out = f * (1.0 - alpha * (1.0 - b / 255.0))
    return Image.fromarray(np.clip(out, 0, 255).astype(np.uint8), "L")


def sec_bleed():
    print("\n== bleed-through: a mirrored second page shows through at strength alpha")
    print(f"{'alpha':>6} {'bleed':>7} {'ghost ratio':>12} {'mid mass':>9} {'binariser':>10} {'grade':>5}")
    front = render("latin", seed=6)
    back = render("latin", seed=7)
    for a in [0.0, 0.08, 0.15, 0.25, 0.4]:
        q, f, _ = analyse(composite_bleed(front, back, a) if a else front)
        print(f"{a:6.2f} {q['bleed_through']:7.3f} {f['bleed_ghost_stroke_ratio']:12.3f} "
              f"{f['bleed_mid_mass']:9.4f} {q['recommended_binarizer']:>10} {q['grade']:>5}")


def add_crease(img: Image.Image, horizontal: bool, at: float, depth=22, shade=6) -> Image.Image:
    a = np.asarray(img, dtype=np.float32).copy()
    n = a.shape[0] if horizontal else a.shape[1]
    k = int(at * n)
    sl = (lambda lo, hi: a[lo:hi]) if horizontal else (lambda lo, hi: a[:, lo:hi])
    sl(k, k + 2)[...] -= depth
    sl(k + 2, k + 30)[...] -= shade
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8), "L")


def sec_folds():
    print("\n== fold lines: synthetic creases (2 px valley 22 levels deep, 6 levels of shade on one side)")
    base = render("latin", seed=8)
    cases = [("none", base), ("1 horizontal at 1/2", add_crease(base, True, 0.5)),
             ("1 vertical at 1/2", add_crease(base, False, 0.5)),
             ("2 horizontal at 1/3, 2/3", add_crease(add_crease(base, True, 1 / 3), True, 2 / 3)),
             ("letter fold + vertical", add_crease(add_crease(base, True, 0.5), False, 0.5))]
    print(f"{'case':<28} {'folds':>5} {'H':>3} {'V':>3} {'depth':>6}")
    for name, img in cases:
        q, f, _ = analyse(img)
        print(f"{name:<28} {q['fold_lines']:5d} {f['fold_horizontal']:3.0f} {f['fold_vertical']:3.0f} "
              f"{f['fold_max_depth']:6.1f}")


def photocopy(img: Image.Image, generations: int, seed: int) -> Image.Image:
    """Crude simulated photocopying: each pass blurs, adds sensor noise, clips contrast and drops
    some ink. This is a simulation for a sanity check, not a model of a real copier."""
    rng = np.random.default_rng(seed)
    a = np.asarray(img, dtype=np.float32)
    for _ in range(generations):
        pil = Image.fromarray(np.clip(a, 0, 255).astype(np.uint8), "L").filter(ImageFilter.GaussianBlur(0.7))
        a = np.asarray(pil, dtype=np.float32) + rng.normal(0, 7, a.shape)
        a = np.clip((a - 60.0) * 1.5, 0, 255)  # lose the faint end, clip the dark end
        a = 255.0 - 0.82 * (255.0 - a)  # toner never gets fully black again
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8), "L")


def sec_generation():
    print("\n== photocopy generation (EXPERIMENTAL): simulated copies of a clean page")
    print(f"{'copies':>6} {'estimate':>9} {'confidence':>11} {'cv':>6} {'frag':>6} {'gap':>5} {'grade':>5}")
    base = render("latin", seed=9)
    for k in range(0, 6):
        q, f, _ = analyse(photocopy(base, k, 11) if k else base)
        g = q["photocopy_generation_est"]
        print(f"{k:6d} {(g['value'] if g else -1):9d} {(g['confidence'] if g else 0):11.2f} "
              f"{f.get('stroke_width_cv', 0):6.2f} {f['fragment_rate']:6.2f} {f['peak_gap']:5.2f} {q['grade']:>5}")


def sec_time():
    print("\n== time: A4 at 150 dpi, quality pass only (Python call wall time, includes the JSON)")
    img = render("latin", seed=10)
    times = [analyse(img)[2] for _ in range(7)]
    print(f"median {statistics.median(times):.1f} ms, min {min(times):.1f} ms, max {max(times):.1f} ms "
          f"(native, this machine)")


SECTIONS = {"skew": sec_skew, "blur": sec_blur, "speckle": sec_speckle, "bleed": sec_bleed,
            "folds": sec_folds, "generation": sec_generation, "time": sec_time}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--only", help="comma-separated sections: " + ", ".join(SECTIONS))
    args = ap.parse_args(argv)
    names = args.only.split(",") if args.only else list(SECTIONS)
    for n in names:
        if n not in SECTIONS:
            sys.exit(f"unknown section {n}; known: {', '.join(SECTIONS)}")
        SECTIONS[n]()
    return 0


if __name__ == "__main__":
    sys.exit(main())
