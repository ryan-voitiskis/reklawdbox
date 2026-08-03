import copy
import unittest

import build_genre_intelligence_corpus as corpus
import select_genre_truth_active_batch as selector


def row(stratum: str, index: int, artist: str | None = None) -> dict:
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
        "source_recommendation_private": stratum,
        "source_confidence_private": "low",
        "source_row_index": index,
    }


class SelectGenreTruthActiveBatchTests(unittest.TestCase):
    def fixture(self) -> list[dict]:
        return [
            row(stratum, index)
            for stratum, count in selector.QUOTAS.items()
            for index in range(count + 2)
        ]

    def test_selection_is_deterministic_and_meets_frozen_quotas(self) -> None:
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
        self.assertEqual(len({row["artist_group"] for row in first}), 20)
        self.assertEqual(len({row["release_group"] for row in first}), 20)

    def test_private_projection_keeps_model_fields_out_of_review_projection(self) -> None:
        projected = selector.private_row(row("IDM", 1), 1)
        self.assertEqual(projected["code"], "GI04-01")
        self.assertIn("sampling_stratum_private", projected)
        self.assertIn("source_recommendation_private", projected)

    def test_pool_validation_checks_artifact_and_content_fingerprints(self) -> None:
        rows = self.fixture()
        pool = {
            "experiment_id": selector.SOURCE_EXPERIMENT_ID,
            "pool_fingerprint": corpus.fingerprint(rows),
            "rows": rows,
        }
        self.assertEqual(
            selector.validate_pool(
                pool,
                "artifact",
                expected_sha256="artifact",
                expected_fingerprint=corpus.fingerprint(rows),
            ),
            rows,
        )
        pool["rows"] = rows[:-1]
        with self.assertRaisesRegex(ValueError, "do not match"):
            selector.validate_pool(
                pool,
                "artifact",
                expected_sha256="artifact",
                expected_fingerprint=pool["pool_fingerprint"],
            )

    def test_selection_fails_instead_of_relaxing_quota(self) -> None:
        rows = [row("Tech House", index) for index in range(3)]
        with self.assertRaisesRegex(ValueError, "Downtempo.*required 6"):
            selector.select_batch(rows)


if __name__ == "__main__":
    unittest.main()
