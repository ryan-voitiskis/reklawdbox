#!/usr/bin/env python3
"""Evaluate frozen Plan 065 kick-rhythm augmentation for broad genre."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as broad
import discogs_effnet_supervised_broad_eval as supervised
import extract_kick_rhythm_features as kick


EXPERIMENT_ID = "discogs-effnet-kick-rhythm-broad-v1"
METHOD_STATUS = "pre_registered_nested_cross_fitted_development_evaluation"
EXPECTED_KICK_ARTIFACT_SHA256 = (
    "0b5842935ddbf09e58321a10dce97811790fd77465246cb4eef27a8e9b9d341e"
)
EXPECTED_KICK_SEMANTIC_SHA256 = (
    "321b994e907896597ee949358ad8817c3c05a4b912b79d9c80521f40f8cd46a5"
)


def kick_semantic_sha256(features: np.ndarray) -> str:
    matrix = np.asarray(features, dtype="<f8")
    digest = hashlib.sha256()
    digest.update(kick.FEATURE_SCHEMA.encode())
    digest.update(b"\n")
    digest.update(str(len(matrix)).encode())
    digest.update(b"\n")
    digest.update(matrix.tobytes(order="C"))
    return digest.hexdigest()


def augmented_fold_features(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    kick_features: np.ndarray,
    train_mask: np.ndarray,
) -> np.ndarray:
    base_features = supervised.fold_features(
        style_scores,
        baseline_features,
        arrangement,
        embeddings,
        train_mask,
    )
    if len(kick_features) != len(base_features):
        raise ValueError("kick and base feature row counts differ")
    return np.column_stack([base_features, kick_features])


def score_partition(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    kick_features: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    features = augmented_fold_features(
        style_scores,
        baseline_features,
        arrangement,
        embeddings,
        kick_features,
        train_mask,
    )
    indices, scores, classes = supervised.ridge_score_split(
        features, truths, train_mask, test_mask
    )
    predictions, margins = supervised.predictions_and_margins(scores, classes)
    return indices, predictions, margins


def nested_cross_fitted_offers(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    kick_features: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, list[dict[str, Any]]]:
    predictions = np.full(len(truths), -1, dtype=np.int64)
    margins = np.zeros(len(truths), dtype=np.float64)
    offered = np.zeros(len(truths), dtype=bool)
    details: list[dict[str, Any]] = []

    for outer_fold in sorted(int(value) for value in np.unique(folds)):
        outer_train = folds != outer_fold
        outer_test = folds == outer_fold
        inner_predictions = np.full(len(truths), -1, dtype=np.int64)
        inner_margins = np.zeros(len(truths), dtype=np.float64)

        for inner_fold in sorted(int(value) for value in np.unique(folds[outer_train])):
            inner_test = outer_train & (folds == inner_fold)
            inner_train = outer_train & (folds != inner_fold)
            indices, fold_predictions, fold_margins = score_partition(
                style_scores,
                baseline_features,
                arrangement,
                embeddings,
                kick_features,
                truths,
                inner_train,
                inner_test,
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins

        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("nested adapter did not score every outer-training row")
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(outer_train))))
        selected = broad.choose_threshold(
            inner_margins[outer_train],
            inner_predictions[outer_train] == truths[outer_train],
            minimum_offers,
        )

        indices, fold_predictions, fold_margins = score_partition(
            style_scores,
            baseline_features,
            arrangement,
            embeddings,
            kick_features,
            truths,
            outer_train,
            outer_test,
        )
        predictions[indices] = fold_predictions
        margins[indices] = fold_margins

        if selected is None:
            details.append(
                {
                    "fold": outer_fold,
                    "threshold": None,
                    "minimum_inner_offers": minimum_offers,
                    "inner_offers": 0,
                    "inner_coverage": 0.0,
                    "inner_offered_precision": 0.0,
                }
            )
            continue

        threshold = float(selected["threshold"])
        offered[indices] = fold_margins >= threshold
        details.append(
            {
                "fold": outer_fold,
                "threshold": threshold,
                "minimum_inner_offers": minimum_offers,
                "inner_offers": int(selected["offers"]),
                "inner_coverage": broad.safe_fraction(
                    int(selected["offers"]), int(np.sum(outer_train))
                ),
                "inner_offered_precision": float(selected["offered_precision"]),
            }
        )

    if np.any(predictions < 0):
        raise ValueError("adapter did not score every outer held-out row")
    return predictions, margins, offered, details


def load_kick_artifact(path: Path, expected_rows: int) -> np.ndarray:
    artifact_sha256 = broad.sha256_file(path)
    if artifact_sha256 != EXPECTED_KICK_ARTIFACT_SHA256:
        raise ValueError("kick feature artifact SHA-256 changed")
    artifact = np.load(path, allow_pickle=False)
    if "kick_features" not in artifact or "feature_schema" not in artifact:
        raise ValueError("kick feature artifact is incomplete")
    schema = str(np.asarray(artifact["feature_schema"]).item())
    if schema != kick.FEATURE_SCHEMA:
        raise ValueError("kick feature schema changed")
    features = np.asarray(artifact["kick_features"], dtype=np.float64)
    if features.shape != (expected_rows, kick.FEATURE_COUNT):
        raise ValueError(
            f"kick feature shape {features.shape} must be "
            f"({expected_rows}, {kick.FEATURE_COUNT})"
        )
    if not np.all(np.isfinite(features)):
        raise ValueError("kick feature artifact contains non-finite values")
    if kick_semantic_sha256(features) != EXPECTED_KICK_SEMANTIC_SHA256:
        raise ValueError("kick feature semantic SHA-256 changed")
    return features


def run(args: argparse.Namespace) -> dict[str, Any]:
    hashes = {
        "manifest_sha256": broad.sha256_file(args.manifest),
        "feature_sha256": broad.sha256_file(args.features),
        "source_result_sha256": broad.sha256_file(args.source_result),
        "kick_artifact_sha256": broad.sha256_file(args.kick_features),
    }
    expected_hashes = {
        "manifest_sha256": broad.EXPECTED_MANIFEST_SHA256,
        "feature_sha256": broad.EXPECTED_FEATURE_SHA256,
        "source_result_sha256": broad.EXPECTED_SOURCE_RESULT_SHA256,
        "kick_artifact_sha256": EXPECTED_KICK_ARTIFACT_SHA256,
    }
    if hashes != expected_hashes:
        raise ValueError(f"input hashes differ from frozen values: {hashes}")
    if broad.broad_semantic_sha256() != broad.EXPECTED_BROAD_SEMANTIC_SHA256:
        raise ValueError("broad taxonomy semantic checksum changed")

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    source_result = json.loads(args.source_result.read_text(encoding="utf-8"))
    artifact = np.load(args.features, allow_pickle=False)
    supervised.validate_inputs(manifest, artifact, source_result)
    all_kick_features = load_kick_artifact(args.kick_features, len(manifest["rows"]))

    truth_indices = np.asarray(artifact["truth_indices"], dtype=np.int64)
    eligible = np.asarray(
        [broad.FINE_TO_BROAD[broad.CANONICAL[int(index)]] is not None for index in truth_indices],
        dtype=bool,
    )
    truths = np.asarray(
        [
            broad.BROAD_INDEX[broad.FINE_TO_BROAD[broad.CANONICAL[int(index)]]]
            for index in truth_indices[eligible]
        ],
        dtype=np.int64,
    )
    style_scores = broad.broad_scores(
        np.asarray(artifact["style_scores"], dtype=np.float64)[eligible]
    )
    baseline_features = supervised.baseline_broad_one_hot(
        [row["baseline_recommendation"] for row in manifest["rows"]]
    )[eligible]
    arrangement = np.asarray(artifact["arrangement"], dtype=np.float64)[eligible]
    embeddings = np.asarray(artifact["embeddings"], dtype=np.float64)[eligible]
    kick_features = all_kick_features[eligible]
    folds = np.asarray(artifact["folds"], dtype=np.int64)[eligible]

    predictions, margins, offered, thresholds = nested_cross_fitted_offers(
        style_scores,
        baseline_features,
        arrangement,
        embeddings,
        kick_features,
        truths,
        folds,
    )
    unselective = broad.metrics(
        truths, predictions, np.ones(len(truths), dtype=bool), folds
    )
    candidate = broad.metrics(truths, predictions, offered, folds)
    for detail, fold_metric in zip(thresholds, candidate["folds"], strict=True):
        detail["held_out_eligible_rows"] = fold_metric["eligible_rows"]
        detail["held_out_offers"] = fold_metric["offers"]
        detail["held_out_coverage"] = fold_metric["coverage"]
        detail["held_out_offered_precision"] = fold_metric["offered_precision"]
    gate_result = broad.gate(unselective, candidate)
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "corpus_fingerprint": broad.EXPECTED_CORPUS_FINGERPRINT,
        "inputs": hashes,
        "kick_feature_semantic_sha256": EXPECTED_KICK_SEMANTIC_SHA256,
        "broad_semantic_sha256": broad.EXPECTED_BROAD_SEMANTIC_SHA256,
        "rows": len(manifest["rows"]),
        "eligible_rows": int(np.sum(eligible)),
        "excluded_unmodeled_truth_rows": int(np.sum(~eligible)),
        "broad_targets": len(broad.BROAD_TARGETS),
        "adapter": {
            "base": "Plan 064 supervised broad adapter",
            "additional_features": kick.FEATURE_SCHEMA,
            "confidence": "top ridge score minus second ridge score",
            "threshold_calibration": "nested out-of-fold within each outer training partition",
        },
        "configurations": {
            "unselective_kick_augmented_adapter": unselective,
            "nested_selective_kick_augmented_adapter": candidate,
        },
        "fold_thresholds": thresholds,
        "gate": gate_result,
        "outcome": (
            "kick_augmented_candidate_passed_development_gate"
            if gate_result["passed"]
            else "kick_augmented_candidate_failed_development_gate"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--features", required=True, type=Path)
    parser.add_argument("--source-result", required=True, type=Path)
    parser.add_argument("--kick-features", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "eligible_rows": result["eligible_rows"],
                "configurations": {
                    name: {
                        key: value[key]
                        for key in [
                            "offers",
                            "coverage",
                            "offered_precision",
                            "accuracy",
                            "macro_recall",
                            "macro_f1",
                        ]
                    }
                    for name, value in result["configurations"].items()
                },
                "gate": result["gate"],
                "outcome": result["outcome"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
