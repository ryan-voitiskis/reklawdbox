import copy
import unittest

import select_genre_truth_batch as selector


def row(stratum: str, index: int, *, artist: str | None = None) -> dict:
    artist_group = artist or f"artist-{stratum}-{index}"
    return {
        "track_id": f"id-{stratum}-{index}",
        "file_path": f"/music/{stratum}/{index}.flac",
        "artist": f"Artist {stratum} {index}",
        "title": f"Title {index}",
        "album": f"Album {index}",
        "artist_group": artist_group,
        "release_group": f"release-{stratum}-{index}",
        "broad_sampling_stratum": stratum,
        "current_genre_sampling_only": f"hidden-{stratum}",
        "code": f"source-{stratum}-{index}",
    }


class SelectGenreTruthBatchTests(unittest.TestCase):
    def fixture(self) -> list[dict]:
        return [
            *[row("Breakbeat", index) for index in range(5)],
            *[row("Trance", index) for index in range(4)],
            row("Pop", 0),
            row("House", 0),
        ]

    def test_selection_is_deterministic_and_meets_frozen_quotas(self) -> None:
        rows = self.fixture()
        selected = selector.select_batch(rows)
        replay = selector.select_batch(list(reversed(copy.deepcopy(rows))))
        self.assertEqual(selected, replay)
        self.assertEqual(len(selected), 6)
        self.assertEqual(
            {genre: sum(r["broad_sampling_stratum"] == genre for r in selected)
             for genre in selector.QUOTAS},
            selector.QUOTAS,
        )
        self.assertEqual(len({r["artist_group"] for r in selected}), 6)
        self.assertEqual(len({r["release_group"] for r in selected}), 6)

    def test_private_projection_drops_current_genre_sampling_field(self) -> None:
        projected = selector.private_row(row("Pop", 0), 1)
        self.assertNotIn("current_genre_sampling_only", projected)
        self.assertEqual(projected["sampling_stratum_private"], "Pop")

    def test_selection_fails_instead_of_relaxing_missing_quota(self) -> None:
        rows = [r for r in self.fixture() if r["broad_sampling_stratum"] != "Pop"]
        with self.assertRaisesRegex(ValueError, "Pop.*required 1"):
            selector.select_batch(rows)

    def test_selection_skips_duplicate_artist_groups(self) -> None:
        rows = self.fixture()
        rows.append(row("Breakbeat", 99, artist=rows[0]["artist_group"]))
        selected = selector.select_batch(rows)
        self.assertEqual(len({r["artist_group"] for r in selected}), 6)


if __name__ == "__main__":
    unittest.main()
