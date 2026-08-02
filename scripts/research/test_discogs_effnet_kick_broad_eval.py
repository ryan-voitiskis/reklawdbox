#!/usr/bin/env python3
"""Unit tests for the frozen Plan 065 kick-augmented evaluation."""

from __future__ import annotations

import unittest

import numpy as np

import discogs_effnet_kick_broad_eval as subject
import extract_kick_rhythm_features as kick


class KickAugmentedEvaluationTests(unittest.TestCase):
    def test_semantic_hash_depends_on_values_and_schema(self) -> None:
        features = np.zeros((2, kick.FEATURE_COUNT), dtype=np.float64)
        first = subject.kick_semantic_sha256(features)
        features[1, 0] = 1.0
        second = subject.kick_semantic_sha256(features)
        self.assertNotEqual(first, second)

    def test_augmented_features_append_every_kick_column(self) -> None:
        rows = 4
        style = np.zeros((rows, 2))
        baseline = np.zeros((rows, 2))
        arrangement = np.zeros((rows, 4))
        embeddings = np.column_stack([np.arange(rows), np.arange(rows) ** 2])
        kick_features = np.arange(rows * 3, dtype=np.float64).reshape(rows, 3)
        train = np.asarray([True, True, True, False])
        features = subject.augmented_fold_features(
            style, baseline, arrangement, embeddings, kick_features, train
        )
        np.testing.assert_array_equal(features[:, -3:], kick_features)

    def test_augmented_features_reject_row_mismatch(self) -> None:
        rows = 4
        with self.assertRaisesRegex(ValueError, "row counts differ"):
            subject.augmented_fold_features(
                np.zeros((rows, 2)),
                np.zeros((rows, 2)),
                np.zeros((rows, 4)),
                np.zeros((rows, 2)),
                np.zeros((rows - 1, 3)),
                np.asarray([True, True, True, False]),
            )

    def test_nested_cross_fit_scores_every_outer_row(self) -> None:
        rows_per_fold = 4
        folds = np.repeat(np.arange(5), rows_per_fold)
        truths = np.tile(np.asarray([0, 0, 1, 1]), 5)
        signal = truths.astype(np.float64)[:, None]
        style = np.column_stack([1.0 - signal, signal])
        baseline = style.copy()
        arrangement = np.column_stack([signal, signal, signal, signal])
        embeddings = np.column_stack([signal, 1.0 - signal])
        kick_features = np.column_stack([signal, 1.0 - signal])
        predictions, margins, offered, details = subject.nested_cross_fitted_offers(
            style,
            baseline,
            arrangement,
            embeddings,
            kick_features,
            truths,
            folds,
        )
        self.assertEqual(len(predictions), len(truths))
        self.assertTrue(np.all(predictions >= 0))
        self.assertTrue(np.all(margins >= 0.0))
        self.assertFalse(np.any(offered))
        self.assertEqual(len(details), 5)


if __name__ == "__main__":
    unittest.main()
