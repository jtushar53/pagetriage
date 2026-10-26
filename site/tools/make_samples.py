#!/usr/bin/env python3
"""Regenerate the demo page's sample images and their manifest.

    .venv/bin/python site/tools/make_samples.py [--cli target/release/pagetriage]

Every sample is the same synthetic deed-style text (generated, no real document content) with one
degradation applied, so the page can show what each one does to the readout. The degradation helpers
are the ones `bench/quality_synth_eval.py` uses. Output goes to `site/samples/`:

* `<id>.png`        the page, 8-bit gray, lossless (PNG, never JPEG: decoders differ in the last bit)
* `<id>.thumb.png`  a small copy for the picker; display only, never analysed
* `samples.json`    titles, what each page shows, and the SHA-256 of the report the *native* build
                    produces for it. The page compares that with the report its own WASM build
                    produces in your browser.

Native reference: for a page image, the Rust core's `analyse_page` compiled for this machine and
loaded through the Python wheel (the CLI would wrap the same report in a document envelope, so its
bytes differ); for a PDF, the native CLI's `structure` output, which is the bare report.

The PDF samples are the golden PDFs in `tests/golden/pdf/`; their manifest is written here too, with
native hashes, so `site/tools/check-samples.mjs` can check both against the WASM build.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
sys.path.insert(0, str(ROOT / "bench" / "synth"))

import pagetriage_py  # noqa: E402
import quality_synth_eval as q  # noqa: E402  (bench helpers: render, composite_bleed, add_crease)
from PIL import Image  # noqa: E402

W, H, DPI = 827, 1170, 150  # A5 at 150 dpi: big enough to read, small enough to ship
q.W, q.H = W, H

# Pillow's built-in font has no en dash, so it would print as an empty box in the sample text. Swap
# it for a colon here only; the random draws, and so the layout, are unchanged.
_paragraph = q.synthtext.paragraph
q.synthtext.paragraph = lambda *a, **k: _paragraph(*a, **k).replace("–", ":")

SEED_FRONT, SEED_BACK = 11, 12

# id, title, what it shows (plain words), how it was built
SAMPLES = [
    ("clean", "Clean scan",
     "A tidy page. Nothing to fix, so the cheap binariser is recommended.",
     "No degradation."),
    ("tilted", "Tilted 4 degrees",
     "The page went into the scanner crooked. The readout gives the angle to rotate it back.",
     "Rotated 4.0 degrees counter-clockwise."),
    ("ghosted", "Bleed-through",
     "Ink from the back of the sheet shows through. Plain thresholds read the ghost as text, so the graph-cut method is recommended.",
     "A mirrored, blurred second page blended in at strength 0.25."),
    ("blurred", "Out of focus",
     "Soft strokes and low contrast. The sharpness measure drops and the grade falls.",
     "Gaussian blur, radius 2.5 px."),
    ("blurred_hard", "Badly out of focus",
     "Strokes have merged into blobs. The grade is D. With the line-level detectors switched on (script, handwriting, signature), they are skipped on a page like this and a degraded finding is raised instead of a guess.",
     "Gaussian blur, radius 4 px."),
    ("photocopied", "Photocopied five times",
     "Toner thinned and edges eaten. A known weak spot: the grade stays A and the generation estimate says 2 copies, not 5. Do not trust this readout on heavy copies.",
     "A simulated copier pass (blur, noise, contrast clip, lighter toner) applied five times."),
    ("creased", "Folded twice",
     "A letter fold and a vertical fold left creases. They are found and counted.",
     "One horizontal and one vertical crease at the middle of the page."),
    ("speckled", "Photocopy specks",
     "Dust and toner specks of the kind multi-generation photocopies collect.",
     "Random dark pixels on 0.3% of the page."),
]


def build_images():
    front = q.render("latin", seed=SEED_FRONT)
    back = q.render("latin", seed=SEED_BACK)
    return {
        "clean": front,
        "tilted": q.render("latin", seed=SEED_FRONT, skew=4.0),
        "ghosted": q.composite_bleed(front, back, 0.25),
        "blurred": q.render("latin", seed=SEED_FRONT, blur=2.5),
        "blurred_hard": q.render("latin", seed=SEED_FRONT, blur=4.0),
        "photocopied": q.photocopy(front, 5, 11),
        "creased": q.add_crease(q.add_crease(front, True, 0.5), False, 0.5),
        "speckled": q.render("latin", seed=SEED_FRONT, speckle=0.003),
    }


def native_page_hash(img: Image.Image) -> str:
    report = pagetriage_py.analyse_page(img.tobytes(), img.width, img.height, DPI, None, None, "{}")
    return hashlib.sha256(report.encode("utf-8")).hexdigest()


def native_hash(cli: Path, *args: str) -> str:
    out = subprocess.run([str(cli), *args], check=True, capture_output=True).stdout
    return hashlib.sha256(out.rstrip(b"\n")).hexdigest()


# What each golden PDF is, in plain words (from tests/golden/gen_pdfs.py).
PDFS = [
    ("native_latin", "Born-digital English", "Real text with fonts. The text layer is trustworthy."),
    ("scanned_ocr", "Scan with a hidden OCR layer", "A page image with invisible text laid over it by an OCR engine. Text exists, but it is the OCR's guess."),
    ("jpeg_scan", "Scan, no text at all", "Just a JPEG of a page. The JPEG quality is read from its tables."),
    ("scripts_legacy", "Hindi in four font families", "One page uses a legacy font (Kruti Dev) that looks like Devanagari on screen but copies as Latin gibberish. It is flagged."),
    ("signed_then_edited", "Edited after signing", "An incremental update was appended after the first save, the footprint of a change made later."),
    ("encrypted", "Encrypted PDF", "Cannot be read without a password. Reported as such, not guessed at."),
    ("redacted_text", "Boxes over text that is still there", "A black box over a name and a white box over an address, both painted on top of text that is still in the file. Anyone can copy the text out from under them."),
    ("vector_empty_mixed", "Vector, blank and mixed pages", "Drawn lines with no text, an empty page, and a page mixing both."),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", default=str(ROOT / "target" / "release" / "pagetriage"))
    args = ap.parse_args()
    cli = Path(args.cli)
    if not cli.exists():
        print(f"build the CLI first: cargo build --release -p pagetriage-cli ({cli} not found)", file=sys.stderr)
        return 2

    out = ROOT / "site" / "samples"
    out.mkdir(parents=True, exist_ok=True)
    images = build_images()
    pages = []
    for sid, title, blurb, built in SAMPLES:
        img = images[sid].convert("L")
        assert img.size == (W, H), (sid, img.size)
        path = out / f"{sid}.png"
        img.save(path, optimize=True)
        thumb = img.resize((96, round(96 * H / W)), Image.LANCZOS)
        thumb.save(out / f"{sid}.thumb.png", optimize=True)
        pages.append({
            "id": sid, "title": title, "blurb": blurb, "built": built,
            "file": f"samples/{sid}.png", "thumb": f"samples/{sid}.thumb.png",
            "width": W, "height": H, "dpi": DPI,
            "native_report_sha256": native_page_hash(img),
        })
        print(f"{sid:9s} {path.stat().st_size / 1024:6.0f} KB")

    pdf_dir = out / "pdf"
    pdf_dir.mkdir(exist_ok=True)
    pdfs = []
    for pid, title, blurb in PDFS:
        src = ROOT / "tests" / "golden" / "pdf" / f"{pid}.pdf"
        dst = pdf_dir / src.name
        shutil.copyfile(src, dst)
        pdfs.append({
            "id": pid, "title": title, "blurb": blurb, "file": f"samples/pdf/{src.name}",
            "bytes": dst.stat().st_size,
            "native_report_sha256": native_hash(cli, "structure", str(dst)),
        })

    manifest = {
        "note": "Generated by site/tools/make_samples.py. Synthetic pages: no real document content.",
        "pages": pages, "pdfs": pdfs,
    }
    (out / "samples.json").write_text(json.dumps(manifest, indent=1) + "\n")
    print(f"wrote {out / 'samples.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
