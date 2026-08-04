import unittest

import verify_genre_intelligence_open_set_holdout_audio as subject


class VerifyGenreIntelligenceOpenSetHoldoutAudioTests(unittest.TestCase):
    def test_audit_passes_for_isolated_audio(self) -> None:
        result = subject.audit_hashes(["a" * 64, "b" * 64], ["c" * 64])
        self.assertTrue(result["passed"])
        self.assertEqual(result["cross_partition_decoded_audio_overlap"], 0)

    def test_audit_rejects_cross_partition_duplicate(self) -> None:
        duplicate = "d" * 64
        result = subject.audit_hashes([duplicate], [duplicate])
        self.assertFalse(result["passed"])
        self.assertEqual(result["cross_partition_decoded_audio_overlap"], 1)

    def test_ordered_digest_is_order_sensitive(self) -> None:
        first = subject.ordered_digest(["a" * 64, "b" * 64])
        second = subject.ordered_digest(["b" * 64, "a" * 64])
        self.assertNotEqual(first, second)


if __name__ == "__main__":
    unittest.main()
