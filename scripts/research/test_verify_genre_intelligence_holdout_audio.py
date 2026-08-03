import unittest

import verify_genre_intelligence_holdout_audio as subject


class VerifyGenreIntelligenceHoldoutAudioTests(unittest.TestCase):
    def test_audit_passes_for_disjoint_decoded_audio(self) -> None:
        result = subject.audit_hashes(["a" * 64, "b" * 64], ["c" * 64])
        self.assertTrue(result["passed"])
        self.assertEqual(result["cross_partition_decoded_audio_overlap"], 0)

    def test_audit_rejects_cross_partition_decoded_audio(self) -> None:
        result = subject.audit_hashes(["a" * 64], ["a" * 64])
        self.assertFalse(result["passed"])
        self.assertEqual(result["cross_partition_decoded_audio_overlap"], 1)

    def test_ordered_digest_is_order_sensitive(self) -> None:
        self.assertNotEqual(
            subject.ordered_digest(["a" * 64, "b" * 64]),
            subject.ordered_digest(["b" * 64, "a" * 64]),
        )


if __name__ == "__main__":
    unittest.main()
