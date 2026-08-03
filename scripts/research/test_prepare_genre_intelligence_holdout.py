import unittest

import prepare_genre_intelligence_holdout as subject


class PrepareGenreIntelligenceHoldoutTests(unittest.TestCase):
    def rows(self):
        selected = []
        audit = []
        for position in range(1, subject.EXPECTED_ROWS + 1):
            path = f"/music/holdout-{position}.flac"
            selected.append(
                {
                    "position": position,
                    "file_path": path,
                    "track_id": position,
                    "artist_group": f"holdout-artist-{position}",
                    "release_group": f"holdout-release-{position}",
                }
            )
            audit.append({"file_path": path, "track_id": position})
        development = [
            {
                "row_id": "development-1",
                "artist_group": "development-artist",
                "release_group": "development-release",
            }
        ]
        development_features = [
            {"row_id": "development-1", "file_path": "/music/development.flac"}
        ]
        corpus = [{"file_path": "/music/reviewed.flac"}]
        return selected, audit, development, development_features, corpus

    def test_preparation_emits_only_opaque_identity_and_path(self) -> None:
        rows, leakage = subject.prepare_rows(*self.rows())
        self.assertEqual(
            rows[0],
            {"row_id": "GIH-001", "file_path": "/music/holdout-1.flac"},
        )
        self.assertEqual(len(rows), subject.EXPECTED_ROWS)
        self.assertTrue(all(value == 0 for value in leakage.values()))

    def test_preparation_rejects_development_artist_leakage(self) -> None:
        selected, audit, development, development_features, corpus = self.rows()
        selected[0]["artist_group"] = "development-artist"
        with self.assertRaisesRegex(ValueError, "holdout leakage detected"):
            subject.prepare_rows(
                selected, audit, development, development_features, corpus
            )

    def test_preparation_rejects_prior_truth_path_leakage(self) -> None:
        selected, audit, development, development_features, corpus = self.rows()
        corpus[0]["file_path"] = selected[0]["file_path"]
        with self.assertRaisesRegex(ValueError, "holdout leakage detected"):
            subject.prepare_rows(
                selected, audit, development, development_features, corpus
            )


if __name__ == "__main__":
    unittest.main()
