import unittest

import evaluate_genre_intelligence_holdout as subject


def row(
    suggested: str,
    human: str | None,
    confidence: str | None,
    *,
    outcome: str = "label",
    alternatives: list[str] | None = None,
) -> dict[str, object]:
    return {
        "suggested_parent": suggested,
        "human_parent": human,
        "confidence": confidence,
        "outcome": outcome,
        "alternatives": alternatives or [],
        "exact_match": outcome == "label" and human == suggested,
    }


class EvaluateGenreIntelligenceHoldoutTests(unittest.TestCase):
    def test_exact_primary_only_and_confidence_sensitivity(self) -> None:
        rows = [
            row("House", "House", "high"),
            row("House", "Techno", "low", alternatives=["House"]),
            row("Techno", None, "medium", outcome="ambiguous", alternatives=["Techno"]),
            row("Ambient", None, None, outcome="skip"),
        ]

        result = subject.evaluate_rows(rows)

        self.assertEqual(result["aggregate"]["correct"], 1)
        self.assertEqual(result["aggregate"]["offers"], 4)
        self.assertEqual(result["aggregate"]["offered_precision"], 0.25)
        self.assertEqual(
            result["high_medium_confidence_sensitivity"],
            {"offers": 2, "correct": 1, "incorrect": 1, "offered_precision": 0.5},
        )

    def test_parent_gate_applies_only_at_five_offers(self) -> None:
        rows = [row("Ambient", "Ambient", "high") for _ in range(4)]
        rows += [row("House", "House", "high") for _ in range(26)]

        result = subject.evaluate_rows(rows)

        self.assertIsNone(result["per_suggested_parent"]["Ambient"]["gate_passed"])
        self.assertTrue(result["per_suggested_parent"]["House"]["gate_passed"])
        self.assertTrue(result["metric_gates"]["offers_at_least_30"])
        self.assertTrue(result["metric_gates"]["aggregate_precision_at_least_0_90"])

    def test_parent_precision_boundary_is_inclusive(self) -> None:
        rows = [row("Ambient", "Ambient", "high") for _ in range(4)]
        rows.append(row("Ambient", "House", "high"))
        rows += [row("House", "House", "high") for _ in range(25)]

        result = subject.evaluate_rows(rows)

        ambient = result["per_suggested_parent"]["Ambient"]
        self.assertEqual(ambient["offered_precision"], 0.8)
        self.assertTrue(ambient["gate_passed"])
        self.assertTrue(
            result["metric_gates"]["every_gated_parent_precision_at_least_0_80"]
        )

    def test_export_provenance_does_not_change_original_mapping_hash(self) -> None:
        mapping = {"schema_version": 1, "selected": [{"code": "GIH01-01"}]}
        before = subject.pre_export_mapping_sha256(mapping)
        mapping["export"] = {"xml_path": "/private/export.xml"}

        self.assertEqual(subject.pre_export_mapping_sha256(mapping), before)


if __name__ == "__main__":
    unittest.main()
