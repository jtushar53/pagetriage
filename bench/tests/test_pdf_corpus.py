"""Tests for the synthetic PDF corpus generator (bench/synth/make_pdf_corpus.py).

Offline. Tests that need qpdf or poppler skip cleanly when those tools are missing (one test below
runs the suite with an empty PATH to prove it). Generator-only tests need Pillow and numpy, as the
rest of bench/ does.

    python -I -m unittest discover -s bench/tests
"""
import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import zlib
from functools import lru_cache
from pathlib import Path

try:
    import numpy  # noqa: F401
    import PIL  # noqa: F401
except ImportError as exc:  # pragma: no cover
    raise unittest.SkipTest(f"corpus generator needs Pillow and numpy: {exc}")

SYNTH = Path(__file__).resolve().parents[1] / "synth"
sys.path.insert(0, str(SYNTH))
import make_pdf_corpus as gen  # noqa: E402
import synthtext  # noqa: E402

PAGES, SEED = 60, 1
LAYERS = {"native_text", "raster_only", "invisible_ocr_under_image", "mixed", "vector_only", "empty"}
SCRIPTS = {"latin", "devanagari", "bengali", "gurmukhi", "gujarati", "tamil", "telugu", "kannada",
           "malayalam", "arabic", "none"}
NO_TEXT = {"raster_only", "vector_only", "empty"}
LEGACY_NAMES = ["KrutiDev010", "DV-TTYogesh", "DVTTYogesh", "Shree-Lipi", "Chanakya", "APS-Dev",
                "Walkman-Chanakya"]
BIDI = {c: None for c in list(range(0x202A, 0x202F)) + list(range(0x2066, 0x206A))}


@lru_cache(maxsize=None)
def corpus(pages=PAGES, seed=SEED):
    return gen.build_corpus(pages, seed)


@lru_cache(maxsize=None)
def corpus_on_disk(pages=PAGES, seed=SEED) -> Path:
    """The corpus written once to a temp dir that lives for the whole test run."""
    d = Path(tempfile.mkdtemp(prefix="pdf_corpus_test_"))
    files, labels, detail = corpus(pages, seed)
    gen.write_corpus(d, files, labels, detail, force=True)
    return d


def tool(name):
    return shutil.which(name)


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=120)


def norm_tokens(s):
    return s.translate(BIDI).split()


