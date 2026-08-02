from __future__ import annotations

import unittest
from collections import Counter

import select_scoped_broad_genre_holdout as subject


def row(index: int, target: str, artist: str | None = None) -> dict:
    artist = artist or f"artist-{index}"
    return {
        "track_id": str(index),
        "file_path": f"/track-{index}.wav",
        "broad_sampling_stratum": target,
        "artist_group": artist,
        "release_group": f"{artist}\0release-{index}",
    }


class ScopedBroadGenreHoldoutTests(unittest.TestCase):
    def test_selection_is_deterministic_diverse_and_capped(self) -> None:
        rows = [
            row(target_index * 10 + index, f"target-{target_index}")
            for target_index in range(12)
            for index in range(6)
        ]
        first = subject.select_holdout(rows, count=48)
        second = subject.select_holdout(list(reversed(rows)), count=48)
        self.assertEqual(
            [item["track_id"] for item in first],
            [item["track_id"] for item in second],
        )
        self.assertEqual(len(first), 48)
        counts = Counter(item["broad_sampling_stratum"] for item in first)
        self.assertTrue(all(count <= 8 for count in counts.values()))
        self.assertEqual(len({item["artist_group"] for item in first}), 48)
        self.assertEqual(len({item["release_group"] for item in first}), 48)

    def test_selection_skips_repeated_artist_and_release(self) -> None:
        rows = [
            row(1, "House", artist="shared"),
            row(2, "Techno", artist="shared"),
            row(3, "Ambient"),
            row(4, "Electro"),
        ]
        selected = subject.select_holdout(rows, count=3)
        self.assertEqual(len({item["artist_group"] for item in selected}), 3)

    def test_selection_fails_when_constraints_cannot_fill(self) -> None:
        rows = [row(index, "House") for index in range(10)]
        with self.assertRaisesRegex(ValueError, "produced 8 rows; required 9"):
            subject.select_holdout(rows, count=9)


if __name__ == "__main__":
    unittest.main()
