#!/usr/bin/env python3
"""Regenerate tests/golden/pdf/*.pdf: tiny synthetic PDFs for the structural layer's golden tests.

Synthetic only (never real documents: they contain PII). Pure standard library, no randomness and
no clock, so the bytes are identical on every machine. The one exception is `encrypted.pdf`, which
is made by `qpdf` when it is installed (and left alone when it is not).

After changing a fixture regenerate the expected reports with
`UPDATE_GOLDEN=1 cargo test -p pagetriage-core --features pdf --test golden_pdf` and review the
diff: a changed report is a behaviour change.

Fixtures (what each one is for):
  native_latin       one page of Latin text in an unnamed font
  scanned_ocr        raster-only page, an invisible OCR layer under a scan, and visible text that a
                     later page-sized image hides ("text under image"), producer OCRmyPDF
  scripts_legacy     Unicode Devanagari (Type0 + ToUnicode), text in a legacy Kruti Dev font,
                     Identity-H text without a ToUnicode map, and plain English
  vector_empty_mixed rules only, an empty page, text outlined into paths, text with a small logo
  signed_then_edited a signed base revision, an update that adds an annotation, an update that
                     rewrites page content after the signature
  jpeg_scan          two scans whose DCT streams carry an IJG quality-75 table and a custom table
  encrypted          opens only with a password (made with qpdf)
"""
import pathlib
import shutil
import subprocess
import sys

OUT = pathlib.Path(__file__).resolve().parent / "pdf"

# --- a minimal PDF assembler ---------------------------------------------------------------------


