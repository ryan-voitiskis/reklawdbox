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

    def test_b03_selects_twenty_rows_after_b01_exclusions(self) -> None:
        quotas = selector.BATCH_CONFIGS["genre-intelligence-truth-v1-b03"][
            "quotas"
        ]
        rows = [
            row(stratum, index)
            for stratum, count in quotas.items()
            for index in range(count + 1)
        ]
        excluded = rows[::100]
        selected = selector.select_batch(
            rows,
            quotas=quotas,
            seed="genre-intelligence-truth-v1-b03",
            excluded_paths={item["file_path"] for item in excluded},
            excluded_artists={item["artist_group"] for item in excluded},
            excluded_releases={item["release_group"] for item in excluded},
        )
        self.assertEqual(len(selected), 20)
        self.assertEqual(
            {
                genre: sum(row["broad_sampling_stratum"] == genre for row in selected)
                for genre in quotas
            },
            quotas,
        )
        self.assertEqual(
            selector.private_row(
                selected[0], 1, "genre-intelligence-truth-v1-b03"
            )["code"],
            "GI03-01",
        )

    def test_selection_fails_instead_of_relaxing_missing_quota(self) -> None:
        rows = [r for r in self.fixture() if r["broad_sampling_stratum"] != "Pop"]
        with self.assertRaisesRegex(ValueError, "Pop.*required 1"):
            selector.select_batch(rows)

    def test_selection_skips_duplicate_artist_groups(self) -> None:
        rows = self.fixture()
        rows.append(row("Breakbeat", 99, artist=rows[0]["artist_group"]))
        selected = selector.select_batch(rows)
        self.assertEqual(len({r["artist_group"] for r in selected}), 6)

    def test_source_validation_accepts_original_and_verified_recovery(self) -> None:
        selected = self.fixture()
        roster_sha256 = selector.source_roster_fingerprint(selected)
        source = {
            "experiment_id": selector.SOURCE_EXPERIMENT_ID,
            "roster_sha256": roster_sha256,
            "selected": selected,
        }
        self.assertEqual(
            selector.validate_source(
                source,
                "original",
                expected_artifact_sha256="original",
                expected_roster_sha256=roster_sha256,
            ),
            "original_artifact",
        )
        source["recovery"] = {
            "original_artifact_sha256": "original",
            "original_roster_sha256": roster_sha256,
            "identity_roster_replayed_exactly": True,
            "byte_identical_replay": True,
            "model_features_or_predictions_used": False,
        }
        self.assertEqual(
            selector.validate_source(
                source,
                "recovered",
                expected_artifact_sha256="original",
                expected_roster_sha256=roster_sha256,
            ),
            "verified_recovery",
        )

    def test_source_validation_rejects_unverified_recovery(self) -> None:
        selected = self.fixture()
        roster_sha256 = selector.source_roster_fingerprint(selected)
        source = {
            "experiment_id": selector.SOURCE_EXPERIMENT_ID,
            "roster_sha256": roster_sha256,
            "selected": selected,
        }
        with self.assertRaisesRegex(ValueError, "without a verified recovery"):
            selector.validate_source(
                source,
                "recovered",
                expected_artifact_sha256="original",
                expected_roster_sha256=roster_sha256,
            )

    def test_prior_mapping_validation_checks_content_checksum(self) -> None:
        selected = self.fixture()
        roster_sha256 = selector.private_fingerprint(selected)
        mapping = {
            "experiment_id": "prior",
            "roster_sha256": roster_sha256,
            "selected": selected,
        }
        batch_ids, rows = selector.validated_prior_rows(
            [mapping], {"prior": roster_sha256}
        )
        self.assertEqual(batch_ids, {"prior"})
        self.assertEqual(rows, selected)
        mapping["selected"] = selected[:-1]
        with self.assertRaisesRegex(ValueError, "do not match"):
            selector.validated_prior_rows([mapping], {"prior": roster_sha256})


if __name__ == "__main__":
    unittest.main()
