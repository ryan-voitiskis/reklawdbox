#!/usr/bin/env python3
"""Synthetic checks for the frozen broad Discogs-EffNet evaluation."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

import numpy as np


MODULE_PATH = Path(__file__).with_name("discogs_effnet_broad_eval.py")
SPEC = importlib.util.spec_from_file_location("discogs_effnet_broad_eval", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class DiscogsEffnetBroadEvalTests(unittest.TestCase):
    def test_mapping_is_complete_and_semantically_frozen(self) -> None:
        self.assertEqual(MODULE.FINE_TO_BROAD["Experimental"], None)
        self.assertTrue(
            all(
                MODULE.FINE_TO_BROAD[genre] is not None
                for genre in MODULE.CANONICAL
                if genre != "Experimental"
            )
        )
        self.assertEqual(
            MODULE.broad_semantic_sha256(), MODULE.EXPECTED_BROAD_SEMANTIC_SHA256
        )

    def test_broad_scores_use_max_and_ties_use_frozen_order(self) -> None:
        scores = np.zeros((1, len(MODULE.CANONICAL)), dtype=np.float64)
        scores[0, MODULE.CANONICAL.index("Deep House")] = 0.7
        scores[0, MODULE.CANONICAL.index("House")] = 0.6
        scores[0, MODULE.CANONICAL.index("Techno")] = 0.7
        broad = MODULE.broad_scores(scores)
        self.assertEqual(broad[0, MODULE.BROAD_INDEX["House"]], 0.7)
        prediction, margin = MODULE.top_predictions_and_margins(broad)
        self.assertEqual(prediction[0], MODULE.BROAD_INDEX["House"])
        self.assertEqual(margin[0], 0.0)

    def test_threshold_maximizes_eligible_coverage(self) -> None:
        margins = np.asarray([0.1, 0.2, 0.3, 0.4, 0.5])
        correct = np.asarray([False, True, True, True, True])
        selected = MODULE.choose_threshold(margins, correct, minimum_offers=3)
        self.assertIsNotNone(selected)
        assert selected is not None
        self.assertEqual(selected["threshold"], 0.2)
        self.assertEqual(selected["offers"], 4)
        self.assertEqual(selected["offered_precision"], 1.0)

    def test_threshold_abstains_when_minimum_offer_gate_cannot_pass(self) -> None:
        selected = MODULE.choose_threshold(
            np.asarray([0.1, 0.2, 0.3]),
            np.asarray([True, True, True]),
            minimum_offers=4,
        )
        self.assertIsNone(selected)

    def test_cross_fitted_thresholds_do_not_use_held_out_rows(self) -> None:
        predictions = np.asarray([0, 0, 1, 1, 0, 1])
        truths = np.asarray([0, 0, 1, 1, 1, 0])
        margins = np.asarray([0.9, 0.8, 0.7, 0.6, 0.95, 0.94])
        folds = np.asarray([0, 0, 1, 1, 2, 2])
        offered, details = MODULE.cross_fitted_offers(predictions, margins, truths, folds)
        self.assertFalse(np.any(offered))
        self.assertEqual([row["fold"] for row in details], [0, 1, 2])
        self.assertTrue(all(row["threshold"] is None for row in details))

    def test_metrics_separate_precision_from_coverage(self) -> None:
        truths = np.asarray([0, 0, 1, 1])
        predictions = np.asarray([0, 1, 1, 0])
        offered = np.asarray([True, False, True, True])
        folds = np.asarray([0, 0, 1, 1])
        result = MODULE.metrics(truths, predictions, offered, folds)
        self.assertEqual(result["offers"], 3)
        self.assertEqual(result["correct_offers"], 2)
        self.assertAlmostEqual(result["coverage"], 0.75)
        self.assertAlmostEqual(result["offered_precision"], 2 / 3)
        self.assertAlmostEqual(result["accuracy"], 0.5)


if __name__ == "__main__":
    unittest.main()
