import tempfile
import unittest
from pathlib import Path

import prepare_genre_intelligence_open_set_holdout as subject


class PrepareGenreIntelligenceOpenSetHoldoutTests(unittest.TestCase):
    def rows(self, directory: Path) -> tuple[list[dict], list[dict]]:
        selected = []
        live = []
        for index in range(1, subject.EXPECTED_ROWS + 1):
            path = directory / f"track-{index}.wav"
            path.touch()
            row = {
                "position": index,
                "track_id": str(index),
                "file_path": str(path),
                "artist_group": f"artist-{index}",
                "release_group": f"release-{index}",
            }
            selected.append(row)
            live.append(dict(row))
        return selected, live

    def test_preparation_audits_all_identity_boundaries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            selected, live = self.rows(Path(directory))
            audit = [
                {"file_path": row["file_path"], "track_id": row["track_id"]}
                for row in selected
            ]
            development = [
                {
                    "row_id": "development-1",
                    "artist_group": "development-artist",
                    "release_group": "development-release",
                }
            ]
            development_features = [
                {"row_id": "development-1", "file_path": "/development.wav"}
            ]
            rows, leakage = subject.prepare_rows(
                selected,
                audit,
                development,
                development_features,
                [{"file_path": "/reviewed.wav"}],
                [
                    {
                        "file_path": "/consumed.wav",
                        "artist_group": "consumed-artist",
                        "release_group": "consumed-release",
                    }
                ],
                live,
                {"genre_verified": {"unrelated"}},
                {"genre_verified"},
            )
            self.assertEqual(len(rows), subject.EXPECTED_ROWS)
            self.assertTrue(all(value == 0 for value in leakage.values()))
            self.assertEqual(rows[0]["row_id"], "GIO-001")
            self.assertEqual(set(rows[0]), {"row_id", "file_path"})

    def test_preparation_rejects_research_playlist_overlap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            selected, live = self.rows(Path(directory))
            audit = [
                {"file_path": row["file_path"], "track_id": row["track_id"]}
                for row in selected
            ]
            with self.assertRaisesRegex(ValueError, "leakage"):
                subject.prepare_rows(
                    selected,
                    audit,
                    [],
                    [],
                    [],
                    [],
                    live,
                    {"genre_verified": {selected[0]["track_id"]}},
                    {"genre_verified"},
                )


if __name__ == "__main__":
    unittest.main()
