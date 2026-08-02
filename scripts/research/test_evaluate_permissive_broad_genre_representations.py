from __future__ import annotations

import unittest
from unittest.mock import patch

import numpy as np

import evaluate_permissive_broad_genre_representations as evaluation


class PermissiveRepresentationEvaluationTests(unittest.TestCase):
    def test_current_rows_must_be_ordered_source_subset(self) -> None:
        source = [
            {"file_path": "/a", "truth": "House", "fold": 0},
            {"file_path": "/b", "truth": "Techno", "fold": 1},
            {"file_path": "/c", "truth": "Ambient", "fold": 2},
        ]
        current = [source[0], source[2]]
        np.testing.assert_array_equal(
            evaluation.current_source_indices(current, source), [0, 2]
        )
        fold_changed = [dict(source[0], fold=4), source[2]]
        np.testing.assert_array_equal(
            evaluation.current_source_indices(fold_changed, source), [0, 2]
        )
        with self.assertRaisesRegex(ValueError, "ordered source subset"):
            evaluation.current_source_indices([source[2], source[0]], source)
        with self.assertRaisesRegex(ValueError, "truth differs"):
            evaluation.current_source_indices(
                [dict(source[0], truth="Techno"), source[2]], source
            )

    def test_new_representation_is_projected_from_training_partition(self) -> None:
        rows = 10
        train = np.asarray([True] * 8 + [False] * 2)
        style = np.zeros((rows, 3))
        baseline = np.zeros((rows, 3))
        arrangement = np.zeros((rows, 4))
        effnet = np.arange(rows * 6, dtype=np.float64).reshape(rows, 6)
        kick = np.zeros((rows, 74))
        representation = np.arange(rows * 5, dtype=np.float64).reshape(rows, 5)
        first = evaluation.augmented_fold_features(
            style, baseline, arrangement, effnet, kick, representation, train
        )
        changed = representation.copy()
        changed[~train] += 100_000
        second = evaluation.augmented_fold_features(
            style, baseline, arrangement, effnet, kick, changed, train
        )
        np.testing.assert_allclose(first[train], second[train])
        self.assertFalse(np.allclose(first[~train], second[~train]))

    def test_tie_break_prefers_stability_before_runtime(self) -> None:
        def candidate(minimum_fold: float, precision: float) -> dict:
            return {
                "deployment": {
                    "metrics": {
                        "folds": [
                            {"offered_precision": minimum_fold},
                            {"offered_precision": 1.0},
                        ],
                        "per_target": {
                            "House": {
                                "support": 20,
                                "offers": 10,
                                "offered_precision": 0.8,
                            }
                        },
                        "offered_precision": precision,
                        "coverage": 0.5,
                    }
                }
            }

        openl3 = candidate(0.86, 0.95)
        clap = candidate(0.87, 0.90)
        self.assertGreater(
            evaluation.candidate_tie_key("clap", clap),
            evaluation.candidate_tie_key("openl3", openl3),
        )
        self.assertGreater(
            evaluation.candidate_tie_key("openl3", clap),
            evaluation.candidate_tie_key("clap", clap),
        )

    def test_candidate_must_pass_nested_and_deployment_gates(self) -> None:
        truths = np.asarray([0, 1])
        folds = np.asarray([0, 0])
        unselective = {"folds": [{"fold": 0}]}
        selective = {
            "folds": [
                {
                    "eligible_rows": 2,
                    "offers": 1,
                    "coverage": 0.5,
                    "offered_precision": 1.0,
                }
            ]
        }
        deployment = {"folds": [{"fold": 0}]}
        with (
            patch.object(
                evaluation,
                "nested_cross_fitted_offers",
                return_value=(
                    np.asarray([0, 1]),
                    np.asarray([0.9, 0.8]),
                    np.asarray([True, False]),
                    [{"fold": 0}],
                ),
            ),
            patch.object(
                evaluation.broad,
                "metrics",
                side_effect=[unselective, selective, deployment],
            ),
            patch.object(
                evaluation.broad,
                "choose_threshold",
                return_value={"threshold": 0.8, "offers": 2},
            ),
            patch.object(
                evaluation.broad,
                "gate",
                side_effect=[{"passed": True}, {"passed": False}],
            ),
        ):
            candidate = evaluation.evaluate_candidate(
                np.zeros((2, 1)),
                np.zeros((2, 1)),
                np.zeros((2, 1)),
                np.zeros((2, 1)),
                np.zeros((2, 1)),
                np.zeros((2, 1)),
                truths,
                folds,
            )
        self.assertFalse(candidate["passed"])


if __name__ == "__main__":
    unittest.main()
