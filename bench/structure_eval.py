#!/usr/bin/env python3
"""Score the PDF structural layer (modules M1, M2, M12) against the synthetic corpus.

    python -I bench/synth/make_pdf_corpus.py --out /tmp/corpus
    python -I bench/structure_eval.py --corpus /tmp/corpus --split dev

The corpus is written by us, so a high score here is weak evidence about real PDFs (see
docs/detectors/text_layer.md for what it does not cover). To keep one number honest, the 60
documents are split by file number: odd numbers are the *dev* split (used while building the
analyser: inspect errors, tune thresholds), even numbers are the *held-out* split. Run
`--split heldout` sparingly and do not tune on what it shows. `--split all` is for the final table.

Needs `pagetriage_py` (maturin develop --release -m crates/pagetriage-py/Cargo.toml). No algorithm
lives here: this only reads the JSON the core returns and counts.
"""
import argparse
import collections
import json
import pathlib
import sys
import time

import pagetriage_py

STATES = [
    "native_text",
    "raster_only",
    "invisible_ocr_under_image",
    "mixed",
    "vector_only",
    "empty",
]


def doc_number(name):
    return int(name.split("_")[1].split(".")[0])


def in_split(name, split):
    n = doc_number(name)
    return split == "all" or (split == "dev") == (n % 2 == 1)


def pct(a, b):
    return "n/a" if b == 0 else f"{100.0 * a / b:.1f}%"


