"""Fonts, page-content builders and annotation helpers for the synthetic PDF corpus.

Each `build_*` function takes a `PdfFile`, a seeded `random.Random` and a page plan, adds the page's
content, image and font objects to the file, and returns a `PageBuild`: the page-dictionary entries,
the ground-truth label, and details (expected text, image encoding, ...) for the sidecar file.
"""
from __future__ import annotations

import math
import random
from dataclasses import dataclass, field

import numpy as np

import scanimg
import synthtext as st
from pdfio import PdfFile, fmt, hexstr, lit, name, stream_body

LABEL_OF = {"native": "native_text", "raster": "raster_only", "ocr": "invisible_ocr_under_image",
            "mixed": "mixed", "vector": "vector_only", "empty": "empty"}


@dataclass
class PagePlan:
    kind: str                       # native | raster | ocr | mixed | vector | empty
    script: str = "latin"           # script of the text layer (or of the image content for scans)
    legacy: bool = False
    variant: str | None = None
    enc: str | None = None          # force an image encoding (raster and OCR pages)


@dataclass
class PageBuild:
    entries: dict[str, str]         # page dictionary entries (no Type, Parent)
    label: dict
    detail: dict
    media: tuple[float, float] | None = None   # set when the builder dictates the page size


# ---- fonts --------------------------------------------------------------------------------------

class SimpleFont:
    """A single-byte font: WinAnsi text. `avg` is the average advance used for wrapping."""

    def __init__(self, obj: int, avg: float):
        self.obj, self.avg = obj, avg

    def encode(self, text: str) -> str:
        return lit(text.encode("cp1252", errors="replace"))

    def width(self, text: str, fs: float) -> float:
        return len(text) * fs * self.avg


CONSONANT_RE_SCRIPTS = set(st.BLOCK)


class CidFont:
    """Type0 / Identity-H font with a /ToUnicode CMap. Glyph ids are arbitrary; the font program is
    not embedded (nothing here rasterises). Indic conjuncts (consonant + virama + consonant) are one
    glyph that maps back to three code points, as in real subset fonts."""

    def __init__(self, f: PdfFile, rng: random.Random, script: str, base: str):
        self.f, self.script, self.base = f, script, base
        self.units: dict[str, int] = {}
        self.pool = rng.sample(range(3, 3000), 700)
        self.obj = f.new()
        self.o_cid, self.o_desc, self.o_tu = f.new(), f.new(), f.new()
        self.avg = 0.62
        self._inv = st._INV.get(script)

    def segment(self, text: str) -> list[str]:
        if not self._inv:
            return list(text)
        cons = set(self._inv["consonants"])
        v = self._inv["virama"]
        out, i = [], 0
        while i < len(text):
            if i + 2 < len(text) and text[i] in cons and text[i + 1] == v and text[i + 2] in cons:
                out.append(text[i:i + 3])
                i += 3
            else:
                out.append(text[i])
                i += 1
        return out

    def encode(self, text: str) -> str:
        codes = bytearray()
        for u in self.segment(text):
            if u not in self.units:
                self.units[u] = self.pool[len(self.units)]
            codes += self.units[u].to_bytes(2, "big")
        return hexstr(bytes(codes))

    def width(self, text: str, fs: float) -> float:
        return sum(0.25 if ch == " " else 0.6 for ch in text) * fs

    def finalize(self) -> None:
        f = self.f
        items = sorted((cid, u) for u, cid in self.units.items())
        lines = ["/CIDInit /ProcSet findresource begin", "12 dict begin", "begincmap",
                 "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def",
                 "/CMapName /Adobe-Identity-UCS def", "/CMapType 2 def",
                 "1 begincodespacerange", "<0000> <FFFF>", "endcodespacerange"]
        for i in range(0, len(items), 100):
            chunk = items[i:i + 100]
            lines.append(f"{len(chunk)} beginbfchar")
            for cid, u in chunk:
                lines.append(f"<{cid:04X}> <{u.encode('utf-16-be').hex().upper()}>")
            lines.append("endbfchar")
        lines += ["endcmap", "CMapName currentdict /CMap defineresource pop", "end", "end"]
        f.set(self.o_tu, stream_body("", "\n".join(lines).encode(), compress=True))
        bn = name(self.base)
        f.set(self.o_desc, f"<< /Type /FontDescriptor /FontName {bn} /Flags 4 /FontBBox [-100 -250 1000 900] "
                           f"/ItalicAngle 0 /Ascent 900 /Descent -250 /CapHeight 700 /StemV 80 >>")
        f.set(self.o_cid, f"<< /Type /Font /Subtype /CIDFontType2 /BaseFont {bn} "
                          f"/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
                          f"/FontDescriptor {self.o_desc} 0 R /DW 600 /W [3 [250]] /CIDToGIDMap /Identity >>")
        f.set(self.obj, f"<< /Type /Font /Subtype /Type0 /BaseFont {bn} /Encoding /Identity-H "
                        f"/DescendantFonts [{self.o_cid} 0 R] /ToUnicode {self.o_tu} 0 R >>")
        # NB: /W maps CID 3 only; the corpus never lays out by glyph advance, so widths are nominal.


