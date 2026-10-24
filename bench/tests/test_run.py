"""End to end: bench/run.py over the golden inputs must reproduce the golden reports."""
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))

try:
    import pagetriage_py  # noqa: F401
    import run
except ImportError as e:  # binding or Pillow not installed
    run = None
    SKIP_REASON = f"needs the pagetriage_py binding and Pillow: {e}"
else:
    SKIP_REASON = ""

GOLDEN = ROOT / "tests" / "golden"


@unittest.skipIf(run is None, SKIP_REASON)
class RunTests(unittest.TestCase):
    def run_main(self, *args):
        err = io.StringIO()
        with redirect_stderr(err):
            code = run.main(list(args))
        return code, err.getvalue()

    def test_reproduces_golden_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "r.jsonl"
            code, _ = self.run_main("--images", str(GOLDEN / "inputs"), "--glob", "*.pgm", "--debug", "--out", str(out))
            self.assertEqual(code, 0)
            lines = [json.loads(l) for l in out.read_text().splitlines()]
        self.assertGreaterEqual(len(lines), 5)
        for line in lines:
            name = Path(line["image"]).stem
            expected = json.loads((GOLDEN / "expected" / f"{name}.json").read_text())
            self.assertEqual(line["report"], expected, name)
            self.assertEqual((line["width"], line["height"]), (64, 80))

    def test_colour_images_go_through_the_core_luma(self):
        from PIL import Image

        with tempfile.TemporaryDirectory() as tmp:
            Image.new("RGB", (16, 16), (250, 250, 250)).save(Path(tmp) / "white.png")
            out = Path(tmp) / "r.jsonl"
            code, _ = self.run_main("--images", tmp, "--glob", "*.png", "--out", str(out))
            self.assertEqual(code, 0)
            report = json.loads(out.read_text())["report"]
        self.assertEqual(report["findings"][0]["kind"], "blank_page")

    def test_undecodable_files_are_skipped_and_empty_runs_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "junk.png").write_bytes(b"not an image")
            code, err = self.run_main("--images", tmp, "--glob", "*.png", "--out", str(Path(tmp) / "o.jsonl"))
            self.assertEqual(code, 1, "nothing analysed is a failure, not a silent success")
            self.assertIn("skip junk.png", err)

    def test_bad_arguments(self):
        self.assertEqual(self.run_main("--images", "/nonexistent-dir")[0], 2)
        self.assertEqual(self.run_main("--images", str(GOLDEN), "--opts", "{oops")[0], 2)


if __name__ == "__main__":
    unittest.main()
