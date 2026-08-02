from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

import numpy as np


MODULE_PATH = Path(__file__).with_name("discogs_effnet_genre_audit.py")
SPEC = importlib.util.spec_from_file_location("discogs_effnet_genre_audit", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(MODULE_PATH.parent))
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def row(
    track_id: str,
    target: str,
    current: str,
    *,
    family_cross: bool = True,
    confidence: str = "high",
    precision: float = 0.8,
    margin: float = 0.2,
    bpm: float = 125.0,
    group: str | None = None,
) -> dict[str, object]:
    return {
        "track_id": track_id,
        "file_path": f"/{track_id}.wav",
        "artist": f"Artist {track_id}",
        "title": f"Title {track_id}",
        "album": f"Album {track_id}",
        "current_genre": current,
        "hidden_target": target,
        "baseline_confidence": confidence,
        "target_precision": precision,
        "target_support": 10,
        "fusion_margin": margin,
        "style_margin": margin / 2,
        "style_score": 0.4,
        "fusion_score": 0.7,
        "release_group": group or f"group-{track_id}",
        "stable_hash": MODULE.stable_hash(track_id),
        "cross_family": family_cross,
        "bpm": bpm,
    }


class GenreAuditTests(unittest.TestCase):
    def test_top_two_margin_is_deterministic_on_ties(self) -> None:
        scores = np.zeros(len(MODULE.base.CANONICAL), dtype=np.float64)
        scores[MODULE.base.CANONICAL_INDEX["House"]] = 0.7
        scores[MODULE.base.CANONICAL_INDEX["Techno"]] = 0.7
        genre, score, margin = MODULE.top_two_margin(
            scores,
            [
                MODULE.base.CANONICAL_INDEX["House"],
                MODULE.base.CANONICAL_INDEX["Techno"],
            ],
        )
        self.assertEqual(genre, "House")
        self.assertEqual(score, 0.7)
        self.assertEqual(margin, 0.0)

    def test_full_centroids_fit_only_development_rows(self) -> None:
        embeddings = np.asarray([[1.0, 0.0], [0.8, 0.2], [0.0, 1.0], [0.2, 0.8]])
        arrangement = np.asarray(
            [[1.0, 2.0], [1.2, 2.2], [8.0, 9.0], [8.2, 9.2]], dtype=np.float64
        )
        state = MODULE.fit_full_centroids(
            embeddings, arrangement, ["House", "House", "Techno", "Techno"]
        )
        self.assertEqual(set(state["embedding_centroids"]), {"House", "Techno"})
        self.assertEqual(state["arrangement_centroids"]["House"].shape, (2,))
        self.assertTrue(np.isfinite(state["means"]).all())

        style_scores = np.zeros((1, len(MODULE.base.CANONICAL)), dtype=np.float64)
        style_scores[0, MODULE.base.CANONICAL_INDEX["House"]] = 0.9
        prediction = MODULE.external_fusion_predictions(
            style_scores,
            np.asarray([[1.0, 0.0]]),
            np.asarray([[1.1, 2.1]]),
            state,
            {"House", "Techno"},
        )[0]
        self.assertEqual(prediction["genre"], "House")
        self.assertGreater(prediction["margin"], 0.0)

    def test_target_qualification_enforces_support_and_precision(self) -> None:
        stage_b = {
            "configurations": {
                "fixed_70_20_10_fusion": {
                    "metrics": {
                        "per_genre": {
                            "House": {"support": 8, "precision": 0.60},
                            "Techno": {"support": 7, "precision": 1.0},
                            "Dub": {"support": 20, "precision": 0.59},
                        }
                    }
                }
            }
        }
        self.assertEqual(set(MODULE.qualifying_targets(stage_b)), {"House"})

    def test_enrichment_requires_three_way_agreement_and_confidence(self) -> None:
        rows = []
        for track_id, current, confidence in [
            ("ranked", "Techno", "high"),
            ("control", "House", "medium"),
            ("weak", "Techno", "low"),
        ]:
            rows.append(
                {
                    "track_id": track_id,
                    "file_path": f"/{track_id}.wav",
                    "artist": f"Artist {track_id}",
                    "title": f"Title {track_id}",
                    "album": f"Album {track_id}",
                    "current_genre": current,
                    "baseline_recommendation": "House",
                    "baseline_confidence": confidence,
                    "bpm": 125.0,
                }
            )
        style_scores = np.zeros((3, len(MODULE.base.CANONICAL)), dtype=np.float64)
        style_scores[:, MODULE.base.CANONICAL_INDEX["House"]] = 0.8
        fusion = [{"genre": "House", "score": 0.7, "margin": 0.2}] * 3
        ranked, controls = MODULE.enrich_rows(
            rows,
            style_scores,
            fusion,
            {"House": {"support": 10, "precision": 0.8}},
        )
        self.assertEqual([item["track_id"] for item in ranked], ["ranked"])
        self.assertEqual([item["track_id"] for item in controls], ["control"])

    def test_ranked_selection_enforces_target_family_and_release_diversity(self) -> None:
        rows = [
            row("a", "House", "Techno"),
            row("b", "House", "Ambient", margin=0.9),
            row("c", "Deep House", "Techno"),
            row("d", "Tech House", "Ambient"),
            row("e", "Electro", "House"),
            row("f", "Dub", "Techno"),
            row("g", "Hardcore", "Ambient"),
        ]
        selected = MODULE.select_ranked(rows)
        self.assertEqual(len(selected), MODULE.RANKED_COUNT)
        self.assertEqual(len({item["hidden_target"] for item in selected}), 4)
        self.assertEqual(len({item["release_group"] for item in selected}), 4)
        families = [MODULE.base.family(item["hidden_target"]) for item in selected]
        self.assertLessEqual(max(families.count(value) for value in set(families)), 2)

    def test_ranked_selection_fails_without_quota_relaxation(self) -> None:
        with self.assertRaisesRegex(ValueError, "required 4"):
            MODULE.select_ranked([row("a", "House", "Techno")])

    def test_controls_match_distinct_targets_and_nearest_bpm(self) -> None:
        ranked = [
            {**row("r1", "House", "Techno", bpm=124.0), "cohort": "ranked"},
            {**row("r2", "Electro", "House", bpm=132.0), "cohort": "ranked"},
            {**row("r3", "Dub", "Techno", bpm=110.0), "cohort": "ranked"},
            {**row("r4", "Hardcore", "Ambient", bpm=170.0), "cohort": "ranked"},
        ]
        controls = [
            row("h-far", "House", "House", bpm=118.0),
            row("h-near", "House", "House", bpm=123.5),
            row("e-near", "Electro", "Electro", bpm=131.0),
        ]
        selected = MODULE.select_controls(controls, ranked)
        self.assertEqual([item["track_id"] for item in selected], ["h-near", "e-near"])
        self.assertEqual({item["hidden_target"] for item in selected}, {"House", "Electro"})


if __name__ == "__main__":
    unittest.main()
