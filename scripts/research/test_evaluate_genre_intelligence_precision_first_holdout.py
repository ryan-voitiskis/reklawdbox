import unittest

import evaluate_genre_intelligence_precision_first_holdout as subject


def row(
    suggested: str,
    human: str | None,
    confidence: str | None,
    *,
    v033: str | None = None,
    outcome: str = "label",
    alternatives: list[str] | None = None,
) -> dict[str, object]:
    return {
        "suggested_parent": suggested,
        "v033_parent": v033,
        "human_parent": human,
        "confidence": confidence,
        "outcome": outcome,
        "alternatives": alternatives or [],
        "exact_match": outcome == "label" and human == suggested,
        "v033_exact_match": outcome == "label" and human == v033,
    }


class EvaluateGenreIntelligencePrecisionFirstHoldoutTests(unittest.TestCase):
    def test_exact_primary_only_and_confidence_sensitivity(self) -> None:
        rows = [
            row("House", "House", "high", v033="House"),
            row("House", "Techno", "low", v033="Techno", alternatives=["House"]),
            row(
                "Techno",
                None,
                "medium",
                v033="Techno",
                outcome="ambiguous",
                alternatives=["Techno"],
            ),
            row("Ambient", None, None, outcome="skip"),
        ]

        result = subject.evaluate_rows(rows)

        self.assertEqual(result["aggregate"]["correct"], 1)
        self.assertEqual(result["aggregate"]["offered_precision"], 0.25)
        self.assertEqual(
            result["high_medium_confidence_sensitivity"],
            {"offers": 2, "correct": 1, "incorrect": 1, "offered_precision": 0.5},
        )

    def test_parent_precision_boundary_is_inclusive(self) -> None:
        rows = [row("Ambient", "Ambient", "high") for _ in range(4)]
        rows.append(row("Ambient", "House", "high"))
        rows += [row("House", "House", "high") for _ in range(30)]

        result = subject.evaluate_rows(rows)

        self.assertEqual(
            result["per_suggested_parent"]["Ambient"]["offered_precision"], 0.8
        )
        self.assertTrue(result["per_suggested_parent"]["Ambient"]["gate_passed"])

    def test_paired_baseline_uses_only_rows_where_v033_offers(self) -> None:
        rows = [
            row("House", "House", "high", v033="Techno"),
            row("Techno", "Techno", "high", v033=None),
            row("Ambient", "House", "high", v033="House"),
        ]

        result = subject.paired_v033(rows)

        self.assertEqual(result["paired_offers"], 2)
        self.assertEqual(result["o3_offered_precision"], 0.5)
        self.assertEqual(result["v033_offered_precision"], 0.5)
        self.assertEqual(result["precision_improvement"], 0.0)

    def test_all_metric_gates_pass_at_frozen_boundaries(self) -> None:
        rows = [row("House", "House", "high", v033="Techno") for _ in range(35)]

        result = subject.evaluate_rows(rows)

        self.assertTrue(all(result["metric_gates"].values()))

    def test_no_paired_v033_rows_fails_comparison_gate(self) -> None:
        rows = [row("House", "House", "high") for _ in range(35)]

        result = subject.evaluate_rows(rows)

        self.assertIsNone(result["paired_v033"]["precision_improvement"])
        self.assertFalse(
            result["metric_gates"][
                "paired_v033_precision_improvement_at_least_0_05"
            ]
        )

    def test_batch_label_maps_hidden_holdout_suffix_to_review_label(self) -> None:
        self.assertEqual(
            subject.batch_label("genre-intelligence-precision-first-v1-h06"),
            "P06",
        )
        with self.assertRaisesRegex(ValueError, "unexpected review batch"):
            subject.batch_label("genre-intelligence-holdout-v1-h06")


if __name__ == "__main__":
    unittest.main()
