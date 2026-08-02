#!/usr/bin/env python3
"""Unit tests for the frozen Plan 065 kick feature extractor."""

from __future__ import annotations

import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

import numpy as np

import extract_kick_rhythm_features as subject


def valid_features(pattern: str = "broken_beat") -> dict[str, object]:
    return {
        "kick_pattern": pattern,
        "kick_pattern_confidence": 0.75,
        "kick_kicks_per_bar": 2.5,
        "kick_rate_basis": "main_groove",
        "kick_histogram": [1.0] * 64,
    }


class KickFeatureExtractionTests(unittest.TestCase):
    def test_missing_result_is_all_zero(self) -> None:
        vector = subject.kick_vector({"kick_pattern": None})
        self.assertEqual(vector.shape, (74,))
        self.assertEqual(float(np.sum(vector)), 0.0)

    def test_vector_order_and_histogram_normalization(self) -> None:
        vector = subject.kick_vector(valid_features())
        self.assertEqual(vector[0], 1.0)
        self.assertEqual(vector[1 + subject.PATTERNS.index("broken_beat")], 1.0)
        self.assertEqual(vector[6], 0.75)
        self.assertEqual(vector[7], 2.5)
        self.assertEqual(vector[8 + subject.RATE_BASES.index("main_groove")], 1.0)
        self.assertAlmostEqual(float(np.sum(vector[10:])), 1.0)

    def test_malformed_present_result_fails(self) -> None:
        malformed = valid_features()
        malformed["kick_histogram"] = [1.0] * 63
        with self.assertRaisesRegex(ValueError, "64"):
            subject.kick_vector(malformed)
        with self.assertRaisesRegex(ValueError, "unknown kick_pattern"):
            subject.kick_vector(valid_features("shuffle"))

    def test_extraction_preserves_manifest_order_and_requires_version(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "cache.sqlite3"
            connection = sqlite3.connect(database)
            connection.execute(
                "CREATE TABLE audio_analysis_cache ("
                "file_path TEXT, analyzer TEXT, analysis_version TEXT, "
                "input_fingerprint TEXT, features_json TEXT)"
            )
            for path, pattern in [("second", "irregular"), ("first", "four_on_floor")]:
                connection.execute(
                    "INSERT INTO audio_analysis_cache VALUES (?, ?, ?, ?, ?)",
                    (
                        path,
                        subject.ANALYZER,
                        subject.ANALYSIS_VERSION,
                        f"fingerprint-{path}",
                        json.dumps(valid_features(pattern)),
                    ),
                )
            connection.commit()
            connection.close()

            matrix, summary = subject.extract_rows(
                database, [{"file_path": "first"}, {"file_path": "second"}]
            )
            self.assertEqual(matrix[0, 1 + subject.PATTERNS.index("four_on_floor")], 1.0)
            self.assertEqual(matrix[1, 1 + subject.PATTERNS.index("irregular")], 1.0)
            self.assertEqual(summary["available_rows"], 2)
            self.assertEqual(summary["rows"], 2)

            connection = sqlite3.connect(database)
            connection.execute(
                "UPDATE audio_analysis_cache SET analysis_version = '20' WHERE file_path = 'first'"
            )
            connection.commit()
            connection.close()
            with self.assertRaisesRegex(ValueError, "version differs"):
                subject.extract_rows(database, [{"file_path": "first"}])


if __name__ == "__main__":
    unittest.main()