class GlyphlessFont:
    """Tesseract-style invisible-text font: 2-byte codes are UTF-16 code units, identity ToUnicode."""

    def __init__(self, f: PdfFile):
        tu = (b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
              b"/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n"
              b"/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\n"
              b"endcodespacerange\n1 beginbfrange\n<0000> <FFFF> <0000>\nendbfrange\nendcmap\n"
              b"CMapName currentdict /CMap defineresource pop\nend\nend")
        o_tu = f.add_stream("", tu, compress=True)
        o_desc = f.add("<< /Type /FontDescriptor /FontName /GlyphLessFont /Flags 5 /FontBBox [0 0 500 1000] "
                       "/ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 1000 /StemV 80 >>")
        o_cid = f.add("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /GlyphLessFont "
                      "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> "
                      f"/FontDescriptor {o_desc} 0 R /DW 500 /CIDToGIDMap /Identity >>")
        self.obj = f.add("<< /Type /Font /Subtype /Type0 /BaseFont /GlyphLessFont /Encoding /Identity-H "
                         f"/DescendantFonts [{o_cid} 0 R] /ToUnicode {o_tu} 0 R >>")
        self.avg = 0.5

    def encode(self, text: str) -> str:
        return hexstr(text.encode("utf-16-be"))

    def width(self, text: str, fs: float) -> float:
        return len(text) * fs * 0.5


# Latin simple fonts: key -> (BaseFont, bold BaseFont, subtype, descriptor, uniform /Widths, wrap factor)
LATIN_FONTS = {
    "helv": ("Helvetica", "Helvetica-Bold", "Type1", False, False, 0.56),
    "times": ("Times-Roman", "Times-Bold", "Type1", False, False, 0.5),
    "courier": ("Courier", "Courier-Bold", "Type1", False, False, 0.6),
    "arial": ("ArialMT", "Arial-BoldMT", "TrueType", True, True, 0.5),
    "tnr": ("TimesNewRomanPSMT", "TimesNewRomanPS-BoldMT", "TrueType", True, True, 0.5),
    "calibri": ("Calibri", "Calibri-Bold", "TrueType", True, True, 0.5),
    "cmr": ("CMR10", "CMBX12", "Type1", True, True, 0.5),
    "verdana": ("Verdana", "Verdana-Bold", "TrueType", True, True, 0.5),
}

INDIC_FAMILIES = {
    "latin": ["Arial", "TimesNewRoman", "Cambria", "Segoe UI"],
    "devanagari": ["Mangal", "Kokila", "NirmalaUI", "NotoSansDevanagari-Regular"],
    "bengali": ["Vrinda", "Shonar Bangla", "NotoSansBengali-Regular"],
    "gurmukhi": ["Raavi", "NotoSansGurmukhi-Regular"],
    "gujarati": ["Shruti", "NotoSansGujarati-Regular"],
    "tamil": ["Latha", "Vijaya", "NotoSansTamil-Regular"],
    "telugu": ["Gautami", "Vani", "NotoSansTelugu-Regular"],
    "kannada": ["Tunga", "Nirmala UI", "NotoSansKannada-Regular"],
    "malayalam": ["Kartika", "NotoSansMalayalam-Regular"],
    "arabic": ["Arial", "TraditionalArabic", "Tahoma", "NotoNaskhArabic-Regular"],
}

LEGACY_FONT_NAMES = ["Kruti Dev 010", "DV-TTYogesh", "Shree-Lipi", "Chanakya", "APS-Dev", "Walkman-Chanakya"]


