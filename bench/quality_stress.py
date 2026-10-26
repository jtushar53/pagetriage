#!/usr/bin/env python3
"""Stress the layout and the quality gate on much worse synthetic pages than the demo's.

    python -I bench/quality_stress.py

One 8-paragraph Latin page (bench/synth, as in quality_synth_eval.py), each damage applied alone. For
every case: the quality grade and blur severity, the text lines the layout finds with the gate off
(`no_gate`), and what the default run reports (the `degraded` finding when the gate fired). The page
has as many lines as the clean render shows (first row). Synthetic pages only: nothing here says how a
real bad scan behaves. Needs the Python binding, numpy and Pillow.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pagetriage_py
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import quality_synth_eval as q  # noqa: E402

DETECTORS = ["layout", "script"]


def run(img, no_gate):
    opts = {"detectors": DETECTORS, "debug": True, "no_gate": no_gate}
    out = pagetriage_py.analyse_page(img.tobytes(), img.width, img.height, q.DPI, None, None, json.dumps(opts))
    return json.loads(out)


def main():
    base = q.render("latin", seed=1)
    back = q.render("latin", seed=2)
    cases = [("clean", base)]
    cases += [(f"blur {b} px", q.render("latin", blur=b, seed=1)) for b in (2, 3, 4, 6)]
    cases += [(f"noise sigma {n}", q.render("latin", noise=n, seed=1)) for n in (20, 40, 60)]
    cases += [("speckle 3%", q.render("latin", speckle=0.03, seed=1)),
              ("bleed-through 0.4", q.composite_bleed(base, back, 0.4)),
              ("photocopied x5", q.photocopy(base, 5, 11)),
              ("photocopied x10", q.photocopy(base, 10, 11)),
              ("skew 25 deg", q.render("latin", skew=25.0, seed=1)),
              ("blank", Image.fromarray(np.full((q.H, q.W), 240, np.uint8)))]
    print(f"{'case':<18}{'grade':>6}{'blur sev':>9}{'lines (gate off)':>18}   default run")
    for name, img in cases:
        off = run(img, True)
        on = run(img, False)
        kinds = sorted({f["kind"] for f in on["findings"]})
        print(f"{name:<18}{off['quality']['grade']:>6}{off['debug']['quality']['grade_sev_blur']:>9.2f}"
              f"{off['debug']['layout']['lines']:>18.0f}   {', '.join(kinds) or '-'}")


if __name__ == "__main__":
    main()
