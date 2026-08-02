#!/usr/bin/env python3
"""Evaluate the frozen Plan 064 supervised broad-genre adapter offline."""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as broad


EXPERIMENT_ID = "discogs-effnet-supervised-broad-v1"
METHOD_STATUS = "pre_registered_nested_cross_fitted_development_evaluation"
RIDGE_PENALTY = 10.0
PCA_COMPONENTS = 64


def balanced_weights(truths: np.ndarray) -> np.ndarray:
    counts = Counter(int(value) for value in truths)
    class_count = len(counts)
    return np.asarray(
        [len(truths) / (class_count * counts[int(truth)]) for truth in truths],
        dtype=np.float64,
    )


def baseline_broad_one_hot(predictions: list[str | None]) -> np.ndarray:
    values = np.zeros(
        (len(predictions), len(broad.BROAD_TARGETS)), dtype=np.float64
    )
    for row_index, prediction in enumerate(predictions):
        mapped = broad.FINE_TO_BROAD.get(prediction) if prediction is not None else None
        if mapped is not None:
            values[row_index, broad.BROAD_INDEX[mapped]] = 1.0
    return values


def impute_arrangement(
    arrangement: np.ndarray, train_mask: np.ndarray
) -> np.ndarray:
    means = np.nanmean(arrangement[train_mask], axis=0)
    means = np.where(np.isfinite(means), means, 0.0)
    return np.where(np.isfinite(arrangement), arrangement, means)


def pca_projection(
    embeddings: np.ndarray, train_mask: np.ndarray, components: int
) -> np.ndarray:
    train_mean = embeddings[train_mask].mean(axis=0)
    centered_train = embeddings[train_mask] - train_mean
    component_count = min(components, centered_train.shape[0], centered_train.shape[1])
    _, _, right = np.linalg.svd(centered_train, full_matrices=False)
    return (embeddings - train_mean) @ right[:component_count].T


def fold_features(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    train_mask: np.ndarray,
) -> np.ndarray:
    return np.column_stack(
        [
            style_scores,
            baseline_features,
            impute_arrangement(arrangement, train_mask),
            pca_projection(embeddings, train_mask, PCA_COMPONENTS),
        ]
    )