class FontPool:
    def __init__(self, f: PdfFile, rng: random.Random):
        self.f, self.rng = f, rng
        self.simple_cache: dict[tuple, SimpleFont] = {}
        self.cid_cache: dict[tuple, CidFont] = {}
        self.glyphless: GlyphlessFont | None = None

    def latin(self, key: str, bold: bool = False) -> SimpleFont:
        base, bold_base, sub, desc, widths, avg = LATIN_FONTS[key]
        return self.simple(bold_base if bold else base, sub, desc, widths, avg)

    def simple(self, base: str, subtype: str, descriptor: bool, widths: bool, avg: float,
               flags: int = 32, family: str | None = None, encoding: str | None = "WinAnsiEncoding") -> SimpleFont:
        key = (base, subtype, descriptor, widths, flags, family, encoding)
        if key in self.simple_cache:
            return self.simple_cache[key]
        parts = [f"/Type /Font /Subtype /{subtype} /BaseFont {name(base)}"]
        if encoding:
            parts.append(f"/Encoding /{encoding}")
        if widths:
            parts.append("/FirstChar 32 /LastChar 255 /Widths [" + " ".join(["500"] * 224) + "]")
        if descriptor:
            fam = f" /FontFamily {lit(family.encode())}" if family else ""
            fd = self.f.add(f"<< /Type /FontDescriptor /FontName {name(base)} /Flags {flags} "
                            f"/FontBBox [-100 -250 1000 900] /ItalicAngle 0 /Ascent 900 /Descent -250 "
                            f"/CapHeight 700 /StemV 80{fam} >>")
            parts.append(f"/FontDescriptor {fd} 0 R")
        font = SimpleFont(self.f.add("<< " + " ".join(parts) + " >>"), 0.5 if widths else avg)
        self.simple_cache[key] = font
        return font

    def cid(self, script: str) -> CidFont:
        key = (script,)
        if key not in self.cid_cache:
            base = self.rng.choice(INDIC_FAMILIES[script])
            self.cid_cache[key] = CidFont(self.f, self.rng, script, base)
        return self.cid_cache[key]

    def glyphless_font(self) -> GlyphlessFont:
        if self.glyphless is None:
            self.glyphless = GlyphlessFont(self.f)
        return self.glyphless

    def finalize(self) -> None:
        for c in self.cid_cache.values():
            c.finalize()


# ---- geometry and content helpers ---------------------------------------------------------------

PAGE_SIZES = {"A4": (595.276, 841.89), "Letter": (612.0, 792.0), "Legal": (612.0, 1008.0),
              "Foolscap": (595.276, 936.0), "A3": (841.89, 1190.551), "A5": (419.528, 595.276),
              "B5": (498.898, 708.661)}
SIZE_WEIGHTS = {"A4": 55, "Legal": 12, "Letter": 8, "Foolscap": 10, "A3": 6, "A5": 5, "B5": 4}


def pick_size(rng: random.Random) -> str:
    return rng.choices(list(SIZE_WEIGHTS), weights=list(SIZE_WEIGHTS.values()))[0]


@dataclass
class Geo:
    w: float
    h: float
    ml: float
    mt: float
    mr: float
    mb: float


def make_geo(rng: random.Random, w: float, h: float) -> Geo:
    unit = min(w, h)
    return Geo(w, h, rng.uniform(0.07, 0.17) * unit, rng.uniform(0.07, 0.16) * unit,
               rng.uniform(0.07, 0.15) * unit, rng.uniform(0.07, 0.15) * unit)


def resources(fonts: dict[str, int] | None = None, xobjs: dict[str, int] | None = None,
              procset: bool = False) -> str:
    parts = []
    if fonts:
        parts.append("/Font << " + " ".join(f"/{k} {v} 0 R" for k, v in fonts.items()) + " >>")
    if xobjs:
        parts.append("/XObject << " + " ".join(f"/{k} {v} 0 R" for k, v in xobjs.items()) + " >>")
    if procset:
        parts.append("/ProcSet [/PDF /Text /ImageB /ImageC /ImageI]")
    return "<< " + " ".join(parts) + " >>"


def content_obj(f: PdfFile, ops: list[str], compress: bool) -> int:
    return f.add_stream("", "\n".join(ops).encode("latin-1"), compress=compress)


