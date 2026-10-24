"""PDF structural layer through the Python binding: JPEG quality against real libjpeg output, and
the scoring script against the golden PDFs."""
import importlib.util
import io
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GOLDEN = ROOT / "tests" / "golden"

try:
    import pagetriage_py
    from PIL import Image
except ImportError as e:  # binding or Pillow not installed
    pagetriage_py = None
    SKIP_REASON = f"needs the pagetriage_py binding and Pillow: {e}"
else:
    SKIP_REASON = ""


def load_gen_pdfs():
    spec = importlib.util.spec_from_file_location("gen_pdfs", GOLDEN / "gen_pdfs.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipIf(pagetriage_py is None, SKIP_REASON)
class StructureTests(unittest.TestCase):
    def test_jpeg_quality_is_recovered_from_libjpeg_tables(self):
        # Pillow's encoder is libjpeg(-turbo): its quantisation tables are the IJG scaling the
        # estimator inverts, so every quality must come back exactly with zero fit error.
        g = load_gen_pdfs()
        for quality in (1, 5, 10, 25, 40, 50, 60, 75, 85, 90, 95, 100):
            buf = io.BytesIO()
            Image.new("L", (16, 16), 128).save(buf, "JPEG", quality=quality)
            pdf = g.Pdf()
            info = g.pages_doc(
                pdf,
                [(g.FULL, "/XObject << /Im1 100 0 R >>")],
                extra_objects=lambda p: p.stream(
                    100,
                    "/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceGray "
                    "/BitsPerComponent 8 /Filter /DCTDecode",
                    buf.getvalue(),
                ),
            )
            report = json.loads(pagetriage_py.analyse_pdf_structure(pdf.finish(1, 101, info)))
            jpeg = report["pages"][0]["images"][0]["jpeg"]
            self.assertEqual(jpeg["luma_quality"], quality)
            self.assertEqual(jpeg["luma_fit_error"], 0.0)
            self.assertTrue(jpeg["ijg_scaled"])

    def test_golden_pdfs_have_the_expected_states(self):
        want = {
            "native_latin": ["native_text"],
            "scanned_ocr": ["raster_only", "invisible_ocr_under_image", "invisible_ocr_under_image"],
            "vector_empty_mixed": ["vector_only", "empty", "vector_only", "mixed"],
            "jpeg_scan": ["raster_only", "raster_only"],
        }
        for name, states in want.items():
            report = json.loads(pagetriage_py.analyse_pdf_structure((GOLDEN / "pdf" / f"{name}.pdf").read_bytes()))
            self.assertEqual([p["text_layer"]["state"] for p in report["pages"]], states, name)

    def test_errors_are_value_errors(self):
        for bad in (b"", b"not a pdf", b"%PDF-1.4\ngarbage"):
            with self.assertRaises(ValueError):
                pagetriage_py.analyse_pdf_structure(bad)
        with self.assertRaises(ValueError):
            pagetriage_py.analyse_pdf_structure(b"%PDF-1.4", '{"unknown": 1}')

    def test_schema_is_exposed(self):
        schema = json.loads(pagetriage_py.structure_schema_json())
        self.assertEqual(schema["title"], "StructureReport")


if __name__ == "__main__":
    sys.exit(unittest.main())
