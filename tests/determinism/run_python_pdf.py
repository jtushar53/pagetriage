#!/usr/bin/env python3
"""Run the golden PDFs through the Python binding and require byte-identical structure reports.

Needs `pagetriage_py` installed (scripts/determinism.sh does `maturin develop`). Run with `-I`.
"""
import hashlib
import pathlib
import sys

import pagetriage_py

GOLDEN = pathlib.Path(__file__).resolve().parents[1] / "golden"


def main():
    inputs = sorted((GOLDEN / "pdf").glob("*.pdf"))
    if len(inputs) < 6:
        sys.exit(f"expected at least 6 golden PDFs, found {len(inputs)}")

    failures = 0
    for path in inputs:
        actual = pagetriage_py.analyse_pdf_structure(path.read_bytes()) + "\n"
        expected = (GOLDEN / "pdf_expected" / f"{path.stem}.json").read_text(encoding="utf-8")
        digest = hashlib.sha256(actual.encode()).hexdigest()[:16]
        if actual == expected:
            print(f"ok   python {path.stem:<20} sha256={digest}")
        else:
            failures += 1
            at = next((i for i, (a, b) in enumerate(zip(actual, expected)) if a != b), min(len(actual), len(expected)))
            print(f"FAIL python {path.stem}: first difference at byte {at}")
            print(f"  actual  : ...{actual[max(0, at - 30) : at + 40]!r}")
            print(f"  expected: ...{expected[max(0, at - 30) : at + 40]!r}")

    try:
        pagetriage_py.analyse_pdf_structure(b"definitely not a pdf")
    except ValueError:
        print("ok   python non-PDF input raises ValueError")
    else:
        failures += 1
        print("FAIL python: non-PDF input did not raise")

    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
