import copy
import unittest

import build_genre_intelligence_corpus as corpus
import select_genre_truth_metadata_batch as selector


def row(
    stratum: str,
    index: int,
    *,
    source: str = "current_rekordbox_genre",
    artist: str | None = None,
    new_artist: bool = True,
) -> dict:
    artist_group = artist or f"artist-{stratum}-{index}"
    return {
        "track_id": f"id-{stratum}-{index}",
        "file_path": f"/music/{stratum}/{index}.flac",
        "artist": f"Artist {stratum} {index}",
        "title": f"Title {index}",
        "album": f"Album {index}",
        "artist_group": artist_group,
        "release_group": f"release-{stratum}-{index}",
        "sampling_stratum_private": stratum,
        "sampling_source_private": source,
        "source_value_private": stratum,
        "source_confidence_private": None,
        "source_row_index_private": index,
        "current_genre_private": stratum,
        "baseline_recommendation_private": stratum,
        "baseline_confidence_private": "low",
        "artist_new_to_parent_truth_private": new_artist,
        "release_new_to_parent_truth_private": True,
    }


class SelectGenreTruthMetadataBatchTests(unittest.TestCase):
    def fixture(self) -> list[dict]:
        return [
            row(stratum, index)
            for stratum, count in selector.QUOTAS.items()
            for index in range(count + 2)
        ]

    def test_selection_is_deterministic_and_meets_frozen_constraints(self) -> None:
        rows = self.fixture()
        first = selector.select_batch(rows)
        second = selector.select_batch(list(reversed(copy.deepcopy(rows))))
        self.assertEqual(first, second)
        self.assertEqual(len(first), 20)
        self.assertEqual(
            {
                genre: sum(
                    row["sampling_stratum_private"] == genre for row in first
                )
                for genre in selector.QUOTAS
            },
            selector.QUOTAS,
        )
        self.assertEqual(len({row["release_group"] for row in first}), 20)
        self.assertLessEqual(
            max(
                sum(row["artist_group"] == artist for row in first)
                for artist in {row["artist_group"] for row in first}
            ),
            3,
        )

    def test_current_genre_is_preferred_after_new_artist_minimum(self) -> None:
        rows = self.fixture()
        rows.extend(
            row(
                "Tech House",
                100 + index,
                source="v0_33_recommendation",
            )
            for index in range(8)
        )
        selected = selector.select_batch(rows)
        tech_house = [
            row
            for row in selected
            if row["sampling_stratum_private"] == "Tech House"
        ]
        self.assertTrue(
            all(
                row["sampling_source_private"] == "current_rekordbox_genre"
                for row in tech_house
            )
        )

    def test_new_parent_artist_minimum_fails_closed(self) -> None:
        rows = self.fixture()
        for row_value in rows:
            if row_value["sampling_stratum_private"] == "Garage":
                row_value["artist_new_to_parent_truth_private"] = False
        with self.assertRaisesRegex(ValueError, "Garage.*new parent artists"):
            selector.select_batch(rows)

    def test_unknown_quota_strata_fail_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "frozen stratum order"):
            selector.select_batch(self.fixture(), quotas={"Garage": 1})

    def test_private_config_validates_batch_pair_and_constraints(self) -> None:
        config = {
            "schema_version": 1,
            "experiment_id": "genre-intelligence-truth-v1-b06",
            "source_experiment_id": "genre-intelligence-candidate-pool-v1-b06",
            "quotas": {"Drum & Bass": 2, "Tech House": 1},
            "minimum_new_parent_artists": {
                "Drum & Bass": 1,
                "Tech House": 0,
            },
            "maximum_tracks_per_artist": 2,
            "stratum_order": ["Drum & Bass", "Tech House"],
        }
        normalized = selector.validate_config(config)
        self.assertEqual(normalized["experiment_id"], config["experiment_id"])
        self.assertEqual(
            normalized["stratum_order"], ("Drum & Bass", "Tech House")
        )
        with self.assertRaisesRegex(ValueError, "does not match"):
            selector.validate_config(
                {**config, "source_experiment_id": "wrong-source"}
            )

    def test_private_config_controls_batch_code_and_is_recorded_by_hash(self) -> None:
        config = {
            "schema_version": 1,
            "experiment_id": "genre-intelligence-truth-v1-b06",
            "source_experiment_id": "genre-intelligence-candidate-pool-v1-b06",
            "quotas": {"Tech House": 1},
            "minimum_new_parent_artists": {"Tech House": 0},
            "maximum_tracks_per_artist": 1,
            "stratum_order": ["Tech House"],
        }
        rows = [row("Tech House", 1)]
        fingerprint = corpus.fingerprint(rows)
        result = selector.build_result(
            {
                "experiment_id": config["source_experiment_id"],
                "pool_fingerprint": fingerprint,
                "rows": rows,
            },
            "artifact",
            expected_sha256="artifact",
            expected_fingerprint=fingerprint,
            config=config,
            private_config_sha256="config-hash",
        )
        self.assertEqual(result["selected"][0]["code"], "GI06-01")
        self.assertEqual(
            result["selection_rule"]["private_config_sha256"], "config-hash"
        )

    def test_pool_validation_checks_artifact_and_content_fingerprints(self) -> None:
        rows = self.fixture()
        fingerprint = corpus.fingerprint(rows)
        pool = {
            "experiment_id": selector.SOURCE_EXPERIMENT_ID,
            "pool_fingerprint": fingerprint,
            "rows": rows,
        }
        self.assertEqual(
            selector.validate_pool(
                pool,
                "artifact",
                expected_sha256="artifact",
                expected_fingerprint=fingerprint,
            ),
            rows,
        )
        pool["rows"] = rows[:-1]
        with self.assertRaisesRegex(ValueError, "do not match"):
            selector.validate_pool(
                pool,
                "artifact",
                expected_sha256="artifact",
                expected_fingerprint=fingerprint,
            )


if __name__ == "__main__":
    unittest.main()
