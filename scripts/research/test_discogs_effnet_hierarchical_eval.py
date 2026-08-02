#!/usr/bin/env python3
"""Synthetic checks for the Plan 057 hierarchical router."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("discogs_effnet_hierarchical_eval.py")
SPEC = importlib.util.spec_from_file_location(
    "discogs_effnet_hierarchical_eval", MODULE_PATH
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(MODULE_PATH.parent))
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class HierarchicalRouterTests(unittest.TestCase):
    def test_baseline_resolves_only_within_effnet_family(self) -> None:
        predictions = MODULE.hierarchical_predictions(
            ["Techno", "Techno", "House", "Breakbeat"],
            ["Deep Techno", "House", "Tech House", None],
        )
        self.assertEqual(
            predictions,
            ["Deep Techno", "Techno", "Tech House", "Breakbeat"],
        )

    def test_length_mismatch_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "differ in length"):
            MODULE.hierarchical_predictions(["Techno"], [])


if __name__ == "__main__":
    unittest.main()