class Pdf:
    def __init__(self):
        self.out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        self.offsets = {}
        self.pending = []
        self.last_xref = None

    def obj(self, num, body):
        if isinstance(body, str):
            body = body.encode("latin-1")
        self.offsets[num] = len(self.out)
        self.pending.append(num)
        self.out += b"%d 0 obj\n" % num + body + b"\nendobj\n"

    def stream(self, num, dict_text, data):
        if isinstance(data, str):
            data = data.encode("latin-1")
        self.obj(num, b"<< " + dict_text.encode("latin-1") + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream")

    def finish(self, root, size, info=None):
        at = len(self.out)
        nums = sorted(set(self.pending))
        self.pending = []
        x = "xref\n"
        if self.last_xref is None:
            x += "0 1\n0000000000 65535 f \n"
        i = 0
        while i < len(nums):
            j = i
            while j + 1 < len(nums) and nums[j + 1] == nums[j] + 1:
                j += 1
            x += f"{nums[i]} {j - i + 1}\n"
            for n in nums[i : j + 1]:
                x += f"{self.offsets[n]:010d} 00000 n \n"
            i = j + 1
        x += f"trailer\n<< /Size {size} /Root {root} 0 R"
        if info:
            x += f" /Info {info} 0 R"
        if self.last_xref is not None:
            x += f" /Prev {self.last_xref}"
        x += f" >>\nstartxref\n{at}\n%%EOF\n"
        self.out += x.encode("latin-1")
        self.last_xref = at
        return bytes(self.out)


HELV = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
LETTER = "[0 0 612 792]"


def gray_image(num, pdf, w, h, extra=""):
    pdf.stream(num, f"/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8 {extra}", bytes((x * 7 + y * 3) % 256 for y in range(h) for x in range(w)))


def pages_doc(pdf, pages, producer=None, extra_objects=None):
    """pages: list of (content, resources_text). Objects: 1 catalog, 2 pages, then 3 + 2k page,
    4 + 2k content; resources may refer to numbers >= 100."""
    n = len(pages)
    pdf.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    pdf.obj(2, "<< /Type /Pages /Kids [" + " ".join(f"{3 + 2 * k} 0 R" for k in range(n)) + f"] /Count {n} >>")
    for k, (content, resources) in enumerate(pages):
        pdf.obj(3 + 2 * k, f"<< /Type /Page /Parent 2 0 R /MediaBox {LETTER} /Contents {4 + 2 * k} 0 R /Resources << {resources} >> >>")
        pdf.stream(4 + 2 * k, "", content)
    if extra_objects:
        extra_objects(pdf)
    info = None
    if producer:
        info = 99
        pdf.obj(99, f"<< /Producer ({producer}) /Creator (Synthetic fixture) >>")
    return info


def text(lines, font="F1", size=12, mode=None):
    y = 700
    out = "BT "
    if mode is not None:
        out += f"{mode} Tr "
    out += f"/{font} {size} Tf 72 {y} Td 16 TL "
    out += " ".join(f"({line}) '" for line in lines)
    return out + " ET\n"


FULL = "q 612 0 0 792 0 0 cm /Im1 Do Q\n"

# --- fixtures ------------------------------------------------------------------------------------


def native_latin():
    pdf = Pdf()
    lines = [
        "THIS DEED OF SALE is made at Pune on this day between the Vendor and the Purchaser,",
        "whereas the Vendor is the absolute owner of the property described in the Schedule",
        "hereunder written and has agreed to sell the same for the consideration mentioned herein.",
    ]
    info = pages_doc(pdf, [(text(lines), "/Font << /F1 100 0 R >>")], producer="Synthetic Writer 1.0",
                     extra_objects=lambda p: p.obj(100, HELV))
    return pdf.finish(1, 101, info)


def scanned_ocr():
    pdf = Pdf()
    ocr = ["The Vendor shall convey the property to the Purchaser free from encumbrances,", "charges and litigation of any kind whatsoever."]
    pages = [
        (FULL, "/XObject << /Im1 101 0 R >>"),
        (FULL + text(ocr, mode=3), "/Font << /F1 100 0 R >> /XObject << /Im1 101 0 R >>"),
        (text(ocr) + FULL, "/Font << /F1 100 0 R >> /XObject << /Im1 101 0 R >>"),
    ]

    def extra(p):
        p.obj(100, HELV)
        gray_image(101, p, 24, 32)

    info = pages_doc(pdf, pages, producer="OCRmyPDF 16.4.0 / Tesseract OCR-PDF 5.3.0", extra_objects=extra)
    return pdf.finish(1, 102, info)


def devanagari_cmap():
    # code -> Devanagari letters of "bharat sarkar"
    pairs = [(0x0003, 0x0020), (0x0024, 0x092D), (0x0025, 0x093E), (0x0026, 0x0930), (0x0027, 0x0924), (0x0028, 0x0938), (0x0029, 0x0915)]
    body = "\n".join(f"<{c:04X}> <{u:04X}>" for c, u in pairs)
    return ("/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n/CMapName /Adobe-Identity-UCS def\n"
            "1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
            f"{len(pairs)} beginbfchar\n{body}\nendbfchar\nendcmap end end")


def scripts_legacy():
    pdf = Pdf()
    hindi = "<0024002500260027000300280026002900250026>"  # bharat sarkar in the ToUnicode map
    legacy = [
        "Hkkjr ljdkj ds ukeku ls ;g foys[k fu\\\"iknu fd;k tkrk gS A",
        "{ks=Qy ds vk/kkj ij ¼[kjhnh½ fcØh ys[k lEifRr dk fooj.k",
    ]
    legacy = [l.replace("¼", "\\274").replace("½", "\\275").replace("Ø", "\\330") for l in legacy]
    pages = [
        (f"BT /F2 14 Tf 72 700 Td {hindi} Tj ET\n", "/Font << /F2 101 0 R >>"),
        (text(legacy), "/Font << /F1 103 0 R >>"),
        ("BT /F2 14 Tf 72 700 Td <002400250026002700280029002A002B> Tj ET\n", "/Font << /F2 104 0 R >>"),
        (text(["The Vendor shall convey the property to the Purchaser free from all encumbrances."]), "/Font << /F1 100 0 R >>"),
    ]

    def extra(p):
        p.obj(100, HELV)
        p.obj(101, "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Mangal /Encoding /Identity-H /DescendantFonts [102 0 R] /ToUnicode 105 0 R >>")
        p.obj(102, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Mangal /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>")
        p.obj(103, "<< /Type /Font /Subtype /TrueType /BaseFont /GHIJKL+KrutiDev010 /Encoding /WinAnsiEncoding >>")
        p.obj(104, "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Noto /Encoding /Identity-H /DescendantFonts [102 0 R] >>")
        p.stream(105, "", devanagari_cmap())

    info = pages_doc(pdf, pages, producer="Synthetic Writer 1.0", extra_objects=extra)
    return pdf.finish(1, 106, info)


def vector_empty_mixed():
    pdf = Pdf()
    rules = "0.5 w\n" + "".join(f"50 {y} m 560 {y} l S\n" for y in range(100, 700, 40)) + "50 100 510 600 re S\n"
    outlined = "".join(
        f"{x} {y} m {x + 2} {y + 5} {x + 4} {y + 5} {x + 6} {y} c {x + 6} {y - 2} {x + 4} {y - 4} {x + 3} {y - 4} c {x + 1} {y - 2} l h f\n"
        for y in range(700, 560, -14) for x in range(60, 500, 10)
    )
    mixed = text(["Registered office: Plot 12, Sector 9, Navi Mumbai. Telephone 022 5555 0100."]) + "q 120 0 0 80 440 690 cm /Im1 Do Q\n"
    pages = [
        (rules, ""),
        ("q Q\n", ""),
        (outlined, ""),
        (mixed, "/Font << /F1 100 0 R >> /XObject << /Im1 101 0 R >>"),
    ]

    def extra(p):
        p.obj(100, HELV)
        gray_image(101, p, 30, 20)

    info = pages_doc(pdf, pages, producer="Synthetic Writer 1.0", extra_objects=extra)
    return pdf.finish(1, 102, info)


def signed_then_edited():
    pdf = Pdf()
    base = text(["Amount payable under this agreement is Rs. 1,00,000 (Rupees one lakh only)."])
    pages_doc(pdf, [(base, "/Font << /F1 100 0 R >>")], extra_objects=lambda p: p.obj(100, HELV))
    pdf.obj(98, "<< /Producer (Synthetic Writer 1.0) /ModDate (D:20200101000000Z) >>")
    pdf.obj(97, "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 100 200 300] /Contents <00112233445566778899> /Name (Synthetic Signer) >>")
    pdf.finish(1, 101, 98)
    # Update 1: a text annotation (benign).
    pdf.obj(96, "<< /Type /Annot /Subtype /Text /Rect [20 20 40 40] /Contents (Seen) >>")
    pdf.obj(3, f"<< /Type /Page /Parent 2 0 R /MediaBox {LETTER} /Contents 4 0 R /Resources << /Font << /F1 100 0 R >> >> /Annots [96 0 R] >>")
    pdf.finish(1, 101, 98)
    # Update 2: the amount is rewritten after the signature.
    pdf.stream(4, "", text(["Amount payable under this agreement is Rs. 9,00,000 (Rupees nine lakh only)."]))
    pdf.finish(1, 101, 98)
    return bytes(pdf.out)


IJG_LUMA = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
]
ZIGZAG = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
]