def contents_entry(f: PdfFile, ops: list[str], compress: bool, rng: random.Random,
                   split_ok: bool = False) -> str:
    """One content stream, or two when `split_ok` and the coin says so. Splits only fall between
    whole operator groups, which the callers guarantee by passing group-aligned `ops`."""
    if split_ok and len(ops) > 8 and rng.random() < 0.15:
        cut = rng.randrange(2, len(ops) - 2)
        while ops[cut].startswith(("/", "ET", "Q")) and cut < len(ops) - 2:
            cut += 1
        a = content_obj(f, ops[:cut], compress)
        b = content_obj(f, ops[cut:], compress)
        return f"[{a} 0 R {b} 0 R]"
    return f"{content_obj(f, ops, compress)} 0 R"


def show(font, text: str, rng: random.Random, tj_p: float = 0.2) -> str:
    """`(text) Tj`, or a TJ array with tiny kerning adjustments."""
    if len(text) > 8 and rng.random() < tj_p:
        cut = rng.randrange(2, len(text) - 2)
        k = rng.choice([-12, -6, -3, 4, 8])
        return f"[{font.encode(text[:cut])} {k} {font.encode(text[cut:])}] TJ"
    return f"{font.encode(text)} Tj"


@dataclass
class TextLine:
    text: str
    x: float
    y: float
    size: float
    bold: bool = False
    centered: bool = False
    rtl: bool = False               # stored in visual order (reversed), as right-to-left PDFs do


def flow_text(rng: random.Random, script: str, legacy: bool, geo: Geo, fs: float, avg: float,
              columns: int, fill: float, title: bool, footer: bool, rtl: bool = False,
              width_fn=None, title_text: str | None = None) -> tuple[list[TextLine], int]:
    gap = 24.0
    col_w = (geo.w - geo.ml - geo.mr - (columns - 1) * gap) / columns
    wf = width_fn or (lambda t, s: len(t) * s * avg)
    max_chars = max(12, int(col_w / (fs * avg * 1.02)))
    lead = fs * rng.uniform(1.22, 1.5)
    top = geo.h - geo.mt
    bottom = geo.mb + (lead * 2 if footer else 0)
    avail = (top - bottom) * fill
    lines: list[TextLine] = []
    if title:
        t = title_text or (st.title(rng, script) if not legacy else st.legacy_word(rng) + " " + st.legacy_word(rng))
        tw = wf(t, fs * 1.4)
        lines.append(TextLine(t, max(geo.ml * 0.5, geo.ml + (col_w - tw) / 2), top - fs * 1.4, fs * 1.4, True, True, rtl))
    for col in range(columns):
        x = geo.ml + col * (col_w + gap)
        y = top - (fs * 1.4 + lead * 1.8 if (title and col == 0) else fs)
        while y > bottom and (top - y) < avail:
            if legacy:
                para = st.legacy_paragraph(rng)
            else:
                para = st.paragraph(rng, script)
            for ln in st.wrap(para, max_chars):
                if y <= bottom or (top - y) >= avail:
                    break
                wid = wf(ln, fs)
                lx = x if not rtl else x + col_w - wid
                lines.append(TextLine(ln, max(geo.ml * 0.5, lx), y, fs, rtl=rtl))
                y -= lead
            y -= lead * 0.6
    if footer:
        n = str(rng.randint(1, 99))
        txt = rng.choice([f"- {n} -", n, f"Page {n}"])
        lines.append(TextLine(txt, geo.w / 2 - 12, geo.mb * 0.55, max(8.0, fs - 2), False, False))
    return lines, columns


def emit_lines(lines: list[TextLine], fonts_res: dict[bool, tuple[str, object]], rng: random.Random,
               per_line_bt: bool, tj_p: float = 0.2) -> list[str]:
    ops: list[str] = []
    if not per_line_bt:
        ops.append("BT")
    px = py = 0.0
    for ln in lines:
        res, font = fonts_res[ln.bold]
        text = ln.text[::-1] if ln.rtl else ln.text
        x, y = ln.x, ln.y
        if per_line_bt:
            ops += ["BT", f"/{res} {fmt(ln.size)} Tf", f"{fmt(x)} {fmt(y)} Td", show(font, text, rng, tj_p), "ET"]
        else:
            ops += [f"/{res} {fmt(ln.size)} Tf", f"{fmt(x - px)} {fmt(y - py)} Td", show(font, text, rng, tj_p)]
            px, py = x, y
    if not per_line_bt:
        ops.append("ET")
    return ops


def expected_from(lines: list[TextLine]) -> str:
    return "\n".join(ln.text for ln in lines)
