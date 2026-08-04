import unittest

import numpy as np

import evaluate_genre_intelligence_precision_buffer as subject


class EvaluateGenreIntelligencePrecisionBufferTests(unittest.TestCase):
    def test_threshold_requires_preregistered_95_percent_precision(self) -> None:
        values = np.arange(20, dtype=np.float64)
        correct = np.ones(20, dtype=bool)
        correct[0] = False
        selected = subject.choose_threshold(values, correct)
        self.assertIsNotNone(selected)
        assert selected is not None
        self.assertEqual(selected["offers"], 20)
        self.assertEqual(selected["threshold"], 0.0)
        self.assertEqual(selected["offered_precision"], 0.95)

    def test_threshold_does_not_accept_only_90_percent_precision(self) -> None:
        values = np.zeros(10, dtype=np.float64)
        correct = np.asarray([False, *([True] * 9)])
        selected = subject.choose_threshold(values, correct)
        self.assertIsNone(selected)

    def test_calibration_is_parent_specific_and_disables_sparse_parent(self) -> None:
        target_count = len(subject.plan071.preparation.OUTPUT_PARENTS)
        scores = np.zeros((16, target_count), dtype=np.float64)
        truths = np.asarray([0] * 8 + [1] * 8)
        scores[:8, 0] = 0.8
        scores[8:, 1] = 0.7
        details = subject.calibrate(scores, truths, np.ones(16, dtype=bool))
        self.assertEqual(details[0]["threshold"], 0.8)
        self.assertEqual(details[1]["threshold"], 0.7)
        self.assertIsNone(details[2]["threshold"])


if __name__ == "__main__":
    unittest.main()
