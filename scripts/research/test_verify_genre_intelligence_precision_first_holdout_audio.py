import unittest

import verify_genre_intelligence_precision_first_holdout_audio as subject


class VerifyGenreIntelligencePrecisionFirstHoldoutAudioTests(unittest.TestCase):
    def test_audit_passes_for_isolated_partitions(self) -> None:
        result = subject.audit_partitions(
            [("development", ["a" * 64]), ("old", ["b" * 64])],
            ["c" * 64, "d" * 64],
        )
        self.assertTrue(result["passed"])
        self.assertEqual(result["cross_partition_decoded_audio_overlap"], 0)
        self.assertEqual(result["reference_rows"], 2)

    def test_audit_rejects_duplicate_with_any_reference(self) -> None:
        duplicate = "d" * 64
        result = subject.audit_partitions(
            [("development", ["a" * 64]), ("old", [duplicate])],
            [duplicate],
        )
        self.assertFalse(result["passed"])
        self.assertEqual(result["per_reference"]["old"]["holdout_overlap"], 1)

    def test_audit_rejects_duplicate_inside_holdout(self) -> None:
        duplicate = "e" * 64
        result = subject.audit_partitions(
            [("development", ["a" * 64])], [duplicate, duplicate]
        )
        self.assertFalse(result["passed"])


if __name__ == "__main__":
    unittest.main()
