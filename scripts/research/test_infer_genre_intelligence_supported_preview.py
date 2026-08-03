import unittest

import numpy as np

import infer_genre_intelligence_supported_preview as subject


class InferGenreIntelligenceSupportedPreviewTests(unittest.TestCase):
    def test_full_fit_model_scores_finite_seven_way_outputs(self) -> None:
        generator = np.random.default_rng(42)
        truths = np.repeat(
            np.arange(len(subject.candidate.RELEASE_SCOPE), dtype=np.int64), 3
        )
        base = generator.normal(
            size=(len(truths), len(subject.base_features.FEATURE_NAMES))
        )
        clap = generator.normal(size=(len(truths), subject.clap_evaluation.CLAP_DIMENSION))
        model = subject.fit_full_model(base, clap, truths)
        test_base = generator.normal(size=(4, base.shape[1]))
        test_clap = generator.normal(size=(4, clap.shape[1]))
        scores, predictions, margins = subject.score_model(
            model, test_base, test_clap
        )
        self.assertEqual(scores.shape, (4, len(subject.candidate.RELEASE_SCOPE)))
        self.assertEqual(predictions.shape, (4,))
        self.assertTrue(np.all(np.isfinite(scores)))
        self.assertTrue(np.all(margins >= 0.0))

    def test_full_fit_model_is_deterministic(self) -> None:
        generator = np.random.default_rng(7)
        truths = np.repeat(
            np.arange(len(subject.candidate.RELEASE_SCOPE), dtype=np.int64), 2
        )
        base = generator.normal(
            size=(len(truths), len(subject.base_features.FEATURE_NAMES))
        )
        clap = generator.normal(size=(len(truths), subject.clap_evaluation.CLAP_DIMENSION))
        first = subject.fit_full_model(base, clap, truths)
        second = subject.fit_full_model(base, clap, truths)
        for key in first:
            np.testing.assert_array_equal(first[key], second[key])


if __name__ == "__main__":
    unittest.main()
