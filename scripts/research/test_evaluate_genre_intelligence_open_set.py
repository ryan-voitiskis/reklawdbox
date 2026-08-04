import unittest

import numpy as np

import evaluate_genre_intelligence_open_set as subject


class EvaluateGenreIntelligenceOpenSetTests(unittest.TestCase):
    def test_o2_collision_and_zero_qualification_abstain(self) -> None:
        scores = np.zeros((3, len(subject.preparation.OUTPUT_PARENTS)))
        scores[0, 0] = 0.9
        scores[1, :2] = 0.9
        thresholds = [
            {"threshold": 0.5 if index < 2 else None}
            for index in range(len(subject.preparation.OUTPUT_PARENTS))
        ]
        predictions, offered, counts = subject.apply_o2(
            scores, thresholds, np.ones(3, dtype=bool)
        )
        np.testing.assert_array_equal(predictions, np.asarray([0, -1, -1]))
        np.testing.assert_array_equal(offered, np.asarray([True, False, False]))
        self.assertEqual(
            counts,
            {"zero_qualified": 1, "one_qualified": 1, "multi_qualified": 1},
        )

    def test_metrics_count_non_target_offer_as_exact_error(self) -> None:
        truth_names = ["House", "Minimal", "Techno"]
        truths = np.asarray([0, -1, 2])
        predictions = np.asarray([0, 0, 2])
        offered = np.ones(3, dtype=bool)
        folds = np.asarray([0, 1, 2])
        result = subject.metrics(
            truth_names, truths, predictions, offered, folds
        )
        self.assertEqual(result["offers"], 3)
        self.assertEqual(result["correct_offers"], 2)
        self.assertEqual(result["non_target"]["support"], 1)
        self.assertEqual(result["non_target"]["false_offers"], 1)
        self.assertEqual(result["non_target"]["false_offer_rate"], 1.0)

    def test_gate_requires_every_preregistered_condition(self) -> None:
        targets = {
            parent: {
                "offers": 8 if index < 4 else 0,
                "offered_precision": 0.875 if index < 4 else 0.0,
            }
            for index, parent in enumerate(subject.preparation.OUTPUT_PARENTS)
        }
        candidate = {
            "offers": 180,
            "coverage": 180 / 716,
            "offered_precision": 0.90,
            "non_target": {"false_offer_rate": 0.10},
            "folds": [{"offers": 36, "offered_precision": 0.85}] * 5,
            "per_target": targets,
        }
        paired = {"precision_improvement": 0.05}
        self.assertTrue(subject.gate(candidate, paired)["passed"])
        candidate["folds"][0] = {"offers": 19, "offered_precision": 1.0}
        self.assertFalse(subject.gate(candidate, paired)["passed"])

    def test_threshold_calibration_is_parent_specific(self) -> None:
        predictions = np.asarray([0] * 8 + [1] * 8)
        margins = np.asarray([0.9] * 8 + [0.4] * 8)
        truths = predictions.copy()
        details = subject.calibrate_o1(
            predictions, margins, truths, np.ones(16, dtype=bool)
        )
        self.assertEqual(details[0]["threshold"], 0.9)
        self.assertEqual(details[1]["threshold"], 0.4)
        self.assertIsNone(details[2]["threshold"])


if __name__ == "__main__":
    unittest.main()
