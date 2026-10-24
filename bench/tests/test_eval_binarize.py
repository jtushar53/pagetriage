"""Offline checks on bench/eval_binarize.py: the hand-copied published tables and the manifest."""
import statistics
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import eval_binarize as eb  # noqa: E402

# Howe 2013 prints Hand / Print / All averages under each table; the copied per-image numbers must
# reproduce them, which catches transcription slips.
PAPER_TABLE3 = {
    "H": (90.5, 94.9, 93.3, 89.6, 93.3),
    "P": (87.9, 93.9, 90.0, 90.3, 90.5),
    "": (89.2, 94.4, 91.7, 90.0, 91.9),
}
PAPER_TABLE1 = {
    "H": (92.3, 94.7, 93.3, 94.3, 94.1),
    "P": (93.9, 95.8, 93.9, 95.6, 94.6),
    "": (92.7, 95.0, 93.5, 94.7, 94.3),
}
# The four 2009 training images are not in the downloaded test sets, but they are in the paper's
# averages, so they are added back for this check.
TRAINING_ROWS = {
    "H2009-T1": (96.4, 96.6, 96.6, 96.1, 96.6),
    "H2009-T2": (93.3, 93.4, 92.5, 93.1, 93.1),
    "P2009-T1": (88.5, 97.4, 96.7, 97.3, 97.2),
    "P2009-T2": (98.0, 98.5, 98.1, 98.4, 98.5),
}


def column_means(table, prefix):
    rows = [v for k, v in table.items() if k.startswith(prefix)]
    return tuple(statistics.fmean(c) for c in zip(*rows))


class PublishedTables(unittest.TestCase):
    def test_table3_reproduces_the_printed_averages(self):
        for prefix, printed in PAPER_TABLE3.items():
            for ours, theirs in zip(column_means(eb.TABLE3, prefix), printed):
                self.assertAlmostEqual(ours, theirs, delta=0.051, msg=prefix or "All")

    def test_table1_reproduces_the_printed_averages(self):
        table = {**eb.TABLE1, **TRAINING_ROWS}
        for prefix, printed in PAPER_TABLE1.items():
            for ours, theirs in zip(column_means(table, prefix), printed):
                # The paper rounds averages of unrounded values; 0.15 covers that.
                self.assertAlmostEqual(ours, theirs, delta=0.15, msg=prefix or "All")

    def test_every_published_row_has_five_columns(self):
        for table in (eb.TABLE1, eb.TABLE3):
            for key, row in table.items():
                self.assertEqual(len(row), 5, key)
                self.assertTrue(all(0 < v <= 100 for v in row), key)


class Manifest(unittest.TestCase):
    def test_counts_and_unique_keys(self):
        manifest = eb._manifest()
        self.assertEqual([len(manifest[k]) for k in ("dibco2009", "dibco2010", "dibco2011")], [10, 10, 16])
        keys = [key for rows in manifest.values() for key, _, _ in rows]
        self.assertEqual(len(keys), len(set(keys)))

    def test_every_manifest_image_with_a_published_number_is_in_a_table(self):
        manifest = eb._manifest()
        for rows in manifest.values():
            for key, _, _ in rows:
                self.assertTrue(key in eb.TABLE1 or key in eb.TABLE3, key)


if __name__ == "__main__":
    unittest.main()
