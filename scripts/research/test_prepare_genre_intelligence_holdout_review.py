import unittest

import prepare_genre_intelligence_holdout_review as subject


class PrepareGenreIntelligenceHoldoutReviewTests(unittest.TestCase):
    def predictions(self):
        rows = []
        for position in range(1, subject.EXPECTED_ROWS + 1):
            offered = position <= subject.EXPECTED_OFFERS
            rows.append(
                {
                    "row_id": f"GIH-{position:03d}",
                    "offered": offered,
                    "suggested_parent": "House" if offered else None,
                    "internal_parent": "House",
                    "margin": 0.5,
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
                }
                for position in range(1, subject.EXPECTED_ROWS + 1)
            ]
        }

    def test_review_rows_contain_no_prediction_or_sampling_fields(self) -> None:
        rows = subject.review_rows(self.holdout(), self.predictions())
        self.assertEqual(len(rows), subject.EXPECTED_OFFERS)
        forbidden = {"suggested_parent", "internal_parent", "margin", "stratum"}
        self.assertTrue(all(not (set(row) & forbidden) for row in rows))

    def test_batches_are_capped_at_six_and_cover_every_offer(self) -> None:
        rows = subject.review_rows(self.holdout(), self.predictions())
        batches = subject.batches(rows)
        self.assertEqual([len(batch) for batch in batches], [6, 6, 6, 6, 6, 1])


if __name__ == "__main__":
    unittest.main()
