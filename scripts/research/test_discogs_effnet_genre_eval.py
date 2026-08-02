#!/usr/bin/env python3
"""Synthetic checks for the isolated Discogs-EffNet genre evaluation."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


MODULE_PATH = Path(__file__).with_name("discogs_effnet_genre_eval.py")
SPEC = importlib.util.spec_from_file_location("discogs_effnet_genre_eval", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class DiscogsEffnetGenreEvalTests(unittest.TestCase):
    def test_style_mapping_uses_category_context(self) -> None:
        self.assertEqual(MODULE.class_to_canonical("Electronic---Hardcore"), "Hardcore")
        self.assertEqual(MODULE.class_to_canonical("Rock---Hardcore"), "Rock")
        self.assertEqual(MODULE.class_to_canonical("Reggae---Dancehall"), "Dancehall")
        self.assertEqual(MODULE.class_to_canonical("Electronic---Drum n Bass"), "Drum & Bass")
        self.assertIsNone(MODULE.class_to_canonical("Electronic---Drone"))
        self.assertEqual(MODULE.family("Experimental"), "Downtempo")

    def test_style_projection_takes_maximum_synonym_probability(self) -> None:
        classes = ["Electronic---Disco", "Funk / Soul---Disco", "Electronic---Techno"]
        values = np.asarray([[0.2, 0.8, 0.4]], dtype=np.float32)
        scores = MODULE.canonical_style_scores(values, classes)
        self.assertAlmostEqual(float(scores[0, MODULE.CANONICAL_INDEX["Disco"]]), 0.8)
        self.assertAlmostEqual(float(scores[0, MODULE.CANONICAL_INDEX["Techno"]]), 0.4)

    def test_metrics_keep_macro_and_accuracy_denominators_distinct(self) -> None:
        metrics = MODULE.aggregate_metrics(
            ["House", "House", "Techno"],
            ["House", "Techno", "Deep Techno"],
            np.asarray([0, 1, 2]),
        )
        self.assertAlmostEqual(metrics["exact_accuracy"], 1 / 3)
        self.assertAlmostEqual(metrics["per_genre"]["House"]["recall"], 0.5)
        self.assertEqual(metrics["per_genre"]["Techno"]["recall"], 0.0)
        self.assertAlmostEqual(metrics["same_family_accuracy"], 2 / 3)
        self.assertEqual(len(metrics["folds"]), 3)

    def test_fold_centroid_excludes_held_out_values(self) -> None:
        embeddings = MODULE.l2_normalize(
            np.asarray([[1.0, 0.0], [1.0, 0.0], [-1.0, 0.0]], dtype=np.float32)
        )
        arrangement = np.asarray([[0.0, 0.0], [2.0, 2.0], [100.0, 100.0]])
        truths = ["House", "House", "House"]
        folds = np.asarray([0, 0, 1])
        embedding_centroids, arrangement_centroids, _, means, _ = MODULE.fit_fold_centroids(
            embeddings, arrangement, truths, folds, held_out_fold=1
        )
        np.testing.assert_allclose(embedding_centroids["House"], [1.0, 0.0])
        np.testing.assert_allclose(means, [1.0, 1.0])
        np.testing.assert_allclose(arrangement_centroids["House"], [0.0, 0.0])


if __name__ == "__main__":
    unittest.main()
