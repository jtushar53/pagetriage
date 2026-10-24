#!/usr/bin/env python3
"""The structural layer's known wrong or doubtful answers, built in front of you.

    python -I bench/tools/structure_gallery.py

Each case is a one-page synthetic PDF made with the golden-fixture assembler (tests/golden/gen_pdfs.py)
and analysed through the Python binding. The failure galleries in docs/detectors/text_layer.md
quote this output. A case that starts giving a different answer is a behaviour change to review.
"""
import importlib.util
import json
import pathlib
import sys

import pagetriage_py

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("gen", ROOT / "tests/golden/gen_pdfs.py")
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
FULL = g.FULL
TXT = "BT /F1 10 Tf 72 700 Td (Words of text that sit on the page) Tj ET\n"
RES_IMG = "/Font << /F1 100 0 R >> /XObject << /Im1 101 0 R >>"
def build(content, resources=RES_IMG, extra=None):
    pdf = g.Pdf()
    def ex(p):
        p.obj(100, g.HELV); g.gray_image(101, p, 24, 32)
        if extra: extra(p)
    info = g.pages_doc(pdf, [(content, resources)], extra_objects=ex)
    return pdf.finish(1, 150, info)
cases = {
 "white text over scan (1 g, mode 0)": FULL + "1 g\n" + TXT,
 "text clipped away (0 0 0 0 re W n)": "q 0 0 0 0 re W n\n" + TXT + "Q\n",
 "text far off the page (x=-5000)": "BT /F1 10 Tf -5000 700 Td (Words of text that sit on the page) Tj ET\n",
 "invisible text under a scan covering 81%": "q 560 0 0 700 26 46 cm /Im1 Do Q\nBT 3 Tr /F1 10 Tf 72 700 Td (Words of text that sit on the page) Tj ET\n",
 "scan rotated 90 degrees": "q 0 792 -612 0 612 0 cm /Im1 Do Q\n",
 "scan painted with a pattern fill, not Do": "/Pattern cs /P1 scn 0 0 612 792 re f\n",
 "full-page inline image": "q 612 0 0 792 0 0 cm BI /W 4 /H 4 /BPC 8 /CS /G ID " + "\x80"*16 + "\nEI Q\n",
 "text drawn as Type3 bitmap font": "BT /F3 10 Tf 72 700 Td (abcdef ghij) Tj ET\n",
 "invisible text + visible stamp text on a scan": FULL + "BT 3 Tr /F1 10 Tf 72 700 Td (Words of text that sit on the page) Tj ET\nBT 0 Tr /F1 12 Tf 400 100 Td (Seal 1234) Tj ET\n",
}
def extra3(p):
    p.stream(102, "", "0 0 500 700 d1\n")
    p.obj(103, "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 102 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [500] >>")
for name, content in cases.items():
    res = RES_IMG
    ex = None
    if "Type3" in name:
        res = "/Font << /F3 103 0 R >>"; ex = extra3
    if "pattern" in name:
        res = "/Pattern << /P1 104 0 R >>"
        ex = lambda p: p.stream(104, "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>", "0 g 0 0 5 5 re f")
    r = json.loads(pagetriage_py.analyse_pdf_structure(build(content, res, ex)))
    p = r["pages"][0]; t = p["text_layer"]; f = p["features"]
    print(f"{name:<50} -> {t['state']:<26} conf {t['confidence']:.3f}  glyphs {f['glyphs']} hidden(mode {f['glyphs_invisible_mode']}, under {f['glyphs_under_image']}) cover {f['image_cover']:.2f} paint {f['paint_ops']}")
