import unittest

import convert_genre_truth_markdown_review as subject


def markdown() -> str:
    return """# Review

### GIP02-01: Artist One – Track One

- Verdict: Techno
- Confidence: medium
- Alternatives: Minimal
- Notes: first line
  continued note

### GIP02-02: Artist Two – Track Two

- Verdict: House
- Confidence: high
- Alternatives:
- Notes:
"""


def mapping() -> dict:
    return {
        "experiment_id": "genre-intelligence-precision-first-v1-h02",
        "selected": [
            {
                "position": 1,
                "code": "GIP02-01",
                "artist": "Artist One",
                "title": "Track One",
            },
            {
                "position": 2,
                "code": "GIP02-02",
                "artist": "Artist Two",
                "title": "Track Two",
            },
        ],
    }


class ConvertGenreTruthMarkdownReviewTests(unittest.TestCase):
    def test_parses_fields_and_continuation_lines(self) -> None:
        rows = subject.parse_markdown(markdown())
        self.assertEqual(rows["GIP02-01"]["Verdict"], "Techno")
        self.assertEqual(rows["GIP02-01"]["Notes"], "first line continued note")
        self.assertEqual(rows["GIP02-02"]["Alternatives"], "")

    def test_tsv_uses_mapping_order_and_exact_identity(self) -> None:
        value = subject.tsv_bytes(
            mapping(), subject.parse_markdown(markdown())
        ).decode()
        lines = value.splitlines()
        self.assertEqual(lines[0].split("\t"), subject.TSV_HEADERS)
        self.assertEqual(
            lines[1].split("\t")[1:4], ["GIP02-01", "Artist One", "Track One"]
        )
        self.assertEqual(
            lines[2].split("\t")[1:4], ["GIP02-02", "Artist Two", "Track Two"]
        )

    def test_rejects_identity_drift_and_missing_answers(self) -> None:
        changed = markdown().replace("Artist One", "Different Artist")
        with self.assertRaisesRegex(ValueError, "identity differs"):
            subject.tsv_bytes(mapping(), subject.parse_markdown(changed))
        blank = markdown().replace("- Verdict: Techno", "- Verdict:")
        with self.assertRaisesRegex(ValueError, "verdict is blank"):
            subject.parse_markdown(blank)


if __name__ == "__main__":
    unittest.main()
