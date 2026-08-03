import unittest

import numpy as np

import evaluate_genre_intelligence_openl3 as subject


class EvaluateGenreIntelligenceOpenL3Tests(unittest.TestCase):
    def test_projection_is_fit_on_training_partition_only(self) -> None:
        rows = 8
        base = np.arange(rows * 2, dtype=np.float64).reshape(rows, 2)
        representation = np.arange(rows * 4, dtype=np.float64).reshape(rows, 4)
        train = np.asarray([True] * 6 + [False] * 2)
        first = subject.augmented_matrix(base, representation, train)
        changed = representation.copy()
        changed[~train] += 10000
        second = subject.augmented_matrix(base, changed, train)
        np.testing.assert_allclose(first[train], second[train])

    def test_projection_appends_at_most_64_components(self) -> None:
        base = np.ones((70, 3), dtype=np.float64)
        representation = np.arange(70 * 80, dtype=np.float64).reshape(70, 80)
        train = np.asarray([True] * 65 + [False] * 5)
        result = subject.augmented_matrix(base, representation, train)
        self.assertEqual(result.shape, (70, 67))


if __name__ == "__main__":
    unittest.main()
