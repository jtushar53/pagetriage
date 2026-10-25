#!/usr/bin/env python3
"""Run the golden inputs through the Python binding and require byte-identical output.

Needs `pagetriage_py` installed (scripts/determinism.sh does `maturin develop`). Run with `-I`.
"""
import hashlib
import pathlib
import sys

import pagetriage_py

GOLDEN = pathlib.Path(__file__).resolve().parents[1] / "golden"
OPTS = '{"debug":true}'
LAYOUT_OPTS = '{"debug":true,"detectors":["layout"]}'
DPI = 150


def read_pgm(path):
    data = path.read_bytes()
    pos, tokens = 0, []
    while len(tokens) < 4:
        while data[pos : pos + 1].isspace():
            pos += 1
        start = pos
        while not data[pos : pos + 1].isspace():
            pos += 1
        tokens.append(data[start:pos].decode())
    pos += 1
    assert tokens[0] == "P5" and tokens[3] == "255", f"{path}: unsupported PGM"
    w, h = int(tokens[1]), int(tokens[2])
    pixels = data[pos:]
    assert len(pixels) == w * h, f"{path}: pixel count mismatch"
    return w, h, pixels


def main():
    inputs = sorted([*(GOLDEN / "inputs").glob("*.pgm"), *(GOLDEN / "quality_inputs").glob("*.pgm")])
    if len(inputs) < 4:
        sys.exit(f"expected at least 4 golden inputs, found {len(inputs)}")

    failures = 0
    for path in inputs:
        w, h, gray = read_pgm(path)
        actual = pagetriage_py.analyse_page(gray, w, h, DPI, None, None, OPTS) + "\n"
        expected = (GOLDEN / "expected" / f"{path.stem}.json").read_text(encoding="utf-8")
        digest = hashlib.sha256(actual.encode()).hexdigest()[:16]
        if actual == expected:
            print(f"ok   python {path.stem:<16} sha256={digest}")
        else:
            failures += 1
            at = next((i for i, (a, b) in enumerate(zip(actual, expected)) if a != b), min(len(actual), len(expected)))
            print(f"FAIL python {path.stem}: first difference at byte {at}")
            print(f"  actual  : ...{actual[max(0, at - 30) : at + 40]!r}")
            print(f"  expected: ...{expected[max(0, at - 30) : at + 40]!r}")

    # The opt-in layout detector, on the layout pages and the quality pages.
    layout_inputs = sorted([*(GOLDEN / "layout_inputs").glob("*.pgm"), *(GOLDEN / "quality_inputs").glob("*.pgm")])
    if len(layout_inputs) < 9:
        sys.exit(f"expected at least 9 layout inputs, found {len(layout_inputs)}")
    for path in layout_inputs:
        w, h, gray = read_pgm(path)
        actual = pagetriage_py.analyse_page(gray, w, h, DPI, None, None, LAYOUT_OPTS) + "\n"
        expected = (GOLDEN / "layout_expected" / f"{path.stem}.json").read_text(encoding="utf-8")
        if actual == expected:
            print(f"ok   python layout {path.stem}")
        else:
            failures += 1
            print(f"FAIL python layout {path.stem}")

    try:
        pagetriage_py.analyse_page(b"\x00" * 3, 2, 2, DPI)
    except ValueError:
        print("ok   python bad buffer raises ValueError")
    else:
        failures += 1
        print("FAIL python: inconsistent buffer length did not raise")

    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