def jpeg_header(natural_table, w=64, h=80):
    """SOI, one DQT, a baseline SOF, an SOS and EOI: enough for the table parser, not decodable."""
    dqt = bytes([0xFF, 0xDB, 0x00, 0x43, 0x00]) + bytes(natural_table[i] for i in ZIGZAG)
    sof = bytes([0xFF, 0xC0, 0x00, 0x0B, 0x08, h >> 8, h & 255, w >> 8, w & 255, 1, 1, 0x11, 0])
    sos = bytes([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0, 0xFF, 0xD9])
    return bytes([0xFF, 0xD8]) + dqt + sof + sos


def ijg_table(quality):
    scale = 5000 // quality if quality < 50 else 200 - quality * 2
    return [min(255, max(1, (v * scale + 50) // 100)) for v in IJG_LUMA]


def jpeg_scan():
    pdf = Pdf()
    custom = [3 + (i * 7) % 31 for i in range(64)]
    pages = [(FULL, "/XObject << /Im1 100 0 R >>"), (FULL, "/XObject << /Im1 101 0 R >>")]

    def extra(p):
        dict_text = "/Type /XObject /Subtype /Image /Width 64 /Height 80 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode"
        p.stream(100, dict_text, jpeg_header(ijg_table(75)))
        p.stream(101, dict_text, jpeg_header(custom))

    info = pages_doc(pdf, pages, producer="Synthetic Scanner Suite 2.0", extra_objects=extra)
    return pdf.finish(1, 102, info)


def encrypted(plain):
    qpdf = shutil.which("qpdf")
    if not qpdf:
        return None
    src = OUT / "_plain_for_encrypt.pdf"
    dst = OUT / "encrypted.pdf"
    src.write_bytes(plain)
    try:
        subprocess.run([qpdf, "--static-id", "--static-aes-iv", "--encrypt", "userpw", "ownerpw", "128", "--use-aes=y", "--", str(src), str(dst)], check=True, capture_output=True)
    finally:
        src.unlink()
    return dst.read_bytes()


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    files = {
        "native_latin": native_latin(),
        "scanned_ocr": scanned_ocr(),
        "scripts_legacy": scripts_legacy(),
        "vector_empty_mixed": vector_empty_mixed(),
        "signed_then_edited": signed_then_edited(),
        "jpeg_scan": jpeg_scan(),
    }
    for name, data in files.items():
        (OUT / f"{name}.pdf").write_bytes(data)
    if encrypted(files["native_latin"]) is None:
        print("qpdf not found: encrypted.pdf left as it is", file=sys.stderr)
    print(f"wrote {len(files)} PDFs to {OUT}")


if __name__ == "__main__":
    main()
