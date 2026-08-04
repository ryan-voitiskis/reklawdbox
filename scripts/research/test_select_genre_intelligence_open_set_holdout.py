import unittest

import select_genre_intelligence_open_set_holdout as subject


def rows(stratum: str, count: int, *, shared_artist: str | None = None):
    return [
        {
            "track_id": f"{stratum}-{index}",
            "file_path": f"/music/{stratum}-{index}.flac",
            "artist_group": shared_artist or f"artist-{stratum}-{index}",
            "release_group": f"release-{stratum}-{index}",
            "broad_sampling_stratum": stratum,
        }
        for index in range(count)
    ]


class SelectGenreIntelligenceOpenSetHoldoutTests(unittest.TestCase):
    def setUp(self) -> None:
        self.original_rows = subject.HOLDOUT_ROWS
        self.original_cap = subject.MAX_PER_STRATUM
        self.original_quotas = subject.DESIRED_QUOTAS

    def tearDown(self) -> None:
        subject.HOLDOUT_ROWS = self.original_rows
        subject.MAX_PER_STRATUM = self.original_cap
        subject.DESIRED_QUOTAS = self.original_quotas

    def test_scarce_quota_is_protected_before_abundant_fill(self) -> None:
        subject.HOLDOUT_ROWS = 5
        subject.MAX_PER_STRATUM = 4
        subject.DESIRED_QUOTAS = {"Scarce": 2, "Large": 3}
        candidates = rows("Scarce", 2) + rows("Large", 8)

        selected = subject.select_holdout(candidates)

        counts = {
            stratum: sum(
                row["broad_sampling_stratum"] == stratum for row in selected
            )
            for stratum in subject.DESIRED_QUOTAS
        }
        self.assertEqual(counts, {"Scarce": 2, "Large": 3})

    def test_fill_respects_artist_release_and_stratum_cap(self) -> None:
        subject.HOLDOUT_ROWS = 4
        subject.MAX_PER_STRATUM = 2
        subject.DESIRED_QUOTAS = {"A": 1, "B": 1, "C": 1}
        candidates = rows("A", 4) + rows("B", 4) + rows("C", 4)
        candidates.append(
            {
                **rows("B", 1)[0],
                "track_id": "duplicate-artist",
                "file_path": "/music/duplicate-artist.flac",
                "artist_group": candidates[0]["artist_group"],
                "release_group": "unique-release",
            }
        )

        selected = subject.select_holdout(candidates)

        self.assertEqual(len(selected), 4)
        self.assertEqual(len({row["artist_group"] for row in selected}), 4)
        self.assertEqual(len({row["release_group"] for row in selected}), 4)
        counts = {}
        for row in selected:
            stratum = row["broad_sampling_stratum"]
            counts[stratum] = counts.get(stratum, 0) + 1
        self.assertLessEqual(max(counts.values()), 2)

    def test_selection_is_deterministic(self) -> None:
        subject.HOLDOUT_ROWS = 4
        subject.MAX_PER_STRATUM = 3
        subject.DESIRED_QUOTAS = {"A": 2, "B": 2}
        candidates = rows("A", 5) + rows("B", 5)

        first = subject.select_holdout(candidates)
        second = subject.select_holdout(list(reversed(candidates)))

        self.assertEqual(
            [row["track_id"] for row in first],
            [row["track_id"] for row in second],
        )

    def test_fails_closed_when_unique_groups_cannot_fill(self) -> None:
        subject.HOLDOUT_ROWS = 3
        subject.MAX_PER_STRATUM = 3
        subject.DESIRED_QUOTAS = {"A": 3}
        candidates = rows("A", 4, shared_artist="same-artist")

        with self.assertRaisesRegex(ValueError, "produced 1 rows; required 3"):
            subject.select_holdout(candidates)


if __name__ == "__main__":
    unittest.main()
