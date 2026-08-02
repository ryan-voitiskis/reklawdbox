from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

import numpy as np


SCRIPT = (
    Path(__file__).parent
    / "research"
    / "extract_broad_genre_representation.py"
)
SPEC = importlib.util.spec_from_file_location(
    "extract_broad_genre_representation", SCRIPT
)
assert SPEC is not None and SPEC.loader is not None
representation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(representation)


class BroadGenreRepresentationTests(unittest.TestCase):
    def test_even_excerpt_starts_cover_full_available_span(self) -> None:
        self.assertEqual(
            representation.even_excerpt_starts(100, 20, 3),
            [0, 40, 80],
        )
        self.assertEqual(
            representation.even_excerpt_starts(10, 20, 3),
            [0, 0, 0],
        )

    def test_zero_and_repeat_padding_are_distinct_and_exact(self) -> None:
        values = np.asarray([1.0, 2.0, 3.0], dtype=np.float32)
        zero = representation.pad_excerpt(values, 8, "zero")
        repeat = representation.pad_excerpt(values, 8, "repeat")
        np.testing.assert_array_equal(
            zero, np.asarray([1, 2, 3, 0, 0, 0, 0, 0], dtype=np.float32)
        )
        np.testing.assert_array_equal(
            repeat, np.asarray([1, 2, 3, 1, 2, 3, 1, 2], dtype=np.float32)
        )

    def test_evenly_spaced_excerpts_preserve_order(self) -> None:
        values = np.arange(10, dtype=np.float32)
        excerpts = representation.evenly_spaced_excerpts(
            values, excerpt_samples=4, excerpt_count=3, padding="zero"
        )
        np.testing.assert_array_equal(
            excerpts,
            np.asarray(
                [[0, 1, 2, 3], [3, 4, 5, 6], [6, 7, 8, 9]],
                dtype=np.float32,
            ),
        )

    def test_patch_aggregation_normalizes_patches_then_track(self) -> None:
        patches = np.zeros((2, representation.OUTPUT_DIMENSION), dtype=np.float32)
        patches[0, 0] = 2.0
        patches[1, 1] = 4.0
        track = representation.aggregate_patch_embeddings(patches)
        self.assertAlmostEqual(float(np.linalg.norm(track)), 1.0, places=6)
        self.assertAlmostEqual(float(track[0]), 2**-0.5, places=6)
        self.assertAlmostEqual(float(track[1]), 2**-0.5, places=6)

    def test_aggregation_rejects_wrong_dimension(self) -> None:
        with self.assertRaisesRegex(ValueError, "patch embeddings"):
            representation.aggregate_patch_embeddings(np.zeros((3, 4)))


if __name__ == "__main__":
    unittest.main()