def confusion_table(conf):
    width = max(len(s) for s in STATES) + 2
    head = "label \\ predicted".ljust(width) + "".join(s[:12].rjust(14) for s in STATES + ["(none)"])
    lines = [head]
    for truth in STATES:
        row = [conf[(truth, p)] for p in STATES + [None]]
        lines.append(truth.ljust(width) + "".join(str(v).rjust(14) for v in row))
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--corpus", required=True, type=pathlib.Path)
    ap.add_argument("--split", choices=["dev", "heldout", "all"], default="dev")
    ap.add_argument("--out", type=pathlib.Path, help="write the numbers as JSON")
    ap.add_argument("--errors", type=int, default=0, help="print up to N mismatching pages")
    args = ap.parse_args()

    labels = json.loads((args.corpus / "labels.json").read_text())["files"]
    detail_path = args.corpus / "labels_detail.json"
    detail = json.loads(detail_path.read_text())["files"] if detail_path.exists() else {}

    conf = collections.Counter()
    by_variant = collections.defaultdict(lambda: [0, 0])  # variant -> [correct, total]
    script_ok = script_total = 0
    script_conf = collections.Counter()
    legacy = collections.Counter()  # tp fn fp tn, plus unreliable recall
    inc_ok = inc_total = 0
    prod_ok = prod_total = 0
    conflicts = 0
    jpeg_pairs = []
    errors = []
    degraded_docs = 0
    t_total = 0.0
    pages_total = 0
    failures = []

    names = sorted(n for n in labels if in_split(n, args.split))
    for name in names:
        data = (args.corpus / name).read_bytes()
        t0 = time.perf_counter()
        try:
            report = json.loads(pagetriage_py.analyse_pdf_structure(data))
        except ValueError as e:
            failures.append((name, str(e)))
            continue
        t_total += time.perf_counter() - t0
        lab = labels[name]
        det = detail.get(name, {})
        degraded_docs += bool(report["degraded"])

        inc_total += 1
        inc_ok += report["document"]["incremental_updates"] == lab["incremental_updates"]
        if "producer" in lab:
            prod_total += 1
            prod_ok += report["document"]["producer"] == lab["producer"]

        if len(report["pages"]) != len(lab["pages"]):
            failures.append((name, f"{len(report['pages'])} pages reported, {len(lab['pages'])} labelled"))
            continue
        for i, (pl, pr) in enumerate(zip(lab["pages"], report["pages"])):
            pages_total += 1
            pdet = det.get("pages", [{}] * len(lab["pages"]))[i] if det else {}
            variant = pdet.get("variant", pdet.get("plan", "?"))
            tl = pr["text_layer"]
            pred = tl["state"] if tl else None
            conf[(pl["text_layer"], pred)] += 1
            ok = pred == pl["text_layer"]
            by_variant[variant][0] += ok
            by_variant[variant][1] += 1
            if not ok and len(errors) < args.errors:
                errors.append((name, i, pl["text_layer"], pred, variant, tl["confidence"] if tl else None))

            sc = pr["script"]
            dom = sc["dominant"] if sc else None
            script_total += 1
            script_ok += dom == pl["script"]
            if dom != pl["script"]:
                script_conf[(pl["script"], dom)] += 1

            flagged = bool(sc and sc["legacy_encoded"])
            if pl["legacy_font"]:
                legacy["tp" if flagged else "fn"] += 1
                if tl and tl["reliable"] is False:
                    legacy["unreliable"] += 1
            else:
                legacy["fp" if flagged else "tn"] += 1
            conflicts += len(pr["conflicts"])

            # JPEG quality against the generator's own record of the encoder setting.
            for im_label, im in zip(pdet.get("images", []), pr["images"]):
                if im_label.get("filter") == "DCTDecode" and im["jpeg"]:
                    jpeg_pairs.append((im_label["quality"], im["jpeg"]["luma_quality"], im["jpeg"]["ijg_scaled"]))

    total = sum(conf.values())
    correct = sum(v for (t, p), v in conf.items() if t == p)
    print(f"split={args.split}  documents={len(names)}  pages={pages_total}  parse failures={len(failures)}  degraded docs={degraded_docs}")
    for f in failures:
        print("  FAILED", f)
    print(f"\nM1 page-level text-layer state accuracy: {correct}/{total} = {pct(correct, total)}  (target >= 99%)")
    print(confusion_table(conf))
    print("\nper label:")
    for s in STATES:
        tp = conf[(s, s)]
        n_true = sum(conf[(s, p)] for p in STATES + [None])
        n_pred = sum(conf[(t, s)] for t in STATES)
        print(f"  {s:<28} recall {pct(tp, n_true):>7} ({tp}/{n_true})   precision {pct(tp, n_pred):>7} ({tp}/{n_pred})")
    print("\nby generator variant (accuracy):")
    for v, (c, n) in sorted(by_variant.items()):
        print(f"  {v:<14} {c}/{n} = {pct(c, n)}")

    print(f"\nM2 script (dominant script of the text layer): {script_ok}/{script_total} = {pct(script_ok, script_total)}")
    for (t, p), v in sorted(script_conf.items(), key=lambda kv: -kv[1])[:8]:
        print(f"  label {t!s:<11} predicted {p!s:<11} x{v}")
    tp, fn, fp, tn = (legacy[k] for k in ("tp", "fn", "fp", "tn"))
    print(f"\nM2 legacy-encoded Devanagari: flagged {tp}/{tp + fn} labelled pages = {pct(tp, tp + fn)} (target >= 95%); "
          f"false positives {fp}/{fp + tn} = {pct(fp, fp + tn)}; text_layer.reliable=false on {legacy['unreliable']}/{tp + fn}")
    print(f"consistency conflicts reported: {conflicts}")

    print(f"\nM12 incremental_updates exact: {inc_ok}/{inc_total} = {pct(inc_ok, inc_total)};  producer exact: {prod_ok}/{prod_total} = {pct(prod_ok, prod_total)}")
    if jpeg_pairs:
        exact = sum(1 for q, e, _ in jpeg_pairs if q == e)
        within = sum(1 for q, e, _ in jpeg_pairs if abs(q - e) <= 1)
        scaled = sum(1 for _, _, s in jpeg_pairs if s)
        print(f"M12 JPEG quality estimate vs encoder setting: exact {exact}/{len(jpeg_pairs)}, within 1: {within}/{len(jpeg_pairs)}, "
              f"table recognised as IJG-scaled {scaled}/{len(jpeg_pairs)}")
    print(f"time: {1000 * t_total / max(1, len(names)):.1f} ms per document, {1000 * t_total / max(1, pages_total):.2f} ms per page (Python binding, release build)")

    if errors:
        print("\nmismatching pages (document, 0-based page, label, predicted, variant, confidence):")
        for e in errors:
            print("  ", e)

    if args.out:
        args.out.write_text(json.dumps({
            "split": args.split, "documents": len(names), "pages": pages_total,
            "state_accuracy": correct / total if total else None,
            "confusion": {f"{t}->{p}": v for (t, p), v in sorted(conf.items(), key=str)},
            "by_variant": {k: v for k, v in by_variant.items()},
            "script_accuracy": script_ok / script_total if script_total else None,
            "legacy": dict(legacy),
            "incremental_updates_exact": inc_ok / inc_total if inc_total else None,
        }, indent=1, sort_keys=True) + "\n")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
