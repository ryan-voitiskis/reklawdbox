from __future__ import annotations

import unittest

import numpy as np

import discogs_effnet_broad_eval as broad
import infer_scoped_broad_genre_holdout as subject


class InferScopedBroadGenreHoldoutTests(unittest.TestCase):
    def test_offers_require_threshold_and_allowlist(self) -> None:
        predictions = np.asarray(
            [
                broad.BROAD_INDEX["Ambient"],
                broad.BROAD_INDEX["House"],
                broad.BROAD_INDEX["Electro"],
                broad.BROAD_INDEX["Techno"],
            ]
        )
        margins = np.asarray(
            [
                subject.DEPLOYMENT_THRESHOLD,
                subject.DEPLOYMENT_THRESHOLD - 1e-6,
                subject.DEPLOYMENT_THRESHOLD + 1.0,
                subject.DEPLOYMENT_THRESHOLD + 1e-6,
            ]
        )
        np.testing.assert_array_equal(
            subject.offered_predictions(predictions, margins),
            [True, False, False, True],
        )

    def test_hash_validation_rejects_changed_input(self) -> None:
        with self.assertRaises(FileNotFoundError):
            subject.validate_hash(
                subject.Path("/does/not/exist"), "unused", "missing input"
            )


if __name__ == "__main__":
    unittest.main()
