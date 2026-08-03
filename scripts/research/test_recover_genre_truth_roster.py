from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import recover_genre_truth_roster as recovery


class RecoverGenreTruthRosterTests(unittest.TestCase):
    def test_rekordbox_xml_paths_decodes_file_locations(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "review.xml"
            path.write_text(
                """<?xml version="1.0"?>
<DJ_PLAYLISTS><COLLECTION Entries="1">
<TRACK Location="file://localhost/Music/Test%20Track.flac" />
</COLLECTION></DJ_PLAYLISTS>
""",
                encoding="utf-8",
            )
            self.assertEqual(
                recovery.rekordbox_xml_paths(path), ["/Music/Test Track.flac"]
            )

    def test_audit_recovery_adds_reviews_and_removes_historical_truth(self) -> None:
        current = {
            "rows": [
                {"track_id": "1", "file_path": "/keep.flac"},
                {"track_id": "2", "file_path": "/historical.flac"},
            ]
        }
        live = {
            "/added.flac": {"track_id": "3", "file_path": "/added.flac"}
        }
        result = recovery.recovered_audit_manifest(
            current,
            ["/added.flac", "/keep.flac"],
            {"/historical.flac"},
            live,
        )
        self.assertEqual(
            {row["file_path"] for row in result["rows"]},
            {"/keep.flac", "/added.flac"},
        )
        self.assertFalse(result["recovery"]["model_features_or_predictions_used"])

    def test_require_equal_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "roster differs"):
            recovery.require_equal("roster", "actual", "expected")


if __name__ == "__main__":
    unittest.main()
