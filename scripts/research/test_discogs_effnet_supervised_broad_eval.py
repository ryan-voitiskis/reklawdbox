#!/usr/bin/env python3
"""Unit tests for the frozen Plan 064 broad adapter evaluation."""

from __future__ import annotations

import unittest

import numpy as np

import discogs_effnet_broad_eval as broad
import discogs_effnet_supervised_broad_eval as subject


class SupervisedBroadEvaluationTests(unittest.TestCase):
    def test_balanced_weights_give_each_class_equal_total_weight(self) -> None:
        truths = np.asarray([0, 0, 0, 1], dtype=np.int64)
        weights = subject.balanced_weights(truths)
        self.assertAlmostEqual(float(np.sum(weights[truths == 0])), 2.0)
        self.assertAlmostEqual(float(np.sum(weights[truths == 1])), 2.0)
        self.assertAlmostEqual(float(np.mean(weights)), 1.0)

    def test_baseline_one_hot_maps_fine_labels_and_zeros_unmodeled(self) -> None:
        values = subject.baseline_broad_one_hot(
            ["Deep House", "Jungle", "Experimental", None]
        )
        self.assertEqual(values.shape, (4, len(broad.BROAD_TARGETS)))
        self.assertEqual(values[0, broad.BROAD_INDEX["House"]], 1.0)
        self.assertEqual(values[1, broad.BROAD_INDEX["Drum & Bass"]], 1.0)
        self.assertEqual(float(np.sum(values[2])), 0.0)
        self.assertEqual(float(np.sum(values[3])), 0.0)

    def test_imputation_uses_training_partition_only(self) -> None:
        arrangement = np.asarray([[1.0], [3.0], [100.0], [np.nan]])
        imputed = subject.impute_arrangement(
            arrangement, np.asarray([True, True, False, False])
        )
        self.assertEqual(imputed[3, 0], 2.0)

    def test_pca_centers_on_training_partition(self) -> None:
        embeddings = np.asarray([[0.0, 0.0], [2.0, 0.0], [100.0, 0.0]])
        projected = subject.pca_projection(
            embeddings, np.asarray([True, True, False]), 1
        )
        self.assertAlmostEqual(float(np.mean(projected[:2, 0])), 0.0, places=12)
        self.assertGreater(abs(float(projected[2, 0])), 90.0)

    def test_ridge_rejects_overlapping_partitions(self) -> None:
        features = np.asarray([[0.0], [1.0], [2.0]])
        truths = np.asarray([0, 1, 1])
        with self.assertRaisesRegex(ValueError, "overlap"):
            subject.ridge_score_split(
                features,
                truths,
                np.asarray([True, True, False]),
                np.asarray([False, True, True]),
            )

    def test_prediction_tie_uses_class_order(self) -> None:
        predictions, margins = subject.predictions_and_margins(
            np.asarray([[0.5, 0.5], [0.2, 0.8]]), [3, 7]
        )
        np.testing.assert_array_equal(predictions, [3, 7])
        np.testing.assert_allclose(margins, [0.0, 0.6])

    def test_nested_cross_fit_scores_each_outer_row_once(self) -> None:
        rows_per_fold = 4
        folds = np.repeat(np.arange(5), rows_per_fold)
        truths = np.tile(np.asarray([0, 0, 1, 1]), 5)
        signal = truths.astype(np.float64)[:, None]
        style = np.column_stack([1.0 - signal, signal])
        baseline = style.copy()
        arrangement = np.column_stack([signal, signal, signal, signal])
        embeddings = np.column_stack([signal, 1.0 - signal])
        predictions, margins, offered, details = subject.nested_cross_fitted_offers(
            style, baseline, arrangement, embeddings, truths, folds
        )
        self.assertEqual(len(predictions), len(truths))
        self.assertTrue(np.all(predictions >= 0))
        self.assertTrue(np.all(margins >= 0.0))
        self.assertFalse(np.any(offered))
        self.assertEqual(len(details), 5)
        self.assertTrue(all(row["threshold"] is None for row in details))

    def test_selective_gate_is_unchanged_from_broad_evaluation(self) -> None:
        unselective = {"offered_precision": 0.79}
        candidate = {
            "offered_precision": 0.90,
            "coverage": 0.50,
            "folds": [{"offers": 1, "offered_precision": 0.85}],
            "per_target": {
                "House": {"support": 10, "offers": 5, "offered_precision": 0.75}
            },
        }
        self.assertTrue(broad.gate(unselective, candidate)["passed"])


if __name__ == "__main__":
    unittest.main()
