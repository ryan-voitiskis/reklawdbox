import copy
import unittest

import select_genre_truth_batch_b02 as selector


def row(stratum: str, index: int, *, artist: str | None = None) -> dict:
    return {
        "track_id": f"id-{stratum}-{index}",
        "file_path": f"/music/{stratum}/{index}.flac",
        "artist": f"Artist {stratum} {index}",
        "title": f"Title {index}",
        "album": f"Album {index}",
        "artist_group": artist or f"artist-{stratum}-{index}",
        "release_group": f"release-{stratum}-{index}",
        "sampling_stratum_private": stratum,
        "source_code": "source",
    }


class SelectGenreTruthBatchB02Tests(unittest.TestCase):
    def fixture(self) -> list[dict]:
        return [
            *[row("Minimal", index) for index in range(8)],
            row("IDM", 0),
            row("House", 0),
        ]

    def test_selection_is_deterministic_and_meets_frozen_quotas(self) -> None:
        rows = self.fixture()
        selected = selector.select_batch(rows)
        replay = selector.select_batch(list(reversed(copy.deepcopy(rows))))
        self.assertEqual(selected, replay)
        self.assertEqual(len(selected), 6)
        self.assertEqual(
            {
                genre: sum(
                    row["sampling_stratum_private"] == genre for row in selected
                )
                for genre in selector.QUOTAS
            },
            selector.QUOTAS,
        )
        self.assertEqual(
            [row["code"] for row in selected],
            [f"GI02-{i:02d}" for i in range(1, 7)],
        )

    def test_selection_fails_instead_of_relaxing_artist_diversity(self) -> None:
        rows = self.fixture()
        for row_value in rows:
            if row_value["sampling_stratum_private"] == "Minimal":
                row_value["artist_group"] = "same-minimal-artist"
        with self.assertRaisesRegex(ValueError, "Minimal.*required 5"):
            selector.select_batch(rows)

    def test_eligibility_requires_prior_exclusion_and_rejects_reviewed_rows(self) -> None:
        source = [
            {
                "file_path": f"/music/{index}.flac",
                "artist": f"Artist {index}",
                "title": f"Title {index}",
                "album": f"Album {index}",
                "sampling_stratum_private": "Minimal",
            }
            for index in range(4)
        ]
        tracks = [
            {
                "track_id": str(index),
                "file_path": f"/music/{index}.flac",
                "artist": f"Artist {index}",
                "title": f"Title {index}",
                "album": f"Album {index}",
                "current_genre": "Minimal",
            }
            for index in range(4)
        ]
        memberships = {
            "0": {selector.REQUIRED_PRIOR_PLAYLIST},
            "1": set(),
            "2": {selector.REQUIRED_PRIOR_PLAYLIST, "genre_verified"},
            "3": {selector.REQUIRED_PRIOR_PLAYLIST},
        }
        eligible = selector.eligible_rows(
            source,
            tracks,
            memberships,
            {"/music/3.flac"},
        )
        self.assertEqual([row["track_id"] for row in eligible], ["0"])

    def test_eligibility_fails_on_live_identity_drift(self) -> None:
        source = [
            {
                "file_path": "/music/one.flac",
                "artist": "Artist",
                "title": "Original Title",
                "album": "Album",
                "sampling_stratum_private": "IDM",
            }
        ]
        tracks = [
            {
                "track_id": "1",
                "file_path": "/music/one.flac",
                "artist": "Artist",
                "title": "Changed Title",
                "album": "Album",
                "current_genre": "IDM",
            }
        ]
        with self.assertRaisesRegex(ValueError, "identity or sampling drift"):
            selector.eligible_rows(
                source,
                tracks,
                {"1": {selector.REQUIRED_PRIOR_PLAYLIST}},
                set(),
            )


if __name__ == "__main__":
    unittest.main()
