import unittest

import numpy as np

import infer_genre_intelligence_precision_buffer_holdout as subject


class InferGenreIntelligencePrecisionBufferHoldoutTests(unittest.TestCase):
    def test_full_fit_binary_model_is_deterministic_and_scores(self) -> None:
        generator = np.random.default_rng(42)
        targets = len(subject.plan071.preparation.OUTPUT_PARENTS)
        truths = np.asarray([*range(targets), *range(targets), -1, -1])
        base = generator.normal(
            size=(len(truths), len(subject.plan071.base_features.FEATURE_NAMES))
        )
        clap = generator.normal(
            size=(len(truths), subject.plan071.CLAP_DIMENSION)
        )
        first = subject.fit_full_model(base, clap, truths)
        second = subject.fit_full_model(base, clap, truths)
        for key in first:
            np.testing.assert_array_equal(first[key], second[key])
        test_base = generator.normal(size=(3, base.shape[1]))
        test_clap = generator.normal(size=(3, clap.shape[1]))
        scores = subject.score_model(first, test_base, test_clap)
        self.assertEqual(scores.shape, (3, targets))
        self.assertTrue(np.all(np.isfinite(scores)))

    def test_truth_targets_preserve_unsupported_parents_as_negative(self) -> None:
        rows = []
        for index in range(subject.plan071.preparation.EXPECTED_ACCEPTED_ROWS):
            parent = "House" if index == 0 else "Tech House"
            rows.append({"canonical_parent_genre": parent})
        targets = subject.truth_targets({"rows": rows})
        self.assertEqual(targets[0], 0)
        self.assertTrue(np.all(targets[1:] == -1))


if __name__ == "__main__":
    unittest.main()
