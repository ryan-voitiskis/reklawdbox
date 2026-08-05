import unittest

import prepare_genre_truth_verdicts as prepare


def mapping() -> dict:
    return {
        "experiment_id": "genre-intelligence-truth-v1-b99",
        "selected": [
            {
                "position": 1,
                "code": "GI99-01",
                "artist": "Artist One",
                "title": "Track One",
            },
            {
                "position": 2,
                "code": "GI99-02",
                "artist": "Artist Two",
                "title": "Track Two",
            },
        ],
    }


def review_text(
    first_verdict: str = "Deep House",
    first_confidence: str = "High",
    first_alternatives: str = "Downtempo / breaks",
    second_verdict: str = "unsure",
    second_confidence: str = "low",
    second_alternatives: str = "Jungle, Breaks",
) -> str:
    return (
        "position\tcode\tartist\ttitle\tverdict\tconfidence\talternatives\tnotes\n"
        f"1\tGI99-01\tArtist One\tTrack One\t{first_verdict}\t{first_confidence}"
        f"\t{first_alternatives}\tfirst note\n"
        f"2\tGI99-02\tArtist Two\tTrack Two\t{second_verdict}\t{second_confidence}"
        f"\t{second_alternatives}\tsecond note\n"
    )


class PrepareGenreTruthVerdictsTests(unittest.TestCase):
    def prepare(self, value: str, **kwargs) -> dict:
        return prepare.prepare_verdicts(
            mapping(),
            prepare.read_review_tsv(value),
            reviewer="collection_owner",
            reviewed_at="2026-08-03T16:46:00+10:00",
            **kwargs,
        )

    def test_normalizes_fine_genres_confidence_and_alternatives_losslessly(self) -> None:
        result = self.prepare(review_text())
        label, ambiguous = result["rows"]
        self.assertEqual(label["outcome"], "label")
        self.assertEqual(label["genre"], "House")
        self.assertEqual(label["genre_raw"], "Deep House")
        self.assertEqual(label["confidence"], "high")
        self.assertEqual(label["confidence_raw"], "High")
        self.assertEqual(label["alternatives"], ["Breakbeat", "Downtempo"])
        self.assertEqual(label["alternatives_raw"], "Downtempo / breaks")
        self.assertEqual(ambiguous["outcome"], "ambiguous")
        self.assertIsNone(ambiguous["genre"])
        self.assertEqual(
            ambiguous["alternatives"], ["Breakbeat", "Drum & Bass"]
        )

    def test_mixed_confidence_is_conservatively_medium(self) -> None:
        result = self.prepare(review_text(first_confidence="medium to high"))
        self.assertEqual(result["rows"][0]["confidence"], "medium")
        self.assertEqual(result["rows"][0]["confidence_raw"], "medium to high")

    def test_low_medium_and_very_low_are_conservatively_low(self) -> None:
        result = self.prepare(
            review_text(
                first_confidence="low - medium",
                second_verdict="unsure/ambiguous",
                second_confidence="very low",
            )
        )
        label, ambiguous = result["rows"]
        self.assertEqual(label["confidence"], "low")
        self.assertEqual(label["confidence_raw"], "low - medium")
        self.assertEqual(ambiguous["outcome"], "ambiguous")
        self.assertEqual(ambiguous["confidence"], "low")
        self.assertEqual(ambiguous["genre_raw"], "unsure/ambiguous")

    def test_certain_is_high_and_unsure_none_is_null_losslessly(self) -> None:
        result = self.prepare(
            review_text(first_confidence="certain", second_confidence="none")
        )
        label, ambiguous = result["rows"]
        self.assertEqual(label["confidence"], "high")
        self.assertEqual(label["confidence_raw"], "certain")
        self.assertIsNone(ambiguous["confidence"])
        self.assertEqual(ambiguous["confidence_raw"], "none")

    def test_none_is_rejected_for_a_label_verdict(self) -> None:
        with self.assertRaisesRegex(ValueError, "label verdict requires confidence"):
            self.prepare(review_text(first_confidence="none"))

    def test_primary_parent_is_removed_from_normalized_alternatives(self) -> None:
        result = self.prepare(
            review_text(first_verdict="Deep Techno", first_alternatives="Minimal, Techno")
        )
        self.assertEqual(result["rows"][0]["genre"], "Techno")
        self.assertEqual(result["rows"][0]["alternatives"], ["Minimal"])
        self.assertEqual(result["rows"][0]["alternatives_raw"], "Minimal, Techno")

    def test_unknown_wording_requires_an_explicit_alias(self) -> None:
        value = review_text(first_alternatives="old trance")
        with self.assertRaisesRegex(ValueError, "explicit alias"):
            self.prepare(value)
        result = self.prepare(value, extra_aliases={"old trance": "Trance"})
        self.assertEqual(result["rows"][0]["alternatives"], ["Trance"])

    def test_blank_or_ambiguous_without_alternatives_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "verdict is blank"):
            self.prepare(review_text(first_verdict=""))
        with self.assertRaisesRegex(ValueError, "requires alternatives"):
            self.prepare(review_text(second_alternatives=""))

    def test_identity_or_tsv_shape_drift_is_rejected(self) -> None:
        value = review_text().replace("Artist One", "Different Artist", 1)
        with self.assertRaisesRegex(ValueError, "identity differs"):
            self.prepare(value)
        malformed = review_text().replace("\tfirst note\n", "\tfirst note\textra\n", 1)
        with self.assertRaisesRegex(ValueError, "extra columns"):
            prepare.read_review_tsv(malformed)

    def test_timestamp_requires_timezone_and_supersession_codes_are_checked(self) -> None:
        with self.assertRaisesRegex(ValueError, "include a timezone"):
            prepare.prepare_verdicts(
                mapping(),
                prepare.read_review_tsv(review_text()),
                reviewer="collection_owner",
                reviewed_at="2026-08-03T16:46:00",
            )
        with self.assertRaisesRegex(ValueError, "absent from the mapping"):
            self.prepare(review_text(), supersessions={"unknown": "record"})

    def test_shifted_alternative_cell_can_be_preserved_and_copied_to_notes(self) -> None:
        value = review_text(
            first_alternatives="free-form uncertainty", first_confidence="low"
        ).replace("\tfirst note\n", "\t\n", 1)
        result = self.prepare(value, alternative_notes={"GI99-01"})
        row = result["rows"][0]
        self.assertEqual(row["alternatives"], [])
        self.assertEqual(row["alternatives_raw"], "free-form uncertainty")
        self.assertEqual(
            row["notes"],
            "Non-genre alternative wording: free-form uncertainty",
        )
        self.assertEqual(
            result["normalization"]["alternative_cells_copied_to_notes"],
            ["GI99-01"],
        )

    def test_alternative_as_note_preserves_existing_notes(self) -> None:
        value = review_text(
            first_alternatives="unsure", first_confidence="low"
        )
        result = self.prepare(value, alternative_notes={"GI99-01"})
        row = result["rows"][0]
        self.assertEqual(row["alternatives"], [])
        self.assertEqual(row["alternatives_raw"], "unsure")
        self.assertEqual(
            row["notes"],
            "first note\nNon-genre alternative wording: unsure",
        )


if __name__ == "__main__":
    unittest.main()
