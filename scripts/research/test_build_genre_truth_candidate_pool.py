import tempfile
import unittest
from pathlib import Path

import build_genre_truth_candidate_pool as pool


def live(index: int, artist: str | None = None) -> dict:
    return {
        "track_id": str(index),
        "file_path": f"/music/{index}.flac",
        "artist": artist or f"Artist {index}",
        "title": f"Title {index}",
        "album": f"Album {index}",
    }


def audit(row: dict, recommendation: str = "IDM") -> dict:
    return {
        **row,
        "row_index": int(row["track_id"]),
        "baseline_recommendation": recommendation,
        "baseline_confidence": "low",
    }


class BuildGenreTruthCandidatePoolTests(unittest.TestCase):
    def test_excludes_paths_artists_and_releases(self) -> None:
        live_rows = [
            live(1),
            live(2, "Artist 1"),
            live(3),
            live(4),
        ]
        excluded_paths, excluded_artists, excluded_releases = pool.collect_exclusions(
            [{**live_rows[0], "artist_group": "artist 1", "release_group": "artist 1\0album 1"}],
            [],
            [],
            [],
            [live_rows[3]["file_path"]],
            {row["file_path"]: row for row in live_rows},
        )
        with tempfile.TemporaryDirectory() as directory:
            for row in live_rows:
                row["file_path"] = str(Path(directory) / f"{row['track_id']}.flac")
                Path(row["file_path"]).touch()
            # Rebuild the path exclusion after moving the synthetic files.
            excluded_paths = {live_rows[0]["file_path"], live_rows[3]["file_path"]}
            result = pool.build_pool(
                {"rows": [audit(row) for row in live_rows]},
                live_rows,
                excluded_paths,
                excluded_artists,
                excluded_releases,
            )
        self.assertEqual(result["candidate_rows"], 1)
        self.assertEqual(result["rows"][0]["track_id"], "3")

    def test_parent_sampling_stratum_is_private_model_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            row = live(1)
            row["file_path"] = str(Path(directory) / "one.flac")
            Path(row["file_path"]).touch()
            result = pool.build_pool(
                {"rows": [audit(row, "Deep House")]}, [row], set(), set(), set()
            )
        self.assertEqual(result["candidate_counts"], {"House": 1})
        self.assertEqual(result["rows"][0]["sampling_stratum_private"], "House")
        self.assertEqual(
            result["selection_source"]["recommendation_is_not_truth"], True
        )

    def test_metadata_drift_uses_live_identity_but_keeps_audio_recommendation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            row = live(1)
            row["file_path"] = str(Path(directory) / "one.flac")
            Path(row["file_path"]).touch()
            frozen = audit(dict(row), "Tech House")
            frozen["artist"] = "Old Artist"
            result = pool.build_pool(
                {"rows": [frozen]}, [row], set(), set(), set()
            )
        self.assertEqual(result["identity_metadata_drift_rows"], 1)
        self.assertEqual(result["rows"][0]["artist"], "Artist 1")
        self.assertEqual(
            result["rows"][0]["sampling_stratum_private"], "Tech House"
        )


if __name__ == "__main__":
    unittest.main()
