#!/usr/bin/env python3
"""Deterministic synthetic PDF corpus with ground truth, for scoring the PDF structural layer.

    python -I bench/synth/make_pdf_corpus.py                       # 300 pages in ~60 PDFs -> bench/data/synth_pdf
    python -I bench/synth/make_pdf_corpus.py --pages 60 --seed 7 --out /tmp/corpus --force

The same `--seed` and `--pages` give byte-identical files and labels on the same machine and the
same Pillow, zlib and libjpeg builds (JPEG and Flate encoders are deterministic, but a different
library version may encode the same pixels to different bytes). Nothing reads the clock, the
environment or the filesystem except the output directory. Needs Pillow and numpy; qpdf and
poppler are only used by the tests, never by the generator.

Everything is synthetic. Latin text is built from legal-sounding templates with generic
placeholders; Indic text is meaningless syllable soup; scans are rendered from that text with
Pillow (bundled font for Latin, procedural pseudo-glyphs for Indic scripts) and degraded with skew,
blur, shading, noise and speckle. No real document, name or address is used.

Output
------
`labels.json` (the schema agreed with the structural layer; do not change it)::

    {"files": {"doc_001.pdf": {"producer": str, "incremental_updates": int,
        "pages": [{"text_layer": "native_text|raster_only|invisible_ocr_under_image|mixed|vector_only|empty",
                   "script": "latin|devanagari|bengali|gurmukhi|gujarati|tamil|telugu|kannada|malayalam|arabic|none",
                   "legacy_font": bool}]}}}

`labels_detail.json` is a sidecar, not part of the agreed schema: per file the persona, xref
style, producer history and revisions; per page the expected extracted text, page variant, image
encoding, page size. The tests and any scorer that wants more than the labels read it.

Label semantics
---------------
* `producer` is the `/Producer` of the *final* `/Info` dictionary (after incremental updates), or
  "" when there is none or it is empty. The original producer is in the sidecar.
* Labels describe the *final* state of each page, after any incremental update.
* `incremental_updates` is the number of appended revisions; the file has that many plus one `%%EOF`.
* `script` is the script of the text layer, or the script a legacy font stands for. Pages without a
  text layer (raster_only, vector_only, empty) are `none`: a structural reader cannot know the
  script of pixels. The script of image content is in the sidecar (`content_script`).
* `invisible_ocr_under_image`: a full-page image plus text that is not meant to be seen. Mostly
  render mode 3 (`3 Tr`) after the image; some pages put the text stream first; about one in ten
  uses visible text (`0 Tr`) painted under an opaque image (sidecar `variant: "occluded"`). Exclude
  that variant if you only want render-mode-3 behaviour.
* `mixed`: visible native text together with a partial-page image.
* `legacy_font`: the text uses Latin code points under a font named like a pre-Unicode Indian font
  (Kruti Dev 010, DV-TTYogesh, Shree-Lipi, Chanakya, APS-Dev, Walkman-Chanakya), so extraction gives
  gibberish. Such pages are labelled script `devanagari`.

Page kinds, producers and what is NOT covered: see bench/README.md. In short, this corpus was
written by the team that writes the detector, to the team's own idea of what scans and producers
look like. It is weak evidence of real-world accuracy.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import numpy as np  # noqa: E402

import pdfparts as pp  # noqa: E402
from pagekinds import Ctx, Style, build_page  # noqa: E402
from pdfio import PdfFile, fmt, hexstr, lit, name, stream_body, text_string  # noqa: E402
from pdfparts import PagePlan, FontPool  # noqa: E402

GENERATOR_VERSION = 1
DEFAULT_OUT = Path(__file__).resolve().parents[1] / "data" / "synth_pdf"


def sub_rng(seed: int, *parts) -> random.Random:
    return random.Random("pagetriage-synth/%d/%s" % (seed, "/".join(str(p) for p in parts)))


def sub_np(seed: int, *parts) -> np.random.Generator:
    digest = hashlib.sha256(("pagetriage-synth-np/%d/%s" % (seed, "/".join(str(p) for p in parts))).encode()).digest()
    return np.random.Generator(np.random.PCG64(int.from_bytes(digest[:8], "big")))


# ---- personas -----------------------------------------------------------------------------------

SCRIPT_MIX = {"latin": 50, "devanagari": 22, "bengali": 5, "gurmukhi": 3, "gujarati": 4, "tamil": 4,
              "telugu": 4, "kannada": 3, "malayalam": 3, "arabic": 2}
OCR_SCRIPT_MIX = {"latin": 55, "devanagari": 30, "bengali": 4, "gurmukhi": 3, "gujarati": 3, "tamil": 3,
                  "telugu": 2}


@dataclass
class Persona:
    key: str
    producers: list[str]
    creators: list[str]
    kinds: dict[str, float]
    style: Style
    scripts: dict[str, float] = field(default_factory=lambda: SCRIPT_MIX)
    legacy_p: float = 0.0
    xmp: bool = False
    xref_stream_p: float = 0.0
    versions: list[str] = field(default_factory=lambda: ["1.4", "1.5", "1.6", "1.7"])
    sig: list[PagePlan] = field(default_factory=list)   # pages the persona's first document must contain


def P(kind, script="latin", legacy=False, variant=None, enc=None):
    return PagePlan(kind, script, legacy, variant, enc)


MIX_SCAN = {"dct_gray": 25, "dct_rgb": 18, "flate_gray": 8, "flate_pred_gray": 6, "flate_rgb": 3,
            "flate_1bit": 12, "ccitt": 20}

PERSONAS = [
    Persona("word", ["Microsoft® Word 2016", "Microsoft® Word for Microsoft 365", "Microsoft® Word 2010",
                     "Microsoft: Print To PDF"],
            ["Microsoft® Word 2016", "Microsoft® Word for Microsoft 365", "PScript5.dll Version 5.2.2"],
            {"native": 78, "mixed": 10, "vector": 4, "empty": 4, "raster": 4},
            Style(["calibri", "arial", "tnr", "verdana"], {"dct_rgb": 3, "flate_gray": 1}, "word_simple", 0.9, True),
            xmp=True, xref_stream_p=0.3,
            sig=[P("native"), P("native", "devanagari"), P("native", "bengali")]),
    Persona("ocrmypdf", ["OCRmyPDF 15.4.4 / Tesseract OCR-PDF 5.3.4", "OCRmyPDF 15.2.0 / Tesseract OCR-PDF 5.3.3"],
            ["", "img2pdf 0.5.1"],
            {"ocr": 80, "raster": 10, "empty": 5, "native": 5},
            Style(["tnr", "arial"], MIX_SCAN, "glyphless", 1.0),
            scripts=OCR_SCRIPT_MIX, xmp=True, xref_stream_p=0.7, versions=["1.5", "1.6", "1.7"],
            sig=[P("ocr"), P("ocr", "devanagari"), P("raster", "latin", variant="text"), P("ocr", "latin")]),
    Persona("scanner", ["KONICA MINOLTA bizhub C250i", "Xerox WorkCentre 7855", "HP Digital Sending Device",
                        "Canon iR-ADV C5535", "RICOH MP C3004", "EPSON ES-580W", "SCAN2PDF FW 3.2.1"],
            ["", "KONICA MINOLTA bizhub C250i", "Xerox WorkCentre 7855"],
            {"raster": 100},
            Style(["helv"], {"dct_gray": 28, "dct_rgb": 14, "ccitt": 28, "flate_1bit": 8, "flate_gray": 6,
                             "flate_pred_gray": 4}, "glyphless", 0.5),
            scripts=OCR_SCRIPT_MIX, versions=["1.3", "1.4", "1.5"],
            sig=[P("raster", "latin", enc="ccitt"), P("raster", "devanagari", variant="strips", enc="dct_rgb"),
                 P("raster", "latin", enc="flate_pred_gray"), P("raster", "latin", variant="blank", enc="flate_1bit")]),
    Persona("legacy_hindi", ["Acrobat Distiller 8.1.0 (Windows)", "Adobe PDF Library 9.0", "PDFCreator 1.7.3"],
            ["PScript5.dll Version 5.2.2", "CorelDRAW", "Adobe PageMaker 7.0"],
            {"native": 72, "ocr": 12, "raster": 5, "empty": 5, "vector": 6},
            Style(["times", "helv"], MIX_SCAN, "word_simple", 0.8), legacy_p=0.9, versions=["1.3", "1.4", "1.5"],
            sig=[P("native", "devanagari", True), P("native", "devanagari", True, "english_heading"),
                 P("ocr", "devanagari", True)]),
    Persona("pdftex", ["pdfTeX-1.40.25"], ["LaTeX with hyperref"],
            {"native": 78, "vector": 12, "empty": 5, "mixed": 5},
            Style(["cmr"], {"dct_gray": 1}, "word_simple", 1.0), scripts={"latin": 1}, versions=["1.5", "1.7"],
            xref_stream_p=0.8,
            sig=[P("native"), P("vector", variant="outlined"), P("empty", variant="bt_et")]),
    Persona("abbyy", ["ABBYY FineReader PDF 15", "ABBYY FineReader 11.0", "ABBYY FineReader Engine 12"],
            ["ABBYY FineReader PDF 15", "", "Canon iR-ADV C5535"],
            {"ocr": 52, "mixed": 18, "raster": 10, "native": 12, "vector": 5, "empty": 3},
            Style(["arial", "tnr"], MIX_SCAN, "word_simple", 1.0), scripts=OCR_SCRIPT_MIX, xmp=True,
            xref_stream_p=0.2,
            sig=[P("ocr"), P("mixed"), P("ocr", "devanagari"), P("mixed", "devanagari")]),
    Persona("adobe_pc", ["Adobe Acrobat 9.0 Paper Capture Plug-in", "Adobe Acrobat Pro 10.1.16 Paper Capture Plug-in"],
            ["Adobe Acrobat 9.0", "Adobe Acrobat Pro 10.1.16", ""],
            {"ocr": 80, "raster": 10, "empty": 5, "vector": 5},
            Style(["tnr", "arial"], MIX_SCAN, "word_simple", 0.9), scripts=OCR_SCRIPT_MIX, versions=["1.5", "1.6"],
            sig=[P("ocr"), P("ocr", "devanagari"), P("ocr", "gujarati")]),
    Persona("chrome", ["Skia/PDF m117", "Skia/PDF m120", "Skia/PDF m110"], ["Chromium", "Mozilla/5.0 (X11; Linux x86_64)"],
            {"native": 84, "vector": 8, "empty": 4, "mixed": 4},
            Style(["arial", "tnr", "verdana"], {"dct_rgb": 1}, "word_simple", 1.0, True), versions=["1.4"],
            sig=[P("native", "gujarati"), P("native", "tamil"), P("native", "arabic")]),
    Persona("ghostscript", ["GPL Ghostscript 9.55.0", "GPL Ghostscript 10.02.1"],
            ["PScript5.dll Version 5.2.2", "Microsoft Word", ""],
            {"raster": 38, "native": 30, "ocr": 17, "vector": 10, "empty": 5},
            Style(["helv", "times", "arial"], MIX_SCAN, "word_simple", 1.0), versions=["1.4", "1.5", "1.6"],
            sig=[P("raster"), P("native"), P("ocr")]),
    Persona("empty_producer", [""], ["", "scan"],
            {"native": 30, "raster": 20, "ocr": 10, "mixed": 10, "vector": 15, "empty": 15},
            Style(["helv", "times", "courier"], MIX_SCAN, "glyphless", 0.5), versions=["1.3", "1.4", "1.7"],
            sig=[P("empty"), P("vector"), P("native"), P("empty", variant="empty_array")]),
    Persona("itext", ["iText® 5.5.13 ©2000-2018 iText Group NV (AGPL-version)", "iTextSharp 5.5.13",
                      "ReportLab PDF Library - www.reportlab.com"],
            ["", "iText® 5.5.13"],
            {"native": 84, "vector": 8, "empty": 3, "mixed": 5},
            Style(["helv", "times", "courier"], {"dct_gray": 2, "flate_gray": 1}, "word_simple", 0.6),
            versions=["1.4", "1.5", "1.6"],
            sig=[P("native", "telugu"), P("native", "kannada"), P("native", "malayalam"), P("native", "gurmukhi")]),
    Persona("print_driver", ["Foxit PhantomPDF Printer Version 9.7.1.2", "Nitro PDF Creator (Pro 8)", "PDF24 Creator",
                             "PDFsharp 1.50.5147 (www.pdfsharp.com)"],
            ["", "PDF24 Creator"],
            {"native": 50, "mixed": 10, "raster": 20, "ocr": 10, "vector": 5, "empty": 5},
            Style(["arial", "tnr", "courier"], MIX_SCAN, "word_simple", 0.8), versions=["1.4", "1.5", "1.7"],
            sig=[P("native"), P("raster"), P("mixed")]),
]

EDITORS = ["Adobe Acrobat Pro DC (64-bit) 22.3.20314", "Foxit PDF Editor 12.1.3", "PDF-XChange Editor 9.5.366",
           "Microsoft: Print To PDF"]

COUNT_VALUES = [1, 2, 3, 4, 5, 6, 8, 10, 12, 15]
COUNT_WEIGHTS = [12, 15, 15, 12, 10, 8, 8, 8, 6, 6]
REV_SCHEDULE = [0, 1, 0, 2, 0, 3, 1, 0, 2, 0]
FLAVOURS = ["annot", "replace", "mixed"]
TRANSITIONS = {"native": "empty", "empty": "native", "raster": "ocr", "ocr": "raster", "vector": "empty",
               "mixed": "native"}


# ---- planning -----------------------------------------------------------------------------------

@dataclass
class DocSpec:
    index: int
    persona: Persona
    n_pages: int
    n_revs: int
    flavour: str
    sig: list[PagePlan]


def plan_corpus(seed: int, total: int) -> list[DocSpec]:
    rng = sub_rng(seed, "plan")
    n_docs = max(1, min(total, round(total / 5)))
    counts = [rng.choices(COUNT_VALUES, weights=COUNT_WEIGHTS)[0] for _ in range(n_docs)]
    sigs = [PERSONAS[i % len(PERSONAS)].sig if i < len(PERSONAS) else [] for i in range(n_docs)]
    mins = [max(1, len(s)) for s in sigs]
    counts = [max(c, m) for c, m in zip(counts, mins)]
    while sum(counts) > total:
        cand = [i for i in range(n_docs) if counts[i] > mins[i]]
        if not cand:  # corpus too small to hold every signature page: trim signatures from the back
            i = max(range(n_docs), key=lambda k: mins[k])
            mins[i] -= 1
            counts[i] = min(counts[i], mins[i])
            continue
        i = max(cand, key=lambda k: (counts[k] - mins[k], -k))
        counts[i] -= 1
    while sum(counts) < total:
        counts[rng.randrange(n_docs)] += 1
    docs, revised = [], 0
    for i in range(n_docs):
        n_revs = REV_SCHEDULE[i % len(REV_SCHEDULE)]
        flavour = ""
        if n_revs:
            flavour = FLAVOURS[revised % len(FLAVOURS)]
            revised += 1
        docs.append(DocSpec(i, PERSONAS[i % len(PERSONAS)], counts[i], n_revs, flavour, sigs[i][:counts[i]]))
    return docs


def transition_plan(plan: PagePlan) -> PagePlan:
    """The kind a page turns into when an update replaces its content stream."""
    new_kind = TRANSITIONS[plan.kind]
    script = plan.script
    if plan.kind == "empty" or (script == "arabic" and new_kind not in ("native", "mixed")):
        script = "latin"
    return PagePlan(new_kind, script)


def plan_pages(rng: random.Random, spec: DocSpec) -> list[PagePlan]:
    pers = spec.persona
    plans = list(spec.sig)
    while len(plans) < spec.n_pages:
        kind = rng.choices(list(pers.kinds), weights=list(pers.kinds.values()))[0]
        mix = pers.scripts if kind in ("native", "mixed") else {k: v for k, v in pers.scripts.items() if k != "arabic"}
        script = rng.choices(list(mix), weights=list(mix.values()))[0]
        legacy = kind in ("native", "ocr") and rng.random() < pers.legacy_p
        variant = None
        if legacy:
            script = "devanagari"
            if kind == "native" and rng.random() < 0.3:
                variant = "english_heading"
        plans.append(PagePlan(kind, script, legacy, variant))
    if not spec.sig:
        return plans
    rng.shuffle(plans)
    return plans


# ---- document assembly --------------------------------------------------------------------------

@dataclass
class PageRec:
    num: int
    entries: dict[str, str]
    plan: PagePlan
    build: pp.PageBuild
    size: tuple[float, float]


def pdf_date(rng: random.Random, after: tuple | None = None) -> tuple[str, tuple]:
    if after:
        y, mo, d, hh = after[0], after[1], after[2], after[3]
        d = min(28, d + rng.randint(0, 20))
        t = (y, mo, d, hh, rng.randint(0, 59), rng.randint(0, 59))
    else:
        t = (rng.randint(2006, 2025), rng.randint(1, 12), rng.randint(1, 28), rng.randint(0, 23),
             rng.randint(0, 59), rng.randint(0, 59))
    return "D:%04d%02d%02d%02d%02d%02d+05'30'" % t, t


def xml_escape(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def xmp_packet(producer: str, creator: str) -> bytes:
    body = ('<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>\n'
            '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">\n'
            '<rdf:Description rdf:about="" xmlns:pdf="http://ns.adobe.com/pdf/1.3/" '
            'xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">\n'
            f'<pdf:Producer>{xml_escape(producer)}</pdf:Producer>\n<xmp:CreatorTool>{xml_escape(creator)}</xmp:CreatorTool>\n'
            '<dc:format>application/pdf</dc:format>\n</rdf:Description></rdf:RDF></x:xmpmeta>\n'
            '<?xpacket end="w"?>')
    return body.encode("utf-8")


def page_body(rec: PageRec, parent: int) -> str:
    inner = " ".join(f"/{k} {v}" for k, v in rec.entries.items())
    return f"<< /Type /Page /Parent {parent} 0 R {inner} >>"


def box(w: float, h: float, ox: float = 0.0, oy: float = 0.0) -> str:
    return f"[{fmt(ox)} {fmt(oy)} {fmt(ox + w)} {fmt(oy + h)}]"


def make_annotations(ctx: Ctx, rng: random.Random, rec: PageRec, page_num: int) -> list[int]:
    f = ctx.f
    w, h = rec.size
    out = []
    for _ in range(rng.randint(1, 3)):
        kind = rng.choice(["Text", "Link", "Highlight", "FreeText", "Stamp"])
        x, y = rng.uniform(30, w - 150), rng.uniform(30, h - 80)
        rect = f"[{fmt(x)} {fmt(y)} {fmt(x + 120)} {fmt(y + 24)}]"
        if kind == "Text":
            body = (f"<< /Type /Annot /Subtype /Text /Rect {rect} /Contents {lit(b'Checked')} /T {lit(b'Reviewer')} "
                    f"/Name /Comment /F 4 /P {page_num} 0 R >>")
        elif kind == "Link":
            body = (f"<< /Type /Annot /Subtype /Link /Rect {rect} /Border [0 0 0] "
                    f"/A << /S /URI /URI {lit(b'https://example.org/')} >> /P {page_num} 0 R >>")
        elif kind == "Highlight":
            qp = f"[{fmt(x)} {fmt(y + 24)} {fmt(x + 120)} {fmt(y + 24)} {fmt(x)} {fmt(y)} {fmt(x + 120)} {fmt(y)}]"
            body = (f"<< /Type /Annot /Subtype /Highlight /Rect {rect} /QuadPoints {qp} /C [1 1 0] /F 4 "
                    f"/P {page_num} 0 R >>")
        else:
            helv = f.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
            label = "Approved" if kind == "Stamp" else "See note"
            # The appearance is graphics only: poppler's pdftotext extracts text from annotation
            # appearance streams, which would make the page's extracted text disagree with its label.
            ap = f.add_stream("/Type /XObject /Subtype /Form /BBox [0 0 120 24] "
                              f"/Resources << /Font << /Helv {helv} 0 R >> >>",
                              b"0 0 1 RG 1 w 1 1 118 22 re S 4 4 m 116 20 l S")
            sub = "FreeText" if kind == "FreeText" else "Stamp"
            extra = f"/DA {lit(b'/Helv 12 Tf 0 g')} " if kind == "FreeText" else f"/Name /Approved "
            body = (f"<< /Type /Annot /Subtype /{sub} /Rect {rect} /Contents {lit(label.encode())} {extra}"
                    f"/F 4 /AP << /N {ap} 0 R >> /P {page_num} 0 R >>")
        out.append(f.add(body))
    return out


def build_doc(seed: int, spec: DocSpec) -> tuple[bytes, dict, dict]:
    rng = sub_rng(seed, "doc", spec.index)
    pers = spec.persona
    f = PdfFile()
    pool = FontPool(f, rng)
    ctx = Ctx(f, pool, pers.style, seed)
    plans = plan_pages(rng, spec)
    size_name = pp.pick_size(rng)
    mixed_sizes = rng.random() < 0.12
    catalog, root = f.new(), f.new()
    recs: list[PageRec] = []
    for i, plan in enumerate(plans):
        num = f.new()
        nm = pp.pick_size(rng) if (mixed_sizes and rng.random() < 0.3) else size_name
        size = pp.PAGE_SIZES[nm]
        prng, pnp = sub_rng(seed, "page", spec.index, i, 0), sub_np(seed, "page", spec.index, i, 0)
        pb = build_page(ctx, plan, prng, pnp, size)
        media = pb.media or size
        entries = {"MediaBox": box(*media)}
        if prng.random() < 0.04:
            entries["MediaBox"] = box(*media, ox=-12.0, oy=-12.0)  # origin away from (0,0)
        if prng.random() < 0.05:
            entries["CropBox"] = box(media[0] - 12, media[1] - 12, 6, 6)
        entries.update(pb.entries)
        if pers.style.group:
            entries["Group"] = "<< /Type /Group /S /Transparency /CS /DeviceRGB >>"
        recs.append(PageRec(num, entries, plan, pb, media))
    pool.finalize()

    inherit = len({r.entries["MediaBox"] for r in recs}) == 1 and rng.random() < 0.2
    nested = len(recs) >= 7 and rng.random() < 0.4
    parent_of: dict[int, int] = {}
    if nested:
        groups = [recs[i:i + 4] for i in range(0, len(recs), 4)]
        mids = [f.new() for _ in groups]
        for mid, grp in zip(mids, groups):
            f.set(mid, f"<< /Type /Pages /Parent {root} 0 R /Kids [{' '.join(f'{r.num} 0 R' for r in grp)}] "
                       f"/Count {len(grp)} >>")
            for r in grp:
                parent_of[r.num] = mid
        kids = " ".join(f"{m} 0 R" for m in mids)
    else:
        kids = " ".join(f"{r.num} 0 R" for r in recs)
        for r in recs:
            parent_of[r.num] = root
    root_mb = ""
    if inherit:  # the page-tree root carries the MediaBox, as some producers do
        root_mb = " /MediaBox " + recs[0].entries["MediaBox"]
        for r in recs:
            del r.entries["MediaBox"]
    for r in recs:
        f.set(r.num, page_body(r, parent_of[r.num]))
    f.set(root, f"<< /Type /Pages /Kids [{kids}] /Count {len(recs)}{root_mb} >>")

    producer = rng.choice(pers.producers)
    creator = rng.choice(pers.creators)
    utf16 = rng.random() < 0.3
    cdate, ct = pdf_date(rng)
    mdate, _ = pdf_date(rng, ct)
    info_num = None
    if producer == "" and rng.random() < 0.5:
        info_entries = None
    else:
        info_entries = [f"/Producer {text_string(producer, utf16)}"]
        if creator:
            info_entries.append(f"/Creator {text_string(creator, utf16)}")
        if rng.random() < 0.5:
            info_entries.append(f"/Title {text_string('Scan_%04d' % rng.randint(1, 9999))}")
        info_entries += [f"/CreationDate ({cdate})", f"/ModDate ({mdate})"]
        info_num = f.add("<< " + " ".join(info_entries) + " >>")
    cat_extra = ""
    if pers.xmp and producer:
        xm = f.add_stream("/Type /Metadata /Subtype /XML", xmp_packet(producer, creator))
        cat_extra += f" /Metadata {xm} 0 R"
    if pers.style.group or rng.random() < 0.3:
        cat_extra += f" /Lang {lit(rng.choice([b'en-IN', b'en-US', b'hi-IN']))}"
    f.set(catalog, f"<< /Type /Catalog /Pages {root} 0 R{cat_extra} >>")

    id1 = hashlib.md5(f"{seed}/{spec.index}/id".encode()).hexdigest().upper()
    id2 = id1
    xref_stream = rng.random() < pers.xref_stream_p
    version = rng.choice(pers.versions)
    if xref_stream and version < "1.5":
        version = "1.5"

    def trailer(info: int | None, ident2: str) -> str:
        t = f"/Root {catalog} 0 R"
        if info is not None:
            t += f" /Info {info} 0 R"
        return t + f" /ID [<{id1}> <{ident2}>]"

    buf, prev = f.serialize(version, trailer(info_num, id2), xref_stream=xref_stream)
    revisions: list[dict] = []
    producers = [producer]
    if spec.n_revs:
        f.begin_update()
    for r in range(1, spec.n_revs + 1):
        rrng = sub_rng(seed, "rev", spec.index, r)
        kind = spec.flavour if spec.flavour != "mixed" else ("annot" if r % 2 else "replace")
        pages_touched: list[int] = []
        entry: dict = {"type": kind}
        if kind == "annot":
            pool_pages = recs[:]
            for rec in rrng.sample(pool_pages, min(len(pool_pages), rrng.randint(1, 2))):
                annots = make_annotations(ctx, rrng, rec, rec.num)
                old = rec.entries.get("Annots", "[]").strip("[]").strip()
                rec.entries["Annots"] = "[" + (old + " " if old else "") + " ".join(f"{a} 0 R" for a in annots) + "]"
                f.set(rec.num, page_body(rec, parent_of[rec.num]))
                pages_touched.append(recs.index(rec))
        else:
            for rec in rrng.sample(recs, min(len(recs), rrng.randint(1, 2))):
                idx = recs.index(rec)
                plan = rec.plan
                if rrng.random() < 0.3 and not plan.legacy and plan.kind in TRANSITIONS:
                    plan = transition_plan(plan)
                npool = FontPool(f, rrng)
                nctx = Ctx(f, npool, pers.style, seed)
                pb = build_page(nctx, plan, sub_rng(seed, "page", spec.index, idx, r), sub_np(seed, "page", spec.index, idx, r), rec.size)
                npool.finalize()
                for key in ("Contents", "Resources"):
                    rec.entries.pop(key, None)
                keep = {k: v for k, v in rec.entries.items()}
                rec.entries = {**keep, **{k: v for k, v in pb.entries.items() if k in ("Contents", "Resources")}}
                rec.plan, rec.build = plan, pb
                f.set(rec.num, page_body(rec, parent_of[rec.num]))
                pages_touched.append(idx)
        entry["pages"] = sorted(pages_touched)
        new_id2 = hashlib.md5(f"{seed}/{spec.index}/id/{r}".encode()).hexdigest().upper()
        change_info = (kind == "annot" and rrng.random() < 0.5) or rrng.random() < 0.15
        if change_info:
            editor = rrng.choice(EDITORS)
            t = pdf_date(rrng, ct)[0]
            body = [f"/Producer {text_string(editor)}"]
            if creator:
                body.append(f"/Creator {text_string(creator, utf16)}")
            body += [f"/CreationDate ({cdate})", f"/ModDate ({t})"]
            if info_num is not None and rrng.random() < 0.5:
                f.set(info_num, "<< " + " ".join(body) + " >>")
            else:
                info_num = f.add("<< " + " ".join(body) + " >>")
            producers.append(editor)
            entry["producer"] = editor
        revisions.append(entry)
        prev = f.append_update(buf, prev, trailer(info_num, new_id2), xref_stream=xref_stream)

    final_producer = producers[-1] if (info_num is not None or len(producers) > 1) else ""
    label = {"producer": final_producer, "incremental_updates": spec.n_revs,
             "pages": [dict(r.build.label) for r in recs]}
    detail = {"persona": pers.key, "pdf_version": version, "xref": "stream" if xref_stream else "table",
              "original_producer": producer, "producer_history": producers, "revisions": revisions,
              "nested_page_tree": nested, "inherited_mediabox": inherit,
              "pages": [dict(r.build.detail, plan=r.plan.kind, page_size_pt=[round(v, 2) for v in r.size],
                             rotate=int(r.entries.get("Rotate", 0))) for r in recs]}
    return bytes(buf), label, detail


def build_corpus(pages: int, seed: int) -> tuple[dict[str, bytes], dict, dict]:
    files: dict[str, bytes] = {}
    labels: dict[str, dict] = {}
    details: dict[str, dict] = {}
    for spec in plan_corpus(seed, pages):
        fname = "doc_%03d.pdf" % (spec.index + 1)
        data, label, detail = build_doc(seed, spec)
        files[fname], labels[fname], details[fname] = data, label, detail
    meta = {"generator": "bench/synth/make_pdf_corpus.py", "generator_version": GENERATOR_VERSION,
            "seed": seed, "pages": pages}
    return files, {"files": labels}, {**meta, "files": details}


def dumps(obj) -> bytes:
    return (json.dumps(obj, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode("ascii")


def write_corpus(out: Path, files: dict[str, bytes], labels: dict, detail: dict, force: bool) -> None:
    out.mkdir(parents=True, exist_ok=True)
    stale = [p for p in out.iterdir() if p.name in ("labels.json", "labels_detail.json") or
             (p.name.startswith("doc_") and p.suffix == ".pdf")]
    if stale and not force:
        raise SystemExit(f"{out} already holds a corpus ({len(stale)} files); pass --force to replace it")
    for p in stale:
        p.unlink()
    for fname, data in files.items():
        (out / fname).write_bytes(data)
    (out / "labels.json").write_bytes(dumps(labels))
    (out / "labels_detail.json").write_bytes(dumps(detail))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT, help="output directory (default: bench/data/synth_pdf)")
    ap.add_argument("--pages", type=int, default=300, help="total pages across all PDFs (default 300)")
    ap.add_argument("--seed", type=int, default=0, help="the same seed gives byte-identical output (default 0)")
    ap.add_argument("--force", action="store_true", help="replace an existing corpus in --out")
    args = ap.parse_args(argv)
    if args.pages < 1:
        ap.error("--pages must be at least 1")
    files, labels, detail = build_corpus(args.pages, args.seed)
    write_corpus(args.out, files, labels, detail, args.force)
    kinds: dict[str, int] = {}
    for fl in labels["files"].values():
        for pg in fl["pages"]:
            kinds[pg["text_layer"]] = kinds.get(pg["text_layer"], 0) + 1
    total = sum(len(d) for d in files.values())
    print(f"wrote {len(files)} PDFs, {args.pages} pages, {total / 1e6:.1f} MB to {args.out}")
    for k in sorted(kinds):
        print(f"  {k:28s} {kinds[k]:4d}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
