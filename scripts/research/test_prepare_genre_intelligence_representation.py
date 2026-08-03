import unittest

from unittest.mock import patch

import prepare_genre_intelligence_candidate as candidate
import prepare_genre_intelligence_representation as subject


class PrepareGenreIntelligenceRepresentationTests(unittest.TestCase):
    def test_output_contains_identity_only_in_source_order(self) -> None:
        source = {
            "stage": "private_label_blind_feature_input",
            "model_ready_corpus_fingerprint": candidate.EXPECTED_MODEL_FINGERPRINT,
            "rows": [{"row_id": "b", "file_path": "/b"}, {"row_id": "a", "file_path": "/a"}],
        }
        with (
            patch.object(subject, "EXPECTED_INPUT_SHA256", "input"),
            patch.object(subject, "EXPECTED_ROWS", 2),
        ):
            result = subject.prepare(source, "input")
        self.assertEqual(result["rows"], source["rows"])
        self.assertEqual(result["corpus_fingerprint"], candidate.EXPECTED_MODEL_FINGERPRINT)

    def test_non_identity_field_is_rejected(self) -> None:
        source = {
            "stage": "private_label_blind_feature_input",
            "model_ready_corpus_fingerprint": candidate.EXPECTED_MODEL_FINGERPRINT,
            "rows": [{"row_id": str(index), "file_path": f"/{index}"} for index in range(575)],
        }
        source["rows"][0]["truth"] = "House"
        with patch.object(subject, "EXPECTED_INPUT_SHA256", "input"):
            with self.assertRaisesRegex(ValueError, "non-identity"):
                subject.prepare(source, "input")


if __name__ == "__main__":
    unittest.main()
