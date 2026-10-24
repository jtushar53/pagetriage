#!/usr/bin/env python3
"""Set firecrawl/pdf-inspector's per-page verdicts beside ours on the synthetic PDF corpus.

    cargo build --release --manifest-path bench/tools/pdf_inspector_compare/Cargo.toml
    python -I bench/compare_pdf_inspector.py --corpus /tmp/corpus --split dev

pdf-inspector reports, per page, only whether the page needs OCR and why (`scanned`, `no_text`,
`vector_text`, `invisible_text_layer`, `suspected_garbled_text`). It has no page-level `native_text`
versus `mixed` distinction, so both of those are folded into `text` here. This is a comparison for
the decision recorded in docs/detectors/text_layer.md, not a benchmark of pdf-inspector: nothing in
pagetriage was tuned on it. Same dev/held-out split rule as structure_eval.py.
"""
import argparse
import collections
import json
import pathlib
import subprocess
import sys

import pagetriage_py

COARSE_OF_LABEL = {"native_text": "text", "mixed": "text"}
CLASSES = ["text", "raster_only", "invisible_ocr_under_image", "vector_only", "empty", "garbled"]


def coarse_label(state):
    return COARSE_OF_LABEL.get(state, state)


def pdf_inspector_page(reasons):
    if not reasons:
        return "text"
    if "invisible_text_layer" in reasons:
        return "invisible_ocr_under_image"
    if "scanned" in reasons:
        return "raster_only"
    if "vector_text" in reasons:
        return "vector_only"
    if "no_text" in reasons:
        return "empty"
    return "garbled"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--corpus", required=True, type=pathlib.Path)
    ap.add_argument("--split", choices=["dev", "heldout", "all"], default="dev")
    ap.add_argument(
        "--tool",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parent / "tools/pdf_inspector_compare/target/release/pdf-inspector-compare",
    )
    args = ap.parse_args()

    labels = json.loads((args.corpus / "labels.json").read_text())["files"]
    names = sorted(n for n in labels if args.split == "all" or (args.split == "dev") == (int(n[4:7]) % 2 == 1))
    out = subprocess.run([str(args.tool), *[str(args.corpus / n) for n in names]], capture_output=True, text=True, check=True)
    theirs = {}
    for line in out.stdout.splitlines():
        rec = json.loads(line)
        theirs[rec["file"]] = rec

    agree_theirs = agree_ours = total = 0
    conf = collections.Counter()
    errors = 0
    for n in names:
        rec = theirs[n]
        if "error" in rec:
            errors += 1
            print("pdf-inspector error on", n, rec["error"])
            continue
        ours = json.loads(pagetriage_py.analyse_pdf_structure((args.corpus / n).read_bytes()))
        for i, page in enumerate(labels[n]["pages"]):
            truth = coarse_label(page["text_layer"])
            theirs_page = pdf_inspector_page(rec["reasons"].get(str(i + 1)))
            ours_page = coarse_label(ours["pages"][i]["text_layer"]["state"])
            total += 1
            agree_theirs += theirs_page == truth
            agree_ours += ours_page == truth
            conf[(truth, theirs_page)] += 1
    print(f"split={args.split} pages={total} files with pdf-inspector errors={errors}")
    print(f"coarse classes (native_text and mixed folded into 'text'):")
    print(f"  pdf-inspector agrees with the label on {agree_theirs}/{total} = {100 * agree_theirs / max(1, total):.1f}%")
    print(f"  pagetriage    agrees with the label on {agree_ours}/{total} = {100 * agree_ours / max(1, total):.1f}%")
    print("\npdf-inspector confusion (label \\ pdf-inspector):")
    print("label".ljust(28) + "".join(c[:12].rjust(14) for c in CLASSES))
    for t in CLASSES[:-1]:
        print(t.ljust(28) + "".join(str(conf[(t, p)]).rjust(14) for p in CLASSES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
