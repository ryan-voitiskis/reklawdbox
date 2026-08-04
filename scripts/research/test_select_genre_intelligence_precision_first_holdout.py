import unittest

import select_genre_intelligence_precision_first_holdout as subject


def row(stratum: str, index: int) -> dict:
    return {
        "track_id": f"{stratum}-{index}",
        "file_path": f"/{stratum}-{index}.wav",
        "artist_group": f"artist-{stratum}-{index}",
        "release_group": f"release-{stratum}-{index}",
        "broad_sampling_stratum": stratum,
    }


class SelectGenreIntelligencePrecisionFirstHoldoutTests(unittest.TestCase):
    def test_selection_is_deterministic_and_identity_isolated(self) -> None:
        rows = []
        for stratum, quota in subject.DESIRED_QUOTAS.items():
            rows.extend(row(stratum, index) for index in range(quota + 10))
        first = subject.select_holdout(rows)
        second = subject.select_holdout(list(reversed(rows)))
        self.assertEqual(
            [item["track_id"] for item in first],
            [item["track_id"] for item in second],
        )
        self.assertEqual(len(first), subject.HOLDOUT_ROWS)
        self.assertEqual(
            len({item["artist_group"] for item in first}), subject.HOLDOUT_ROWS
        )
        self.assertEqual(
            len({item["release_group"] for item in first}), subject.HOLDOUT_ROWS
        )

    def test_selection_fills_a_quota_shortfall_from_remaining_rows(self) -> None:
        rows = [row("House", index) for index in range(200)]
        selected = subject.select_holdout(rows)
        self.assertEqual(len(selected), subject.HOLDOUT_ROWS)
        self.assertTrue(
            all(item["broad_sampling_stratum"] == "House" for item in selected)
        )

    def test_selection_rejects_insufficient_unique_artists(self) -> None:
        rows = [row("House", index) for index in range(subject.HOLDOUT_ROWS - 1)]
        with self.assertRaisesRegex(ValueError, "required"):
            subject.select_holdout(rows)


if __name__ == "__main__":
    unittest.main()
