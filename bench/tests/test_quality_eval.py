"""Offline checks on bench/skew_eval.py and bench/quality_eval.py: the tuning split, the angle list,
the label lists and the small statistics helpers. No dataset is needed."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import quality_eval as qe  # noqa: E402
import skew_eval as se  # noqa: E402


class SplitAndLabels(unittest.TestCase):
    def test_tuning_and_held_out_sets_partition_the_datasets(self):
        self.assertEqual(set(se.TUNE) & set(se.HELDOUT), set())
        self.assertEqual(set(se.TUNE) | set(se.HELDOUT), set(se.SETS))
        self.assertTrue(se.TUNE and se.HELDOUT)

    def test_angles_are_fixed_distinct_and_within_ten_degrees(self):
        for angles in (se.ANGLES, se.ANGLES_AUGRAPHY):
            self.assertEqual(len(set(angles)), len(angles))
            self.assertTrue(all(-10 <= a <= 10 for a in angles))
        self.assertTrue(all(float(a).is_integer() for a in se.ANGLES_AUGRAPHY))

    def test_label_lists_name_real_sets_and_do_not_overlap(self):
        for table in (qe.STRONG_BLEED, qe.CLEAN_BLEED):
            self.assertTrue(set(table) <= set(se.SETS))
        for key in se.SETS:
            both = set(qe.STRONG_BLEED.get(key, [])) & set(qe.CLEAN_BLEED.get(key, []))
            self.assertEqual(both, set(), key)

    def test_label_of(self):
        self.assertEqual(qe.label_of("dibco2016", "3.bmp"), 1)
        self.assertEqual(qe.label_of("dibco2016", "4.bmp"), 0)
        self.assertIsNone(qe.label_of("dibco2016", "7.bmp"))
        self.assertIsNone(qe.label_of("dibco2099", "1.bmp"))


class Statistics(unittest.TestCase):
    def test_auc(self):
        self.assertEqual(qe.auc([3, 4], [1, 2]), 1.0)
        self.assertEqual(qe.auc([1, 2], [3, 4]), 0.0)
        self.assertEqual(qe.auc([1], [1]), 0.5)
        self.assertTrue(qe.auc([], [1]) != qe.auc([], [1]))  # NaN

    def test_error_stats(self):
        s = se.stats([0.1, -0.2, 0.0, 1.5, -2.0])
        self.assertEqual(s["n"], 5)
        self.assertEqual(s["median_abs"], 0.2)
        self.assertEqual(s["max_abs"], 2.0)
        self.assertEqual((s["over_tolerance"], s["over_gross"]), (2, 2))
        self.assertEqual(se.stats([]), {})


if __name__ == "__main__":
    unittest.main()