class GeneratorTests(unittest.TestCase):
    def test_same_seed_same_bytes(self):
        files_a, labels_a, detail_a = gen.build_corpus(24, 5)
        files_b, labels_b, detail_b = gen.build_corpus(24, 5)
        self.assertEqual(files_a, files_b)
        self.assertEqual(gen.dumps(labels_a), gen.dumps(labels_b))
        self.assertEqual(gen.dumps(detail_a), gen.dumps(detail_b))

    def test_different_seed_differs(self):
        a, _, _ = gen.build_corpus(24, 5)
        b, _, _ = gen.build_corpus(24, 6)
        self.assertNotEqual(a, b)

    def test_same_bytes_across_processes(self):
        """Two separate interpreters (each with its own hash randomisation) write identical files."""
        outs = []
        for _ in range(2):
            d = Path(tempfile.mkdtemp(prefix="pdf_corpus_det_"))
            self.addCleanup(shutil.rmtree, d, ignore_errors=True)
            r = run([sys.executable, "-I", str(SYNTH / "make_pdf_corpus.py"), "--out", str(d), "--pages", "18",
                     "--seed", "9"])
            self.assertEqual(r.returncode, 0, r.stderr)
            outs.append({p.name: p.read_bytes() for p in sorted(d.iterdir())})
        self.assertEqual(outs[0], outs[1])
        self.assertIn("labels.json", outs[0])
        self.assertIn("labels_detail.json", outs[0])
        # and it matches the in-process result
        files, labels, _ = gen.build_corpus(18, 9)
        for name, data in files.items():
            self.assertEqual(outs[0][name], data)
        self.assertEqual(outs[0]["labels.json"], gen.dumps(labels))

    def test_generator_runs_without_external_tools(self):
        d = Path(tempfile.mkdtemp(prefix="pdf_corpus_path_"))
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        env = {"PATH": str(d)}  # nothing on PATH
        r = subprocess.run([sys.executable, "-I", str(SYNTH / "make_pdf_corpus.py"), "--out", str(d / "o"),
                            "--pages", "6"], capture_output=True, text=True, env=env, timeout=120)
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_refuses_to_overwrite_without_force(self):
        d = Path(tempfile.mkdtemp(prefix="pdf_corpus_force_"))
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        cmd = [sys.executable, "-I", str(SYNTH / "make_pdf_corpus.py"), "--out", str(d), "--pages", "6"]
        self.assertEqual(run(cmd).returncode, 0)
        self.assertNotEqual(run(cmd).returncode, 0)
        self.assertEqual(run(cmd + ["--force"]).returncode, 0)

    def test_page_and_file_counts(self):
        for pages in (1, 7, 60):
            files, labels, _ = gen.build_corpus(pages, 3)
            n = sum(len(v["pages"]) for v in labels["files"].values())
            self.assertEqual(n, pages)
            self.assertEqual(set(files), set(labels["files"]))
        files, _, _ = corpus()
        self.assertEqual(len(files), PAGES // 5)  # about one file per five pages

    def test_default_is_300_pages_in_about_60_files(self):
        docs = gen.plan_corpus(0, 300)
        self.assertEqual(sum(d.n_pages for d in docs), 300)
        self.assertEqual(len(docs), 60)

    def test_label_schema(self):
        _, labels, _ = corpus()
        self.assertEqual(list(labels), ["files"])
        for name, fl in labels["files"].items():
            self.assertRegex(name, r"^doc_\d{3}\.pdf$")
            self.assertEqual(set(fl), {"producer", "incremental_updates", "pages"})
            self.assertIsInstance(fl["producer"], str)
            self.assertIn(fl["incremental_updates"], (0, 1, 2, 3))
            self.assertGreaterEqual(len(fl["pages"]), 1)
            for pg in fl["pages"]:
                self.assertEqual(set(pg), {"text_layer", "script", "legacy_font"})
                self.assertIn(pg["text_layer"], LAYERS)
                self.assertIn(pg["script"], SCRIPTS)
                self.assertIsInstance(pg["legacy_font"], bool)
                if pg["text_layer"] in NO_TEXT:
                    self.assertEqual(pg["script"], "none")
                    self.assertFalse(pg["legacy_font"])
                if pg["legacy_font"]:
                    self.assertEqual(pg["script"], "devanagari")
                    self.assertIn(pg["text_layer"], ("native_text", "invisible_ocr_under_image"))

    def test_corpus_covers_every_kind(self):
        _, labels, _ = corpus()
        pages = [pg for fl in labels["files"].values() for pg in fl["pages"]]
        self.assertEqual({p["text_layer"] for p in pages}, LAYERS)
        self.assertEqual({p["script"] for p in pages}, SCRIPTS)
        self.assertEqual({p["legacy_font"] for p in pages}, {True, False})
        self.assertEqual({fl["incremental_updates"] for fl in labels["files"].values()}, {0, 1, 2, 3})
        self.assertIn("", {fl["producer"] for fl in labels["files"].values()})
        self.assertGreaterEqual(len({fl["producer"] for fl in labels["files"].values()}), 10)
        legacy_ocr = [p for p in pages if p["legacy_font"] and p["text_layer"] == "invisible_ocr_under_image"]
        self.assertTrue(legacy_ocr)

    def test_sidecar_agrees_with_labels(self):
        _, labels, detail = corpus()
        self.assertEqual(set(labels["files"]), set(detail["files"]))
        self.assertEqual((detail["seed"], detail["pages"]), (SEED, PAGES))
        for name, fl in labels["files"].items():
            self.assertEqual(len(fl["pages"]), len(detail["files"][name]["pages"]))
            self.assertEqual(len(detail["files"][name]["revisions"]), fl["incremental_updates"])
            for pg, dt in zip(fl["pages"], detail["files"][name]["pages"]):
                if pg["text_layer"] in NO_TEXT:
                    self.assertIsNone(dt["expected_text"])
                else:
                    self.assertTrue(dt["expected_text"].strip())

    def test_text_content_matches_script(self):
        _, labels, detail = corpus()
        seen = set()
        for name, fl in labels["files"].items():
            for pg, dt in zip(fl["pages"], detail["files"][name]["pages"]):
                if pg["text_layer"] in NO_TEXT:
                    continue
                found = synthtext.script_of(dt["expected_text"])
                if pg["legacy_font"]:
                    self.assertEqual(found, {"latin"}, "legacy text must be Latin code points")
                elif pg["script"] == "latin":
                    self.assertEqual(found, {"latin"})
                else:
                    self.assertIn(pg["script"], found)
                    self.assertTrue(found <= {pg["script"], "latin"}, found)
                seen.add(pg["script"])
        self.assertTrue({"devanagari", "bengali", "tamil", "arabic"} <= seen)

    def test_incremental_update_structure(self):
        """Each revision ends in %%EOF and /Prev points at a real cross-reference section."""
        files, labels, _ = corpus()
        checked = 0
        for name, data in files.items():
            n = labels["files"][name]["incremental_updates"]
            self.assertTrue(data.startswith(b"%PDF-1."))
            self.assertTrue(data.endswith(b"%%EOF\n"))
            self.assertEqual(data.count(b"%%EOF"), n + 1, name)
            self.assertEqual(data.count(b"startxref"), n + 1, name)
            pos = len(data)
            offsets = []
            for _ in range(n + 1):
                i = data.rindex(b"startxref", 0, pos)
                offsets.append(int(data[i:].split()[1]))
                pos = i
            for off in offsets:
                head = data[off:off + 20]
                self.assertTrue(head.startswith(b"xref") or re.match(rb"\d+ 0 obj", head), (name, head))
            # the chain in the newest revision leads to the previous startxref
            if n:
                newest = data[offsets[0]:]
                m = re.search(rb"/Prev (\d+)", newest[:newest.index(b"startxref")])
                self.assertIsNotNone(m, name)
                self.assertEqual(int(m.group(1)), offsets[1], name)
                checked += 1
        self.assertGreaterEqual(checked, 4)

    def test_legacy_pages_use_a_legacy_font_name(self):
        files, labels, _ = corpus()
        found = 0
        for name, data in files.items():
            if not any(p["legacy_font"] for p in labels["files"][name]["pages"]):
                continue
            fonts = [m.decode("latin-1").replace("#20", "") for m in re.findall(rb"/BaseFont /([^\s/>]+)", data)]
            hits = [f for f in fonts if any(f.split("+")[-1] == n for n in LEGACY_NAMES)]
            self.assertTrue(hits, (name, fonts))
            found += 1
        self.assertGreaterEqual(found, 1)

    def test_image_encodings_are_varied(self):
        files, _, detail = corpus()
        filters = set()
        for fl in detail["files"].values():
            for dt in fl["pages"]:
                for im in dt.get("images", []):
                    filters.add(im["filter"])
        self.assertEqual(filters, {"DCTDecode", "FlateDecode", "CCITTFaxDecode"})
        blob = b"".join(files.values())
        for key in (b"/DCTDecode", b"/FlateDecode", b"/CCITTFaxDecode", b"/Predictor 15"):
            self.assertIn(key, blob)

    def test_invisible_text_uses_render_mode_3(self):
        """OCR pages draw their text with `3 Tr`, except the deliberate `occluded` variant (`0 Tr`)."""
        files, labels, detail = corpus()
        variants = set()
        for name, fl in labels["files"].items():
            vs = {detail["files"][name]["pages"][k]["variant"] for k, pg in enumerate(fl["pages"])
                  if pg["text_layer"] == "invisible_ocr_under_image"}
            variants |= vs
            if vs & {"after", "before"}:
                streams = decoded_streams(files[name])
                self.assertTrue(any(b"\n3 Tr\n" in s for s in streams), name)
        self.assertTrue({"after", "before"} <= variants)


def decoded_streams(data: bytes):
    out = []
    for m in re.finditer(rb"stream\n(.*?)\nendstream", data, re.S):
        raw = m.group(1)
        try:
            out.append(zlib.decompress(raw))
        except zlib.error:
            out.append(raw)
    return out


@unittest.skipUnless(tool("qpdf"), "qpdf not installed")
class QpdfTests(unittest.TestCase):
    def test_every_file_passes_qpdf_check(self):
        d = corpus_on_disk()
        for p in sorted(d.glob("*.pdf")):
            r = run([tool("qpdf"), "--check", str(p)])
            out = r.stdout + r.stderr
            self.assertEqual(r.returncode, 0, f"{p.name}: {out[-600:]}")
            self.assertNotIn("WARNING", out, f"{p.name}: {out[-600:]}")
            self.assertIn("No syntax or stream encoding errors found", out, p.name)


@unittest.skipUnless(tool("pdfinfo") and tool("pdftotext"), "poppler (pdfinfo, pdftotext) not installed")
class PopplerTests(unittest.TestCase):
    def test_pdfinfo_is_clean_and_page_counts_match(self):
        _, labels, _ = corpus()
        d = corpus_on_disk()
        for name, fl in labels["files"].items():
            r = run([tool("pdfinfo"), str(d / name)])
            self.assertEqual(r.returncode, 0, f"{name}: {r.stderr}")
            self.assertEqual(r.stderr.strip(), "", name)
            pages = int(re.search(r"Pages:\s+(\d+)", r.stdout).group(1))
            self.assertEqual(pages, len(fl["pages"]), name)

    def test_labels_agree_with_pdftotext(self):
        _, labels, detail = corpus()
        d = corpus_on_disk()
        stats = {"exact": 0, "reordered": 0, "arabic": 0, "no_text": 0, "merged_words": 0}
        for name, fl in labels["files"].items():
            r = run([tool("pdftotext"), "-enc", "UTF-8", str(d / name), "-"])
            self.assertEqual(r.returncode, 0, name)
            self.assertEqual(r.stderr.strip(), "", name)
            pages = r.stdout.split("\f")[:-1]
            self.assertEqual(len(pages), len(fl["pages"]), name)
            for k, (pg, got) in enumerate(zip(fl["pages"], pages)):
                dt = detail["files"][name]["pages"][k]
                where = f"{name} page {k + 1} ({pg['text_layer']}/{pg['script']}, {dt.get('variant')})"
                if pg["text_layer"] in NO_TEXT:
                    self.assertEqual(got.strip(), "", where)
                    stats["no_text"] += 1
                    continue
                got_t, exp_t = norm_tokens(got), norm_tokens(dt["expected_text"])
                if pg["script"] == "arabic":
                    # bidi reordering moves punctuation and drops the space after it: compare characters
                    self.assertEqual("".join(got_t), "".join(exp_t), where)
                    stats["arabic"] += 1
                elif got_t == exp_t:
                    stats["exact"] += 1
                else:
                    # reading order is poppler's call for two columns and for OCR word boxes; the
                    # words themselves must still all be there
                    ocr = pg["text_layer"] == "invisible_ocr_under_image"
                    self.assertTrue(dt.get("columns", 1) > 1 or ocr, f"{where}: text differs and order is not excused")
                    if sorted(got_t) != sorted(exp_t):
                        # Hidden OCR words are squeezed with Tz to fit their boxes, and poppler sometimes
                        # joins two short neighbours. Every character must still be there.
                        self.assertTrue(ocr, f"{where}: words differ")
                        self.assertEqual(sorted("".join(got_t)), sorted("".join(exp_t)), where)
                        stats["merged_words"] += 1
                    stats["reordered"] += 1
        self.assertGreater(stats["exact"], 15)
        self.assertGreater(stats["no_text"], 10)
        # reordering must stay the exception
        self.assertLess(stats["reordered"], stats["exact"])
        self.assertLessEqual(stats["merged_words"], 3)

    def test_legacy_pages_extract_gibberish_not_devanagari(self):
        _, labels, detail = corpus()
        d = corpus_on_disk()
        n = 0
        for name, fl in labels["files"].items():
            if not any(p["legacy_font"] for p in fl["pages"]):
                continue
            r = run([tool("pdftotext"), "-enc", "UTF-8", str(d / name), "-"])
            for pg, got in zip(fl["pages"], r.stdout.split("\f")[:-1]):
                if pg["legacy_font"]:
                    self.assertEqual(synthtext.script_of(got) - {"latin"}, set(), name)
                    self.assertTrue(got.strip())
                    n += 1
        self.assertGreaterEqual(n, 3)


class SkipWhenToolsMissing(unittest.TestCase):
    def test_tool_tests_skip_not_fail_with_empty_path(self):
        """With an empty PATH the qpdf and poppler tests must be skipped, and the run must succeed."""
        d = Path(tempfile.mkdtemp(prefix="pdf_corpus_nopath_"))
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        code = (
            "import sys, unittest\n"
            f"sys.path.insert(0, {str(Path(__file__).resolve().parent)!r})\n"
            "import test_pdf_corpus as t\n"
            "suite = unittest.TestSuite()\n"
            "for cls in (t.QpdfTests, t.PopplerTests):\n"
            "    suite.addTests(unittest.defaultTestLoader.loadTestsFromTestCase(cls))\n"
            "res = unittest.TextTestRunner(verbosity=0).run(suite)\n"
            "print('RAN', res.testsRun, 'SKIPPED', len(res.skipped))\n"
            "sys.exit(0 if res.wasSuccessful() else 1)\n")
        r = subprocess.run([sys.executable, "-I", "-c", code], capture_output=True, text=True,
                           env={"PATH": str(d)}, timeout=120)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        m = re.search(r"RAN (\d+) SKIPPED (\d+)", r.stdout)
        self.assertIsNotNone(m, r.stdout)
        ran, skipped = int(m.group(1)), int(m.group(2))
        self.assertEqual(ran, skipped, "every tool-dependent test should skip when the tools are missing")
        self.assertGreaterEqual(ran, 4)


if __name__ == "__main__":
    unittest.main()
