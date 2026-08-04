import unittest

import prepare_genre_intelligence_precision_first_review as subject


class PrepareGenreIntelligencePrecisionFirstReviewTests(unittest.TestCase):
    def predictions(self):
        rows = []
        for position in range(1, subject.EXPECTED_ROWS + 1):
            offered = position <= subject.EXPECTED_OFFERS
            rows.append(
                {
                    "row_id": f"GIP-{position:03d}",
                    "offered": offered,
                    "suggested_parent": "House" if offered else None,
                    "qualified_parents": ["House"] if offered else [],
                    "scores": {"House": 0.9},
                    "v033_parent": "House",
                }
            )
        return {"offers": subject.EXPECTED_OFFERS, "rows": rows}

    def holdout(self):
        return {
            "selected": [
                {
                    "position": position,
                    "track_id": position,
                    "file_path": f"/music/{position}.flac",
                    "artist": f"Artist {position}",
                    "title": f"Title {position}",
                    "album": None,
                    "artist_group": f"artist-{position}",
                    "release_group": f"release-{position}",
                    "broad_sampling_stratum": "Techno",
                    "current_genre_sampling_only": "Techno",
                }
                for position in range(1, subject.EXPECTED_ROWS + 1)
            ]
        }

    def test_review_rows_contain_no_prediction_or_sampling_fields(self) -> None:
        rows = subject.review_rows(self.holdout(), self.predictions())
        self.assertEqual(len(rows), subject.EXPECTED_OFFERS)
        forbidden = {
            "suggested_parent",
            "qualified_parents",
            "scores",
            "v033_parent",
            "broad_sampling_stratum",
            "current_genre_sampling_only",
        }
        self.assertTrue(all(not (set(row) & forbidden) for row in rows))

    def test_batches_are_capped_at_six_and_cover_every_offer(self) -> None:
        rows = subject.review_rows(self.holdout(), self.predictions())
        batches = subject.batches(rows)
        self.assertEqual([len(batch) for batch in batches], [6, 6, 6, 6, 6, 5])

    def test_mapping_uses_opaque_precision_first_codes(self) -> None:
        rows = subject.review_rows(self.holdout(), self.predictions())
        mapping = subject.mapping_for_batch(rows[:6], 1)
        self.assertEqual(
            mapping["experiment_id"], "genre-intelligence-precision-first-v1-h01"
        )
        self.assertEqual(
            [row["code"] for row in mapping["selected"]],
            [f"GIP01-{position:02d}" for position in range(1, 7)],
        )


if __name__ == "__main__":
    unittest.main()
