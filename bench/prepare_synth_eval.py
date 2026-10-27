#!/usr/bin/env python3
"""Check `pagetriage prepare` on synthetic forms that are tilted and turned by known amounts.

    cargo build --release -p pagetriage-cli
    python -I bench/prepare_synth_eval.py [--cli target/release/pagetriage]

Pages: A4 at 150 dpi, a printed form with handwritten fill-ins (bench/synth/handwrite.py form_page), seeds
1-3 dev and 11-13 test. Each is tilted by 0, 3, 6 or 10 degrees (Pillow, expanding the canvas) and then
turned by 0, 90, 180 or 270 degrees, then given to `prepare`. Reported per tilt:

- the quarter turn the manifest applied, against the one that restores the page: right, abstained (no
  turn although the page was turned) or wrong (a different turn);
- for pages that were turned right or never turned: the skew `analyse` measures on the prepared
  `page.png` (it should be near 0; the input's own measured skew is the baseline), as the share within
  0.5 degrees and the median absolute value;
- the area of `page.png` (the content box, before scaling) as a share of the input's area. A tilted
  page comes out on a bigger canvas, so the crop of a 10-degree page is measured against its tilted
  canvas.

The pages are the author's idea of a filled-in form; nothing here says how a real scan behaves. Needs
numpy, Pillow and the release CLI.
"""
from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent / "synth"))
import handwrite  # noqa: E402

DEV, TEST = [1, 2, 3], [11, 12, 13]
TILTS = [0.0, 3.0, 6.0, 10.0]
TURNS = {0: None, 90: Image.ROTATE_270, 180: Image.ROTATE_180, 270: Image.ROTATE_90}  # clockwise turns


def run(cli, *args):
    out = subprocess.run([cli, *args], check=True, capture_output=True, text=True)
    return json.loads(out.stdout)


def skew_of(cli, path):
    return run(cli, "analyse", "--dpi", "150", "--detectors", "blank_page", str(path))["pages"][0]["quality"]["skew_deg"]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--cli", default=str(Path(__file__).resolve().parents[1] / "target" / "release" / "pagetriage"))
    args = ap.parse_args()
    print(f"{'tilt':>5} {'split':<5} {'turn right':>11} {'abstained':>10} {'wrong':>6} {'level <0.5':>11} {'median |skew|':>14} {'baseline':>9} {'page crop':>13}")
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for tilt in TILTS:
            for split, seeds in (("dev", DEV), ("test", TEST)):
                right = abstained = wrong = 0
                residual, baseline, share = [], [], []
                for seed in seeds:
                    img, _, _ = handwrite.form_page(seed)
                    if tilt:
                        img = img.rotate(tilt, expand=True, fillcolor=img.getpixel((0, 0)))
                    for turn, op in TURNS.items():
                        page = img if op is None else img.transpose(op)
                        src = tmp / "in.png"
                        page.save(src, dpi=(150, 150))
                        out = tmp / "out"
                        m = run(args.cli, "prepare", "-o", str(out), str(src))
                        applied = sum(s["clockwise_deg"] for s in m["steps"] if s["op"] == "rotate") % 360
                        want = (360 - turn) % 360
                        if turn == 0:
                            ok = applied == 0
                            right += ok
                            wrong += not ok
                        elif applied == want:
                            right += 1
                        elif applied == 0:
                            abstained += 1
                        else:
                            wrong += 1
                        if applied == want and m["outputs"]:
                            residual.append(abs(skew_of(args.cli, out / "page.png")))
                            baseline.append(abs(skew_of(args.cli, src)) if turn == 0 else float("nan"))
                            kept = next(o for o in m["outputs"] if o["kind"] == "page")
                            share.append(kept["width"] * kept["height"] / kept["scale"] ** 2 / (page.width * page.height))
                n = 4 * len(seeds)
                level = sum(r < 0.5 for r in residual) / max(len(residual), 1)
                base = [b for b in baseline if b == b]
                print(f"{tilt:>5.0f} {split:<5} {f'{right}/{n}':>11} {abstained:>10} {wrong:>6} {level:>11.2f} "
                      f"{statistics.median(residual) if residual else float('nan'):>14.2f} "
                      f"{statistics.median(base) if base else float('nan'):>9.2f} "
                      f"{statistics.median(share) if share else float('nan'):>13.2f}")


if __name__ == "__main__":
    main()
