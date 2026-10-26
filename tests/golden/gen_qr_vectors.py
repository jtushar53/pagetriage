#!/usr/bin/env python3
"""Write tests/golden/qr_vectors.json: real QR codes (module matrices) of several versions, error-
correction levels, mask patterns and segment types, made with the `qrcode` Python package, for the
decoder's tests. Development-time only (pip install qrcode); the JSON is committed, so the Rust tests
need no Python. The texts are synthetic.

    python3 tests/golden/gen_qr_vectors.py
"""
import json
import pathlib

import qrcode

OUT = pathlib.Path(__file__).resolve().parent / "qr_vectors.json"
LEVELS = {"L": qrcode.constants.ERROR_CORRECT_L, "M": qrcode.constants.ERROR_CORRECT_M,
          "Q": qrcode.constants.ERROR_CORRECT_Q, "H": qrcode.constants.ERROR_CORRECT_H}

TEXTS = {
    "numeric": "0123456789012345678901234567890123456789",
    "alnum": "HTTPS://EXAMPLE.INVALID/STAMP/IN-DL12345678901234 TOTAL 100",
    "url": "https://example.invalid/verify?cert=IN-MH0000000000001&amount=100",
    "utf8": "Stamp duty paid: ₹ 500 — Pune",
}
# (version, level, mask, text key): versions from 1 to 27, every level, every mask, every segment type.
CASES = [
    (1, "L", 0, "numeric"), (1, "M", 1, "alnum"), (1, "Q", 2, "numeric"), (1, "H", 3, "alnum"),
    (2, "L", 4, "url"), (2, "M", 5, "alnum"), (3, "Q", 6, "url"), (3, "H", 7, "numeric"),
    (4, "L", 0, "url"), (4, "M", 1, "utf8"), (5, "Q", 2, "url"), (5, "H", 3, "utf8"),
    (6, "L", 4, "url"), (7, "M", 5, "url"), (7, "Q", 6, "utf8"), (7, "H", 7, "alnum"),
    (10, "L", 0, "url"), (10, "M", 1, "url"), (10, "H", 2, "url"),
    (14, "Q", 3, "url"), (20, "M", 4, "url"), (27, "L", 5, "url"),
]


def main():
    out = []
    for version, level, mask, key in CASES:
        text = TEXTS[key]
        while True:  # the longest prefix that fits this version and level
            qr = qrcode.QRCode(version=version, error_correction=LEVELS[level], box_size=1, border=0, mask_pattern=mask)
            qr.add_data(text)
            try:
                qr.make(fit=False)
                break
            except qrcode.exceptions.DataOverflowError:
                text = text[:-1]
        rows = ["".join("1" if m else "0" for m in row) for row in qr.get_matrix()]
        assert len(rows) == 17 + 4 * version
        out.append({"version": version, "level": level, "mask": mask, "kind": key, "text": text, "rows": rows})
    OUT.write_text(json.dumps(out, separators=(",", ":"), ensure_ascii=False) + "\n", encoding="utf-8")
    print("wrote", len(out), "codes to", OUT)


if __name__ == "__main__":
    main()
