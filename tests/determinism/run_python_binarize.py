#!/usr/bin/env python3
"""Binarise the golden inputs through the Python binding and require the golden SHA-256 hashes.

Needs `pagetriage_py` installed (scripts/determinism.sh does `maturin develop`). Run with `-I`.
"""
import hashlib
import json
import pathlib
import sys

import pagetriage_py

GOLDEN = pathlib.Path(__file__).resolve().parents[1] / "golden"


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
    cases = json.loads((GOLDEN / "binarize_cases.json").read_text())
    expected = json.loads((GOLDEN / "binarize_expected.json").read_text())
    failures = 0
    inputs = sorted([*(GOLDEN / "inputs").glob("*.pgm"), *(GOLDEN / "binarize_inputs").glob("*.pgm")])
    if len(inputs) < 6:
        sys.exit(f"expected at least 6 golden inputs, found {len(inputs)}")
    for path in inputs:
        w, h, gray = read_pgm(path)
        bad = []
        for case in cases:
            ink = pagetriage_py.binarize(gray, w, h, case["method"], case["params"] or None)
            if hashlib.sha256(ink).hexdigest() != expected[path.stem][case["name"]]:
                bad.append(case["name"])
        if bad:
            failures += 1
            print(f"FAIL python binarize {path.stem}: {', '.join(bad)}")
        else:
            print(f"ok   python binarize {path.stem:<16} {len(cases)} methods")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
