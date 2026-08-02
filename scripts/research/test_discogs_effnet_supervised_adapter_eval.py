#!/usr/bin/env python3
"""Synthetic checks for the Plan 058 supervised adapters."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


MODULE_PATH = Path(__file__).with_name("discogs_effnet_supervised_adapter_eval.py")
SPEC = importlib.util.spec_from_file_location(
    "discogs_effnet_supervised_adapter_eval", MODULE_PATH
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(MODULE_PATH.parent))
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class SupervisedAdapterTests(unittest.TestCase):
    def test_balanced_weights_equalize_class_mass(self) -> None:
        weights = MODULE.balanced_weights(["House", "House", "House", "Techno"])
        self.assertAlmostEqual(float(weights[:3].sum()), float(weights[3:].sum()))
        self.assertAlmostEqual(float(weights.mean()), 1.0)

    def test_baseline_one_hot_leaves_abstention_zero(self) -> None:
        values = MODULE.baseline_one_hot(["House", None, "Techno"])
        self.assertEqual(float(values[0].sum()), 1.0)
        self.assertEqual(float(values[1].sum()), 0.0)
        self.assertEqual(float(values[2].sum()), 1.0)

    def test_arrangement_imputation_uses_training_rows_only(self) -> None:
        values = np.asarray([[1.0, np.nan], [3.0, 5.0], [100.0, 100.0]])
        filled = MODULE.impute_arrangement(values, np.asarray([True, True, False]))
        np.testing.assert_allclose(filled[0], [1.0, 5.0])
        np.testing.assert_allclose(filled[2], [100.0, 100.0])

    def test_pca_is_fit_without_held_out_row(self) -> None:
        values = np.asarray([[1.0, 0.0], [-1.0, 0.0], [0.0, 100.0]])
        projected = MODULE.pca_projection(
            values, np.asarray([True, True, False]), components=1
        )
        self.assertAlmostEqual(abs(float(projected[0, 0])), 1.0)
        self.assertAlmostEqual(float(projected[2, 0]), 0.0)


if __name__ == "__main__":
    unittest.main()
