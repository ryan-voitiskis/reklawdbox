import unittest

import prepare_genre_intelligence_candidate as subject


class PrepareGenreIntelligenceCandidateTests(unittest.TestCase):
    def test_artist_folds_are_deterministic_isolated_and_cover_targets(self) -> None:
        rows = []
        for target_index, target in enumerate(subject.RELEASE_SCOPE):
            for artist_index in range(10):
                artist = f"artist-{target_index}-{artist_index}"
                for track_index in range(2):
                    rows.append(
                        {
                            "artist_group": artist,
                            "release_group": f"{artist}\0release-{track_index}",
                            "canonical_parent_genre": target,
                        }
                    )
        first = subject.assign_artist_folds(rows)
        second = subject.assign_artist_folds(list(reversed(rows)))
        self.assertEqual(first, second)
        self.assertEqual(set(first.values()), set(range(subject.FOLD_COUNT)))
        for fold in range(subject.FOLD_COUNT):
            observed = {
                row["canonical_parent_genre"]
                for row in rows
                if first[row["artist_group"]] == fold
            }
            self.assertEqual(observed, set(subject.RELEASE_SCOPE))

    def test_fold_assignment_fails_when_a_target_cannot_cover_every_fold(self) -> None:
        rows = []
        for target in subject.RELEASE_SCOPE:
            artist_count = 4 if target == "Trance" else 5
            for artist_index in range(artist_count):
                rows.append(
                    {
                        "artist_group": f"{target}-{artist_index}",
                        "canonical_parent_genre": target,
                    }
                )
        with self.assertRaisesRegex(ValueError, "missing release targets"):
            subject.assign_artist_folds(rows)


if __name__ == "__main__":
    unittest.main()
