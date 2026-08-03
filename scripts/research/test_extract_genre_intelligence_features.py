import unittest

import numpy as np

import extract_genre_intelligence_features as subject


class ExtractGenreIntelligenceFeaturesTests(unittest.TestCase):
    def test_vector_values_use_frozen_profile_and_timbral_order(self) -> None:
        stratum = {
            "bpm": 120,
            "decay_mid_tau": 2,
            "decay_high_tau": 3,
            "key_clarity": 0.5,
        }
        essentia = {
            field: index + 1
            for index, (_, source, field) in enumerate(subject.PROFILE_FEATURES)
            if source == "essentia"
        }
        essentia["mfcc_mean"] = list(range(13))
        essentia["mfcc_std"] = list(range(20, 33))
        essentia["spectral_contrast_mean"] = list(range(40, 46))
        values = subject.vector_values(stratum, essentia)
        self.assertEqual(values.shape, (29,))
        np.testing.assert_array_equal(values[13:21], np.arange(1, 9))
        np.testing.assert_array_equal(values[21:26], np.arange(21, 26))
        np.testing.assert_array_equal(values[26:], np.asarray([40, 42, 44]))

    def test_missing_values_are_nan_and_baseline_mapping_is_parent_only(self) -> None:
        values = subject.vector_values({}, {})
        self.assertTrue(np.all(np.isnan(values)))
        self.assertEqual(subject.baseline_parent("Deep House"), "House")
        self.assertEqual(subject.baseline_parent("Jungle"), "Drum & Bass")
        self.assertIsNone(subject.baseline_parent(None))

    def test_feature_schema_has_expected_fixed_width(self) -> None:
        self.assertEqual(len(subject.VALUE_FEATURES), 29)
        self.assertEqual(len(subject.MISSINGNESS_FEATURES), 29)
        self.assertEqual(len(subject.BASELINE_FEATURES), 8)
        self.assertEqual(len(subject.FEATURE_NAMES), 140)


if __name__ == "__main__":
    unittest.main()
