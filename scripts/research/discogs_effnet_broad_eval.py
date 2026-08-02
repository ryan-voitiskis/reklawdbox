#!/usr/bin/env python3
"""Evaluate frozen Discogs-EffNet style scores as selective broad genres."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np


EXPERIMENT_ID = "discogs-effnet-broad-genre-v1"
METHOD_STATUS = "pre_registered_cross_fitted_development_evaluation"
RULE_VERSION = "broad-parent-consensus-v1"
EXPECTED_BROAD_SEMANTIC_SHA256 = (
    "efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255"
)
EXPECTED_CORPUS_FINGERPRINT = (
    "sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061"
)
EXPECTED_MANIFEST_SHA256 = (
    "a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9"
)
EXPECTED_FEATURE_SHA256 = (
    "5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd"
)
EXPECTED_SOURCE_RESULT_SHA256 = (
    "57099b002344c4840a80f75db5b73b81bc8679336a6f8d8c5b8ff749511d62da"
)

CANONICAL = [
    "2-Step Garage",
    "Acid",
    "Afro House",
    "Ambient",
    "Ambient Techno",
    "Bassline",
    "Breakbeat",
    "Broken Beat",
    "Dancehall",
    "Deep House",
    "Deep Techno",
    "Disco",
    "Downtempo",
    "Drum & Bass",
    "Dub",
    "Dub Techno",
    "Dubstep",
    "EBM",
    "Electro",
    "Experimental",
    "Footwork",
    "Future Garage",
    "Gabber",
    "Garage",
    "Gospel House",
    "Grime",
    "Happy Hardcore",
    "Hard Techno",
    "Hard Trance",
    "Hardcore",
    "Hardstyle",
    "Highlife",
    "Hip Hop",
    "House",
    "IDM",
    "Italo Disco",
    "Italodance",
    "Jazz",
    "Jungle",
    "Minimal",
    "Pop",
    "Progressive House",
    "Psytrance",
    "R&B",
    "Reggae",
    "Rock",
    "Speed Garage",
    "Synth-pop",
    "Tech House",
    "Techno",
    "Trance",
    "Trip-Hop",
    "UK Funky",
]

GROUPS = {
    "House": {"Afro House", "Deep House", "Gospel House", "House", "Progressive House"},
    "Techno": {"Ambient Techno", "Deep Techno", "Dub Techno", "Hard Techno", "Techno"},
    "Trance": {"Hard Trance", "Psytrance", "Trance"},
    "Garage": {
        "2-Step Garage",
        "Bassline",
        "Future Garage",
        "Garage",
        "Speed Garage",
        "UK Funky",
    },
    "Breakbeat": {"Breakbeat", "Broken Beat"},
    "Drum & Bass": {"Drum & Bass", "Jungle"},
    "Reggae": {"Dancehall", "Dub", "Reggae"},
    "Disco": {"Disco", "Italo Disco"},
    "Hardcore": {"Gabber", "Happy Hardcore", "Hardcore", "Hardstyle"},
    "Downtempo": {"Downtempo", "Trip-Hop"},
    "Pop": {"Italodance", "Pop", "Synth-pop"},
}
SELF_PARENT = {
    "Acid",
    "Ambient",
    "Dubstep",
    "EBM",
    "Electro",
    "Footwork",
    "Grime",
    "Highlife",
    "Hip Hop",
    "IDM",
    "Jazz",
    "Minimal",
    "R&B",
    "Rock",
    "Tech House",
}


def fine_to_broad() -> dict[str, str | None]:
    mapping: dict[str, str | None] = {genre: None for genre in CANONICAL}
    for broad, genres in GROUPS.items():
        for genre in genres:
            if mapping[genre] is not None:
                raise ValueError(f"duplicate broad mapping for {genre}")
            mapping[genre] = broad
    for genre in SELF_PARENT:
        if mapping[genre] is not None:
            raise ValueError(f"duplicate broad mapping for {genre}")
        mapping[genre] = genre
    if mapping["Experimental"] is not None:
        raise ValueError("Experimental must remain unmodeled")
    missing = [genre for genre, broad in mapping.items() if broad is None and genre != "Experimental"]
    if missing:
        raise ValueError(f"missing broad mappings: {missing}")
    return mapping


FINE_TO_BROAD = fine_to_broad()
BROAD_TARGETS = list(dict.fromkeys(FINE_TO_BROAD[genre] for genre in CANONICAL if FINE_TO_BROAD[genre]))
BROAD_INDEX = {genre: index for index, genre in enumerate(BROAD_TARGETS)}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def broad_semantic_sha256() -> str:
    mapping = "\n".join(
        f"{genre}=>{FINE_TO_BROAD[genre] or '<unmodeled>'}" for genre in CANONICAL
    )
    return hashlib.sha256(f"{RULE_VERSION}\n{mapping}".encode()).hexdigest()


def broad_scores(style_scores: np.ndarray) -> np.ndarray:
    if style_scores.ndim != 2 or style_scores.shape[1] != len(CANONICAL):
        raise ValueError(
            f"style score shape {style_scores.shape} must be (rows, {len(CANONICAL)})"
        )
    result = np.zeros((style_scores.shape[0], len(BROAD_TARGETS)), dtype=np.float64)
    for broad, broad_index in BROAD_INDEX.items():
        fine_indices = [
            index for index, fine in enumerate(CANONICAL) if FINE_TO_BROAD[fine] == broad
        ]
        result[:, broad_index] = np.max(style_scores[:, fine_indices], axis=1)
    return result


def top_predictions_and_margins(scores: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    if scores.ndim != 2 or scores.shape[1] < 2:
        raise ValueError("broad scores must contain at least two targets")
    predictions = np.argmax(scores, axis=1)
    top = scores[np.arange(scores.shape[0]), predictions]
    second = np.partition(scores, -2, axis=1)[:, -2]
    return predictions.astype(np.int64), np.maximum(0.0, top - second)


def safe_fraction(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 0.0


def safe_f1(precision: float, recall: float) -> float:
    return 2 * precision * recall / (precision + recall) if precision + recall else 0.0


def choose_threshold(
    margins: np.ndarray,
    correct: np.ndarray,
    minimum_offers: int,
) -> dict[str, float | int] | None:
    candidates = sorted(float(value) for value in np.unique(margins))
    best: tuple[int, float, float] | None = None
    for threshold in candidates:
        offered = margins >= threshold
        offers = int(np.sum(offered))
        if offers < minimum_offers:
            continue
        precision = safe_fraction(int(np.sum(correct & offered)), offers)
        if precision < 0.90 - 1e-12:
            continue
        candidate = (offers, precision, threshold)
        if best is None or candidate > best:
            best = candidate
    if best is None:
        return None
    offers, precision, threshold = best
    return {"threshold": threshold, "offers": offers, "offered_precision": precision}


def cross_fitted_offers(
    predictions: np.ndarray,
    margins: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
) -> tuple[np.ndarray, list[dict[str, Any]]]:
    offered = np.zeros(len(truths), dtype=bool)
    details: list[dict[str, Any]] = []
    correct = predictions == truths
    for fold in sorted(int(value) for value in np.unique(folds)):
        train = folds != fold
        held_out = folds == fold
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(train))))
        selected = choose_threshold(margins[train], correct[train], minimum_offers)
        if selected is None:
            details.append(
                {
                    "fold": fold,
                    "threshold": None,
                    "minimum_training_offers": minimum_offers,
                    "training_offers": 0,
                    "training_coverage": 0.0,
                    "training_offered_precision": 0.0,
                }
            )
            continue
        threshold = float(selected["threshold"])
        offered[held_out] = margins[held_out] >= threshold
        details.append(
            {
                "fold": fold,
                "threshold": threshold,
                "minimum_training_offers": minimum_offers,
                "training_offers": int(selected["offers"]),
                "training_coverage": safe_fraction(int(selected["offers"]), int(np.sum(train))),
                "training_offered_precision": float(selected["offered_precision"]),
            }
        )
    return offered, details


def metrics(
    truths: np.ndarray,
    predictions: np.ndarray,
    offered: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    eligible_rows = len(truths)
    offers = int(np.sum(offered))
    correct = predictions == truths
    correct_offers = int(np.sum(correct & offered))
    per_target: dict[str, Any] = {}
    for target_index in sorted(set(int(value) for value in truths)):
        target = BROAD_TARGETS[target_index]
        truth_mask = truths == target_index
        predicted_mask = (predictions == target_index) & offered
        support = int(np.sum(truth_mask))
        target_offers = int(np.sum(predicted_mask))
        target_correct = int(np.sum(truth_mask & predicted_mask))
        target_abstentions = int(np.sum(truth_mask & ~offered))
        precision = safe_fraction(target_correct, target_offers)
        recall = safe_fraction(target_correct, support)
        confusion = Counter(
            "<abstain>" if not offered[index] else BROAD_TARGETS[int(predictions[index])]
            for index in np.where(truth_mask)[0]
            if not (offered[index] and predictions[index] == target_index)
        )
        per_target[target] = {
            "support": support,
            "offers": target_offers,
            "correct_offers": target_correct,
            "abstentions": target_abstentions,
            "offered_precision": precision,
            "recall": recall,
            "f1": safe_f1(precision, recall),
            "leading_confusions": [
                {"recommended": recommended, "count": count}
                for recommended, count in sorted(
                    confusion.items(), key=lambda item: (-item[1], item[0])
                )[:3]
            ],
        }
    fold_metrics = []
    for fold in sorted(int(value) for value in np.unique(folds)):
        mask = folds == fold
        fold_offers = int(np.sum(offered & mask))
        fold_correct = int(np.sum(correct & offered & mask))
        fold_metrics.append(
            {
                "fold": fold,
                "eligible_rows": int(np.sum(mask)),
                "offers": fold_offers,
                "correct_offers": fold_correct,
                "coverage": safe_fraction(fold_offers, int(np.sum(mask))),
                "offered_precision": safe_fraction(fold_correct, fold_offers),
            }
        )
    return {
        "eligible_rows": eligible_rows,
        "offers": offers,
        "correct_offers": correct_offers,
        "abstentions": eligible_rows - offers,
        "coverage": safe_fraction(offers, eligible_rows),
        "offered_precision": safe_fraction(correct_offers, offers),
        "accuracy": safe_fraction(correct_offers, eligible_rows),
        "macro_recall": float(np.mean([row["recall"] for row in per_target.values()])),
        "macro_f1": float(np.mean([row["f1"] for row in per_target.values()])),
        "per_target": per_target,
        "folds": fold_metrics,
    }


def gate(unselective: dict[str, Any], candidate: dict[str, Any]) -> dict[str, Any]:
    supported_failures = [
        target
        for target, row in candidate["per_target"].items()
        if row["support"] >= 10
        and row["offers"] >= 5
        and row["offered_precision"] < 0.75 - 1e-12
    ]
    checks = {
        "offered_precision_at_least_0_90": candidate["offered_precision"] >= 0.90 - 1e-12,
        "coverage_at_least_0_50": candidate["coverage"] >= 0.50 - 1e-12,
        "every_fold_precision_at_least_0_85": all(
            row["offers"] > 0 and row["offered_precision"] >= 0.85 - 1e-12
            for row in candidate["folds"]
        ),
        "supported_target_precision_at_least_0_75": not supported_failures,
        "precision_improvement_at_least_0_10": candidate["offered_precision"]
        >= unselective["offered_precision"] + 0.10 - 1e-12,
        "supported_target_failures": supported_failures,
    }
    checks["passed"] = all(
        value for key, value in checks.items() if key != "supported_target_failures"
    )
    return checks


def run(args: argparse.Namespace) -> dict[str, Any]:
    hashes = {
        "manifest_sha256": sha256_file(args.manifest),
        "feature_sha256": sha256_file(args.features),
        "source_result_sha256": sha256_file(args.source_result),
    }
    expected_hashes = {
        "manifest_sha256": EXPECTED_MANIFEST_SHA256,
        "feature_sha256": EXPECTED_FEATURE_SHA256,
        "source_result_sha256": EXPECTED_SOURCE_RESULT_SHA256,
    }
    if hashes != expected_hashes:
        raise ValueError(f"input hashes differ from frozen values: {hashes}")
    if broad_semantic_sha256() != EXPECTED_BROAD_SEMANTIC_SHA256:
        raise ValueError("broad taxonomy semantic checksum changed")

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    source_result = json.loads(args.source_result.read_text(encoding="utf-8"))
    if manifest["corpus_fingerprint"] != EXPECTED_CORPUS_FINGERPRINT:
        raise ValueError("manifest corpus fingerprint changed")
    if source_result["corpus_fingerprint"] != EXPECTED_CORPUS_FINGERPRINT:
        raise ValueError("source result corpus fingerprint changed")

    artifact = np.load(args.features)
    style_scores = np.asarray(artifact["style_scores"], dtype=np.float64)
    truth_indices = np.asarray(artifact["truth_indices"], dtype=np.int64)
    folds = np.asarray(artifact["folds"], dtype=np.int64)
    rows = manifest["rows"]
    if len(rows) != len(style_scores) or len(rows) != len(truth_indices) or len(rows) != len(folds):
        raise ValueError("manifest and feature row counts differ")
    for index, row in enumerate(rows):
        if CANONICAL[int(truth_indices[index])] != row["truth"]:
            raise ValueError(f"truth alignment differs at row {index}")
        if int(folds[index]) != int(row["fold"]):
            raise ValueError(f"fold alignment differs at row {index}")

    eligible = np.asarray(
        [FINE_TO_BROAD[CANONICAL[int(index)]] is not None for index in truth_indices],
        dtype=bool,
    )
    broad_truths = np.asarray(
        [
            BROAD_INDEX[FINE_TO_BROAD[CANONICAL[int(index)]]]
            for index in truth_indices[eligible]
        ],
        dtype=np.int64,
    )
    scores = broad_scores(style_scores[eligible])
    predictions, margins = top_predictions_and_margins(scores)
    eligible_folds = folds[eligible]
    all_offered = np.ones(len(predictions), dtype=bool)
    unselective = metrics(broad_truths, predictions, all_offered, eligible_folds)
    selected_offers, thresholds = cross_fitted_offers(
        predictions, margins, broad_truths, eligible_folds
    )
    candidate = metrics(broad_truths, predictions, selected_offers, eligible_folds)
    for detail, fold_metric in zip(thresholds, candidate["folds"], strict=True):
        detail["held_out_eligible_rows"] = fold_metric["eligible_rows"]
        detail["held_out_offers"] = fold_metric["offers"]
        detail["held_out_coverage"] = fold_metric["coverage"]
        detail["held_out_offered_precision"] = fold_metric["offered_precision"]
    gate_result = gate(unselective, candidate)
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "corpus_fingerprint": EXPECTED_CORPUS_FINGERPRINT,
        "inputs": hashes,
        "broad_semantic_sha256": EXPECTED_BROAD_SEMANTIC_SHA256,
        "aggregation": "maximum fine-style score per broad target",
        "confidence": "top broad score minus second broad score",
        "rows": len(rows),
        "eligible_rows": int(np.sum(eligible)),
        "excluded_unmodeled_truth_rows": int(np.sum(~eligible)),
        "broad_targets": len(BROAD_TARGETS),
        "configurations": {
            "unselective_style_projection": unselective,
            "cross_fitted_margin_selection": candidate,
        },
        "fold_thresholds": thresholds,
        "gate": gate_result,
        "outcome": (
            "effnet_broad_candidate_passed_development_gate"
            if gate_result["passed"]
            else "effnet_broad_candidate_failed_development_gate"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--features", required=True, type=Path)
    parser.add_argument("--source-result", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "eligible_rows": result["eligible_rows"],
                "candidate": result["configurations"]["cross_fitted_margin_selection"],
                "gate": result["gate"],
                "outcome": result["outcome"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
