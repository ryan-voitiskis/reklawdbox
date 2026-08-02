from __future__ import annotations

import unittest

import prepare_scoped_broad_holdout_inputs as subject


class PrepareScopedBroadHoldoutInputsTests(unittest.TestCase):
    def test_join_preserves_holdout_order_and_validates_identity(self) -> None:
        audit = [
            {"file_path": "/a", "track_id": "1"},
            {"file_path": "/b", "track_id": "2"},
        ]
        selected = [
            {"position": 1, "file_path": "/b", "track_id": "2"},
            {"position": 2, "file_path": "/a", "track_id": "1"},
        ]
        self.assertEqual(subject.audit_indices(selected, audit).tolist(), [1, 0])
        selected[0]["track_id"] = "wrong"
        with self.assertRaisesRegex(ValueError, "track identity differs"):
            subject.audit_indices(selected, audit)

    def test_representation_manifest_contains_no_sampling_or_truth_fields(self) -> None:
        selected = [
            {
                "code": "SBH001",
                "file_path": "/track.wav",
                "current_genre_sampling_only": "House",
                "broad_sampling_stratum": "House",
            }
        ]
        manifest = subject.representation_manifest(selected, "abc")
        self.assertEqual(
            manifest["rows"],
            [{"row_index": 0, "code": "SBH001", "file_path": "/track.wav"}],
        )
        self.assertNotIn("truth", manifest["rows"][0])
        self.assertNotIn("current_genre", manifest["rows"][0])


if __name__ == "__main__":
    unittest.main()
