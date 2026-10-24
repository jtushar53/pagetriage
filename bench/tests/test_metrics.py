"""Metrics are checked against values worked out by hand, not against another implementation."""
import math
import sys
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import metrics  # noqa: E402


class BinarisationTests(unittest.TestCase):
    def setUp(self):
        # 10 pixels: ground truth has ink at 0..3 (4 px). Prediction has ink at 2..5 (4 px).
        self.gt = np.zeros(10, dtype=bool)
        self.gt[0:4] = True
        self.pred = np.zeros(10, dtype=bool)
        self.pred[2:6] = True

    def test_precision_recall_f_measure(self):
        # tp = {2,3} = 2, fp = {4,5} = 2, fn = {0,1} = 2 -> P = R = 0.5, FM = 50
        self.assertEqual(metrics.precision_recall(self.pred, self.gt), (0.5, 0.5))
        self.assertAlmostEqual(metrics.f_measure(self.pred, self.gt), 50.0)

    def test_psnr(self):
        # 4 of 10 pixels differ: MSE = 0.4, PSNR = 10 log10(1 / 0.4)
        self.assertAlmostEqual(metrics.psnr(self.pred, self.gt), 10 * math.log10(2.5))
        self.assertEqual(metrics.psnr(self.gt, self.gt), math.inf)

    def test_perfect_and_empty_predictions(self):
        self.assertAlmostEqual(metrics.f_measure(self.gt, self.gt), 100.0)
        self.assertEqual(metrics.f_measure(np.zeros(10, dtype=bool), self.gt), 0.0)
        # No ink anywhere in either image: defined as 0, not a division error.
        self.assertEqual(metrics.f_measure(np.zeros(10, dtype=bool), np.zeros(10, dtype=bool)), 0.0)

    def test_ink_mask_treats_dark_as_ink(self):
        self.assertEqual(metrics.ink_mask(np.array([0, 127, 128, 255])).tolist(), [True, True, False, False])

    def test_shape_mismatch_is_an_error(self):
        with self.assertRaises(ValueError):
            metrics.f_measure(np.zeros(3, dtype=bool), np.zeros(4, dtype=bool))

    def test_unimplemented_metrics_say_so(self):
        with self.assertRaises(NotImplementedError):
            metrics.drd(None, None)
        with self.assertRaises(NotImplementedError):
            metrics.pseudo_f_measure(None, None, None)


class DetectionTests(unittest.TestCase):
    def test_f1_from_counts(self):
        self.assertAlmostEqual(metrics.f1_from_counts(8, 2, 2), 0.8)
        self.assertEqual(metrics.f1_from_counts(0, 0, 0), 0.0)

    def test_iou(self):
        self.assertEqual(metrics.iou([0, 0, 10, 10], [0, 0, 10, 10]), 1.0)
        self.assertEqual(metrics.iou([0, 0, 10, 10], [10, 10, 20, 20]), 0.0)
        # overlap 5x10 = 50, union 100 + 100 - 50 = 150
        self.assertAlmostEqual(metrics.iou([0, 0, 10, 10], [5, 0, 15, 10]), 50 / 150)
        self.assertEqual(metrics.iou([0, 0, 0, 0], [0, 0, 0, 0]), 0.0)

    def test_match_boxes_is_one_to_one(self):
        gt = [[0, 0, 10, 10], [50, 50, 60, 60]]
        pred = [[0, 0, 10, 10], [1, 1, 11, 11], [100, 100, 110, 110]]
        # pred[0] matches gt[0]; pred[1] also overlaps gt[0] but it is taken; pred[2] matches nothing.
        self.assertEqual(metrics.match_boxes(pred, gt, 0.5), (1, 2, 1))


class CalibrationTests(unittest.TestCase):
    def test_brier(self):
        self.assertAlmostEqual(metrics.brier([0.9, 0.2], [1, 0]), (0.01 + 0.04) / 2)

    def test_ece_perfectly_calibrated_is_zero(self):
        # 10 samples at confidence 0.7, exactly 7 correct
        conf = [0.7] * 10
        outcome = [1] * 7 + [0] * 3
        self.assertAlmostEqual(metrics.ece(conf, outcome), 0.0)

    def test_ece_overconfident(self):
        # all at 0.9 but only half right: |0.5 - 0.9| = 0.4 in the single occupied bin
        self.assertAlmostEqual(metrics.ece([0.9] * 4, [1, 1, 0, 0]), 0.4)

    def test_ece_confidence_one_lands_in_the_last_bin(self):
        self.assertAlmostEqual(metrics.ece([1.0, 1.0], [1, 0]), 0.5)

    def test_bad_inputs(self):
        with self.assertRaises(ValueError):
            metrics.ece([1.2], [1])
        with self.assertRaises(ValueError):
            metrics.brier([0.5], [1, 0])
        with self.assertRaises(ValueError):
            metrics.ece([], [])


if __name__ == "__main__":
    unittest.main()
