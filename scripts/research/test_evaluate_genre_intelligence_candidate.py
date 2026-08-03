import unittest

import numpy as np

import evaluate_genre_intelligence_candidate as subject


class EvaluateGenreIntelligenceCandidateTests(unittest.TestCase):
    def test_imputation_uses_only_training_partition(self) -> None:
        matrix = np.asarray([[1.0, np.nan], [3.0, 2.0], [1000.0, 1000.0]])
        train = np.asarray([True, True, False])
        first, active = subject.impute_and_standardize(matrix, train)
        changed = matrix.copy()
        changed[2] = [-1000.0, -1000.0]
        second, changed_active = subject.impute_and_standardize(changed, train)
        np.testing.assert_array_equal(active, changed_active)
        np.testing.assert_allclose(first[:2], second[:2])

    def test_paired_baseline_compares_only_candidate_offers_with_baseline(self) -> None:
        truths = np.asarray([0, 1, 2, 3])
        candidate = np.asarray([0, 1, 2, 0])
        offered = np.asarray([True, True, False, True])
        baseline = np.asarray([1, 1, 2, -1])
        result = subject.paired_baseline(truths, candidate, offered, baseline)
        self.assertEqual(result["paired_offers"], 2)
        self.assertEqual(result["candidate_offered_precision"], 1.0)
        self.assertEqual(result["v033_offered_precision"], 0.5)

    def test_gate_requires_coverage_fold_target_and_baseline_checks(self) -> None:
        candidate = {
            "offered_precision": 0.91,
            "coverage": 0.70,
            "folds": [{"offers": 10, "offered_precision": 0.90}] * 5,
            "per_target": {
                target: {"offers": 8, "offered_precision": 0.85}
                for target in subject.RELEASE_SCOPE
            },
        }
        paired = {"precision_improvement": 0.06}
        self.assertTrue(subject.gate(candidate, paired)["passed"])
        paired["precision_improvement"] = 0.04
        self.assertFalse(subject.gate(candidate, paired)["passed"])


if __name__ == "__main__":
    unittest.main()
