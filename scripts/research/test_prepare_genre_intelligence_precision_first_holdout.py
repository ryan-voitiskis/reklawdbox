import tempfile
import unittest
from pathlib import Path

import prepare_genre_intelligence_precision_first_holdout as subject


class PrepareGenreIntelligencePrecisionFirstHoldoutTests(unittest.TestCase):
    def rows(self, directory: Path) -> tuple[list[dict], list[dict]]:
        selected = []
        live = []
        for index in range(1, subject.EXPECTED_ROWS + 1):
            path = directory / f"track-{index}.wav"
            path.touch()
            live_row = {
                "track_id": str(index),
                "file_path": str(path),
                "artist": f"Artist {index}",
                "album": f"Album {index}",
                "title": f"Track {index}",
            }
            selected.append(
                {
                    "position": index,
                    "track_id": str(index),
                    "file_path": str(path),
                    "artist_group": subject.library.normalized(
                        live_row["artist"]
                    ),
                    "release_group": subject.library.release_group(live_row),
                }
            )
            live.append(live_row)
        return selected, live

    def test_preparation_audits_both_consumed_holdouts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            selected, live = self.rows(Path(directory))
            rows, leakage = subject.prepare_rows(
                selected,
                [],
                [],
                [],
                [],
                [],
                live,
                {},
                {"genre_verified"},
            )
            self.assertEqual(len(rows), subject.EXPECTED_ROWS)
            self.assertTrue(all(value == 0 for value in leakage.values()))
            self.assertEqual(set(rows[0]), {"row_id", "file_path"})

    def test_preparation_rejects_second_holdout_artist_overlap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            selected, live = self.rows(Path(directory))
            consumed = [
                {
                    "file_path": "/different.wav",
                    "artist_group": selected[0]["artist_group"],
                    "release_group": "different-release",
                }
            ]
            with self.assertRaisesRegex(ValueError, "leakage"):
                subject.prepare_rows(
                    selected,
                    [],
                    [],
                    [],
                    [],
                    consumed,
                    live,
                    {},
                    {"genre_verified"},
                )


if __name__ == "__main__":
    unittest.main()
