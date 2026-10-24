#!/usr/bin/env python3
"""Run the analyser over a directory of page images and write one JSON line per image.

    python bench/run.py --images bench/data/dibco2014/original_images --out bench/results/dibco2014.jsonl
    python bench/run.py --images tests/golden/inputs --glob '*.pgm' --debug

Needs the Python binding (`maturin develop --release -m crates/pagetriage-py/Cargo.toml`) and
`pip install -r bench/requirements.txt`. All analysis happens in the Rust core: this script only
decodes images, calls `pagetriage_py.analyse_page`, and records the result. With `--debug` the raw
detector features are recorded too, which is what threshold tuning reads.

Each output line: {"image": <path relative to --images>, "width", "height", "report": <page report>}.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pagetriage_py
from PIL import Image

Image.MAX_IMAGE_PIXELS = 400_000_000  # DIBCO 2019 track B scans are large; still refuse true bombs


def load_buffers(path: Path):
    """Return (gray_bytes, rgb_bytes_or_None, width, height). 8-bit gray stays as is; anything else
    becomes RGB and the core's own luma makes the gray plane."""
    with Image.open(path) as img:
        img.load()
        width, height = img.size
        if img.mode == "L":
            return img.tobytes(), None, width, height
        rgb = img.convert("RGB").tobytes()
    return pagetriage_py.rgb_to_gray(rgb), rgb, width, height


def iter_images(root: Path, pattern: str):
    for path in sorted(root.rglob(pattern)):
        if path.is_file():
            yield path


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--images", type=Path, required=True, help="directory searched recursively")
    ap.add_argument("--glob", default="*", help="file name pattern (default: every file Pillow can open)")
    ap.add_argument("--out", type=Path, help="JSONL output path (default: stdout)")
    ap.add_argument("--dpi", type=int, default=150)
    ap.add_argument("--debug", action="store_true", help="record raw detector features")
    ap.add_argument("--opts", default="{}", help="extra analyser options as JSON (merged under debug)")
    args = ap.parse_args(argv)

    if not args.images.is_dir():
        print(f"error: {args.images} is not a directory", file=sys.stderr)
        return 2
    try:
        opts = json.loads(args.opts)
    except json.JSONDecodeError as e:
        print(f"error: --opts is not valid JSON: {e}", file=sys.stderr)
        return 2
    opts["debug"] = args.debug or opts.get("debug", False)
    opts_json = json.dumps(opts, sort_keys=True)

    out = open(args.out, "w", encoding="utf-8") if args.out else sys.stdout
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
    done = skipped = 0
    try:
        for path in iter_images(args.images, args.glob):
            try:
                gray, rgb, width, height = load_buffers(path)
            except (OSError, ValueError) as e:  # not an image, or truncated
                print(f"skip {path.relative_to(args.images)}: {e}", file=sys.stderr)
                skipped += 1
                continue
            report = json.loads(pagetriage_py.analyse_page(gray, width, height, args.dpi, rgb, None, opts_json))
            line = {"image": str(path.relative_to(args.images)), "width": width, "height": height, "report": report}
            out.write(json.dumps(line, sort_keys=True) + "\n")
            done += 1
    finally:
        if args.out:
            out.close()
    print(f"analysed {done} image(s), skipped {skipped}", file=sys.stderr)
    return 0 if done else 1


if __name__ == "__main__":
    sys.exit(main())
