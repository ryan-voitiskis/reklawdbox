import unittest

import numpy as np

import infer_genre_intelligence_precision_first_holdout as subject


class InferGenreIntelligencePrecisionFirstHoldoutTests(unittest.TestCase):
    def test_baseline_predictions_preserve_supported_parent_or_abstain(self) -> None:
        width = len(subject.plan071.base_features.FEATURE_NAMES)
        matrix = np.zeros((3, width), dtype=np.float64)
        offset = width - len(subject.plan071.base_features.BASELINE_FEATURES)
        matrix[0, offset] = 1.0
        matrix[1, offset + 2] = 1.0
        matrix[2, -1] = 1.0
        self.assertEqual(
            subject.baseline_predictions(matrix), ["House", "Techno", None]
        )

    def test_frozen_model_hash_is_bound_as_an_input(self) -> None:
        self.assertEqual(
            subject.EXPECTED_INPUT_SHA256["frozen_model"],
            subject.EXPECTED_FROZEN_MODEL_SHA256,
        )


if __name__ == "__main__":
    unittest.main()
