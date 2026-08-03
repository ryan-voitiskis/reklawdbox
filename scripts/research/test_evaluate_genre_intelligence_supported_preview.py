import unittest

import numpy as np

import evaluate_genre_intelligence_supported_preview as subject


class EvaluateGenreIntelligenceSupportedPreviewTests(unittest.TestCase):
    def test_restricted_thresholds_fail_closed_for_unsupported_parents(self) -> None:
        values = {
            index: {"threshold": 0.1, "offers": 8, "offered_precision": 0.9}
            for index in range(len(subject.candidate_a.RELEASE_SCOPE))
        }
        selected = subject.restricted_thresholds(values)
        for index, value in selected.items():
            self.assertEqual(value is not None, index in subject.SUPPORTED_INDICES)

    def test_exclusion_mask_rejects_unknown_row(self) -> None:
        development = {"rows": [{"row_id": "known"}]}
        exclusions = {
            "stage": "private_holdout_group_development_exclusions",
            "rows": [{"row_id": "unknown", "reasons": ["artist_group"]}],
        }
        with self.assertRaisesRegex(ValueError, "unknown row identities"):
            subject.exclusion_mask(development, exclusions)

    def test_quality_gate_checks_only_supported_target_offers(self) -> None:
        metrics = {
            "offered_precision": 0.92,
            "folds": [{"offers": 10, "offered_precision": 0.9}] * 5,
            "per_target": {
                parent: {
                    "offers": 8 if parent in subject.SUPPORTED_PARENTS else 100,
                    "offered_precision": (
                        0.85 if parent in subject.SUPPORTED_PARENTS else 0.0
                    ),
                }
                for parent in subject.candidate_a.RELEASE_SCOPE
            },
        }
        paired = {"precision_improvement": 0.10}
        self.assertTrue(subject.quality_gate(metrics, paired)["passed"])
        metrics["per_target"]["Ambient"]["offered_precision"] = 0.75
        self.assertFalse(subject.quality_gate(metrics, paired)["passed"])

    def test_exclusion_mask_preserves_manifest_order(self) -> None:
        development = {"rows": [{"row_id": "one"}, {"row_id": "two"}]}
        exclusions = {
            "stage": "private_holdout_group_development_exclusions",
            "rows": [{"row_id": "one", "reasons": ["artist_group"]}],
        }
        np.testing.assert_array_equal(
            subject.exclusion_mask(development, exclusions),
            np.asarray([False, True]),
        )


if __name__ == "__main__":
    unittest.main()