def standardize_fold(
    features: np.ndarray, train_mask: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    means = features[train_mask].mean(axis=0)
    stddev = features[train_mask].std(axis=0)
    active = np.isfinite(stddev) & (stddev > 1e-9)
    if not np.any(active):
        raise ValueError("training partition has no active feature columns")
    return (features[:, active] - means[active]) / stddev[active], active


def ridge_score_split(
    features: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, list[int]]:
    if np.any(train_mask & test_mask):
        raise ValueError("training and test partitions overlap")
    if not np.any(train_mask) or not np.any(test_mask):
        raise ValueError("training and test partitions must both contain rows")
    fold_features_scaled, _ = standardize_fold(features, train_mask)
    train_truths = truths[train_mask]
    classes = sorted(set(int(value) for value in train_truths))
    if len(classes) < 2:
        raise ValueError("training partition must contain at least two classes")
    class_index = {target: index for index, target in enumerate(classes)}
    targets = np.zeros((len(train_truths), len(classes)), dtype=np.float64)
    for row_index, truth in enumerate(train_truths):
        targets[row_index, class_index[int(truth)]] = 1.0

    x_train = np.column_stack(
        [np.ones(int(np.sum(train_mask))), fold_features_scaled[train_mask]]
    )
    x_test = np.column_stack(
        [np.ones(int(np.sum(test_mask))), fold_features_scaled[test_mask]]
    )
    root_weights = np.sqrt(balanced_weights(train_truths))
    weighted_x = x_train * root_weights[:, None]
    weighted_y = targets * root_weights[:, None]
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * RIDGE_PENALTY
    penalty[0, 0] = 0.0
    coefficients = np.linalg.solve(
        weighted_x.T @ weighted_x + penalty,
        weighted_x.T @ weighted_y,
    )
    return np.where(test_mask)[0], x_test @ coefficients, classes


def predictions_and_margins(
    scores: np.ndarray, classes: list[int]
) -> tuple[np.ndarray, np.ndarray]:
    if scores.ndim != 2 or scores.shape[1] < 2:
        raise ValueError("adapter scores must contain at least two classes")
    selected = np.argmax(scores, axis=1)
    predictions = np.asarray([classes[int(index)] for index in selected], dtype=np.int64)
    top = scores[np.arange(len(scores)), selected]
    second = np.partition(scores, -2, axis=1)[:, -2]
    return predictions, np.maximum(0.0, top - second)


def score_partition(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    features = fold_features(
        style_scores,
        baseline_features,
        arrangement,
        embeddings,
        train_mask,
    )
    indices, scores, classes = ridge_score_split(
        features, truths, train_mask, test_mask
    )
    predictions, margins = predictions_and_margins(scores, classes)
    return indices, predictions, margins


def nested_cross_fitted_offers(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
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
                truths,
                inner_train,
                inner_test,
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins

        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("nested adapter did not score every outer-training row")
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(outer_train))))
        inner_correct = inner_predictions[outer_train] == truths[outer_train]
        selected = broad.choose_threshold(
            inner_margins[outer_train], inner_correct, minimum_offers
        )

        indices, fold_predictions, fold_margins = score_partition(
            style_scores,
            baseline_features,
            arrangement,
            embeddings,
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


def validate_inputs(
    manifest: dict[str, Any], artifact: Any, source_result: dict[str, Any]
) -> None:
    if manifest["corpus_fingerprint"] != broad.EXPECTED_CORPUS_FINGERPRINT:
        raise ValueError("manifest corpus fingerprint changed")
    if source_result["corpus_fingerprint"] != broad.EXPECTED_CORPUS_FINGERPRINT:
        raise ValueError("source result corpus fingerprint changed")
    rows = manifest["rows"]
    required = ["style_scores", "embeddings", "arrangement", "truth_indices", "folds"]
    missing = [name for name in required if name not in artifact]
    if missing:
        raise ValueError(f"feature artifact is missing arrays: {missing}")
    if any(len(artifact[name]) != len(rows) for name in required):
        raise ValueError("manifest and feature row counts differ")
    for index, row in enumerate(rows):
        if broad.CANONICAL[int(artifact["truth_indices"][index])] != row["truth"]:
            raise ValueError(f"truth alignment differs at row {index}")
        if int(artifact["folds"][index]) != int(row["fold"]):
            raise ValueError(f"fold alignment differs at row {index}")


def run(args: argparse.Namespace) -> dict[str, Any]:
    hashes = {
        "manifest_sha256": broad.sha256_file(args.manifest),
        "feature_sha256": broad.sha256_file(args.features),
        "source_result_sha256": broad.sha256_file(args.source_result),
    }
    expected_hashes = {
        "manifest_sha256": broad.EXPECTED_MANIFEST_SHA256,
        "feature_sha256": broad.EXPECTED_FEATURE_SHA256,
        "source_result_sha256": broad.EXPECTED_SOURCE_RESULT_SHA256,
    }
    if hashes != expected_hashes:
        raise ValueError(f"input hashes differ from frozen values: {hashes}")
    if broad.broad_semantic_sha256() != broad.EXPECTED_BROAD_SEMANTIC_SHA256:
        raise ValueError("broad taxonomy semantic checksum changed")

    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    source_result = json.loads(args.source_result.read_text(encoding="utf-8"))
    artifact = np.load(args.features, allow_pickle=False)
    validate_inputs(manifest, artifact, source_result)

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
    baseline_features = baseline_broad_one_hot(
        [row["baseline_recommendation"] for row in manifest["rows"]]
    )[eligible]
    arrangement = np.asarray(artifact["arrangement"], dtype=np.float64)[eligible]
    embeddings = np.asarray(artifact["embeddings"], dtype=np.float64)[eligible]
    folds = np.asarray(artifact["folds"], dtype=np.int64)[eligible]

    predictions, margins, offered, thresholds = nested_cross_fitted_offers(
        style_scores,
        baseline_features,
        arrangement,
        embeddings,
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
        "broad_semantic_sha256": broad.EXPECTED_BROAD_SEMANTIC_SHA256,
        "rows": len(manifest["rows"]),
        "eligible_rows": int(np.sum(eligible)),
        "excluded_unmodeled_truth_rows": int(np.sum(~eligible)),
        "broad_targets": len(broad.BROAD_TARGETS),
        "adapter": {
            "estimator": "class-balanced one-versus-rest ridge least squares",
            "ridge_penalty": RIDGE_PENALTY,
            "unpenalized_intercept": True,
            "features": [
                "maximum fine-style score per broad target",
                "mapped v0.33 broad recommendation one-hot",
                "four arrangement features",
                "training-partition EffNet embedding PCA64",
            ],
            "confidence": "top ridge score minus second ridge score",
            "threshold_calibration": "nested out-of-fold within each outer training partition",
        },
        "configurations": {
            "unselective_supervised_broad_adapter": unselective,
            "nested_selective_supervised_broad_adapter": candidate,
        },
        "fold_thresholds": thresholds,
        "gate": gate_result,
        "outcome": (
            "supervised_broad_candidate_passed_development_gate"
            if gate_result["passed"]
            else "supervised_broad_candidate_failed_development_gate"
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
