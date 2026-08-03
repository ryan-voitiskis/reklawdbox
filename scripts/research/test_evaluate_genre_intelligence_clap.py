import tempfile
import unittest

from pathlib import Path

import numpy as np

import evaluate_genre_intelligence_clap as subject


class EvaluateGenreIntelligenceClapTests(unittest.TestCase):
    def test_representation_loader_requires_single_finite_512_array(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "clap.npz"
            np.savez_compressed(path, embeddings=np.ones((3, 512)))
            result = subject.load_representation(path, 3)
            self.assertEqual(result.shape, (3, 512))
            np.savez_compressed(path, embeddings=np.ones((3, 511)))
            with self.assertRaisesRegex(ValueError, "shape or values"):
                subject.load_representation(path, 3)


if __name__ == "__main__":
    unittest.main()
