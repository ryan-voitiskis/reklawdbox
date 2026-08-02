from __future__ import annotations

import unittest

import numpy as np

import discogs_effnet_broad_eval as broad
import evaluate_scoped_broad_genre_mvp as subject


class ScopedBroadGenreMvpTests(unittest.TestCase):
    def test_scope_requires_threshold_and_allowed_prediction(self) -> None:
        predictions = np.asarray(
            [
                broad.BROAD_INDEX["Ambient"],
                broad.BROAD_INDEX["House"],
                broad.BROAD_INDEX["Electro"],
                broad.BROAD_INDEX["Techno"],
            ]
        )
        threshold = np.asarray([True, False, True, True])
        np.testing.assert_array_equal(
            subject.scoped_offer_mask(predictions, threshold),
            [True, False, False, True],
        )

    def test_gate_requires_each_allowed_target_and_both_aggregate_checks(self) -> None:
        unselective = {"offered_precision": 0.75}
        candidate = {
            "offered_precision": 0.91,
            "coverage": 0.40,
            "folds": [{"offers": 10, "offered_precision": 0.85}],
            "per_target": {
                target: {"offers": 5, "offered_precision": 0.85}
                for target in subject.ALLOWLIST
            },
        }
        self.assertTrue(subject.scoped_gate(unselective, candidate)["passed"])
        candidate["per_target"]["Techno"]["offers"] = 4
        gate = subject.scoped_gate(unselective, candidate)
        self.assertFalse(gate["passed"])
        self.assertEqual(gate["insufficient_offer_targets"], ["Techno"])

    def test_openl3_has_frozen_priority(self) -> None:
        both = {"openl3": {"passed": True}, "clap": {"passed": True}}
        self.assertEqual(subject.select_candidate(both), "openl3")
        both["openl3"]["passed"] = False
        self.assertEqual(subject.select_candidate(both), "clap")
        both["clap"]["passed"] = False
        self.assertIsNone(subject.select_candidate(both))


if __name__ == "__main__":
    unittest.main()
