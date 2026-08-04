import unittest

import prepare_genre_intelligence_open_set as subject


class PrepareGenreIntelligenceOpenSetTests(unittest.TestCase):
    def rows(self) -> list[dict]:
        rows = []
        for bucket_index, bucket in enumerate(subject.FOLD_BUCKETS):
            parent = "Minimal" if bucket == subject.OTHER_CLASS else bucket
            artists = 25 if bucket == subject.OTHER_CLASS else 10
            tracks = 5 if bucket == subject.OTHER_CLASS else 2
            for artist_index in range(artists):
                artist = f"artist-{bucket_index}-{artist_index}"
                for track_index in range(tracks):
                    rows.append(
                        {
                            "artist_group": artist,
                            "canonical_parent_genre": parent,
                            "track": track_index,
                        }
                    )
        return rows

    def test_artist_folds_are_deterministic_isolated_and_balanced(self) -> None:
        rows = self.rows()
        first = subject.assign_artist_folds(rows)
        second = subject.assign_artist_folds(list(reversed(rows)))
        self.assertEqual(first, second)
        self.assertEqual(set(first.values()), set(range(subject.FOLD_COUNT)))
        for fold in range(subject.FOLD_COUNT):
            observed = [
                row
                for row in rows
                if first[str(row["artist_group"])] == fold
            ]
            buckets = {
                subject.fold_bucket(str(row["canonical_parent_genre"]))
                for row in observed
            }
            self.assertEqual(buckets, set(subject.FOLD_BUCKETS))
            self.assertGreaterEqual(
                sum(
                    subject.fold_bucket(str(row["canonical_parent_genre"]))
                    == subject.OTHER_CLASS
                    for row in observed
                ),
                20,
            )

    def test_fold_bucket_maps_only_frozen_outputs_to_themselves(self) -> None:
        self.assertEqual(subject.fold_bucket("House"), "House")
        self.assertEqual(subject.fold_bucket("Tech House"), subject.OTHER_CLASS)
        self.assertEqual(subject.fold_bucket("Drum & Bass"), subject.OTHER_CLASS)

    def test_fold_assignment_rejects_missing_output_bucket(self) -> None:
        rows = [
            row
            for row in self.rows()
            if row["canonical_parent_genre"] != subject.OUTPUT_PARENTS[-1]
        ]
        with self.assertRaisesRegex(ValueError, "every fold bucket"):
            subject.assign_artist_folds(rows)


if __name__ == "__main__":
    unittest.main()
