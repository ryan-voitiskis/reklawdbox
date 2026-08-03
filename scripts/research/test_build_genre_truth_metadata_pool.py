import tempfile
import unittest
from pathlib import Path

import build_genre_truth_metadata_pool as pool


def live(index: int, genre: str = "House", artist: str | None = None) -> dict:
    return {
        "track_id": str(index),
        "file_path": f"/music/{index}.flac",
        "artist": artist or f"Artist {index}",
        "title": f"Title {index}",
        "album": f"Album {index}",
        "current_genre": genre,
    }


def audit(row: dict, recommendation: str = "Tech House") -> dict:
    return {
        **row,
        "row_index": int(row["track_id"]),
        "baseline_recommendation": recommendation,
        "baseline_confidence": "low",
    }


class BuildGenreTruthMetadataPoolTests(unittest.TestCase):
    def test_excludes_prior_paths_and_releases_but_not_artists(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            rows = [live(1), live(2, artist="Artist 1"), live(3)]
            for row in rows:
                row["file_path"] = str(Path(directory) / f"{row['track_id']}.flac")
                Path(row["file_path"]).touch()
            excluded_release = pool.corpus.release_group(
                rows[2]["artist"], rows[2]["album"], rows[2]["title"]
            )
            result = pool.build_pool(
                {"rows": [audit(row) for row in rows]},
                rows,
                [],
                {rows[0]["file_path"]},
                {excluded_release},
                experiment_id="pool-test",
            )
        self.assertEqual({row["track_id"] for row in result["rows"]}, {"2"})

    def test_current_genre_precedes_model_for_the_same_parent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            row = live(1, "Deep House")
            row["file_path"] = str(Path(directory) / "one.flac")
            Path(row["file_path"]).touch()
            result = pool.build_pool(
                {"rows": [audit(row, "House")]},
                [row],
                [],
                set(),
                set(),
                experiment_id="pool-test",
            )
        self.assertEqual(len(result["rows"]), 1)
        self.assertEqual(result["rows"][0]["sampling_stratum_private"], "House")
        self.assertEqual(
            result["rows"][0]["sampling_source_private"],
            "current_rekordbox_genre",
        )
        self.assertTrue(result["selection_source"]["current_genre_is_not_truth"])

    def test_disagreeing_sources_create_private_candidates_for_both_parents(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            row = live(1, "Minimal")
            row["file_path"] = str(Path(directory) / "one.flac")
            Path(row["file_path"]).touch()
            result = pool.build_pool(
                {"rows": [audit(row, "Tech House")]},
                [row],
                [],
                set(),
                set(),
                experiment_id="pool-test",
            )
        self.assertEqual(
            {row["sampling_stratum_private"] for row in result["rows"]},
            {"Minimal", "Tech House"},
        )


if __name__ == "__main__":
    unittest.main()
