import unittest

import numpy as np

import evaluate_genre_intelligence_target_calibration as subject


class EvaluateGenreIntelligenceTargetCalibrationTests(unittest.TestCase):
    def test_target_thresholds_fail_closed_and_maximize_qualified_offers(self) -> None:
        predictions = np.asarray([0] * 10 + [1] * 10)
        margins = np.asarray([float(index) for index in range(10)] * 2)
        truths = np.asarray([0] * 9 + [1] + [1] * 7 + [0] * 3)
        selected = subject.choose_target_thresholds(predictions, margins, truths)
        self.assertEqual(selected[0]["offers"], 10)
        self.assertIsNone(selected[1])
        for target_index in range(2, len(subject.candidate_a.RELEASE_SCOPE)):
            self.assertIsNone(selected[target_index])

    def test_application_uses_only_prediction_specific_threshold(self) -> None:
        predictions = np.asarray([0, 0, 1, 1])
        margins = np.asarray([0.2, 0.8, 0.2, 0.8])
        thresholds = {
            0: {"threshold": 0.5, "offers": 1, "offered_precision": 1.0},
            1: None,
        }
        for target_index in range(2, len(subject.candidate_a.RELEASE_SCOPE)):
            thresholds[target_index] = None
        np.testing.assert_array_equal(
            subject.apply_target_thresholds(predictions, margins, thresholds),
            np.asarray([False, True, False, False]),
        )

    def test_extended_gate_requires_four_calibrated_targets(self) -> None:
        metrics = {
            "offered_precision": 0.95,
            "coverage": 0.70,
            "folds": [{"offers": 10, "offered_precision": 0.90}] * 5,
            "per_target": {
                target: {"offers": 8, "offered_precision": 0.85}
                for target in subject.candidate_a.RELEASE_SCOPE
            },
        }
        paired = {"precision_improvement": 0.10}
        self.assertFalse(subject.extended_gate(metrics, paired, 3)["passed"])
        self.assertTrue(subject.extended_gate(metrics, paired, 4)["passed"])


if __name__ == "__main__":
    unittest.main()
