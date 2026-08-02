#!/usr/bin/env python3
"""Evaluate the frozen Plan 058 supervised EffNet adapters offline."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_genre_eval as base


RIDGE_PENALTY = 10.0
PCA_COMPONENTS = 64


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def balanced_weights(truths: list[str]) -> np.ndarray:
    counts = Counter(truths)
    class_count = len(counts)
    return np.asarray(
        [len(truths) / (class_count * counts[truth]) for truth in truths],
        dtype=np.float64,
    )


def baseline_one_hot(predictions: list[str | None]) -> np.ndarray:
    values = np.zeros((len(predictions), len(base.CANONICAL)), dtype=np.float64)
    for row_index, prediction in enumerate(predictions):
        if prediction is not None:
            values[row_index, base.CANONICAL_INDEX[prediction]] = 1.0
    return values


def impute_arrangement(
    arrangement: np.ndarray, train_mask: np.ndarray
) -> np.ndarray:
    train = arrangement[train_mask]
    means = np.nanmean(train, axis=0)
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


def standardize_fold(
    features: np.ndarray, train_mask: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    means = features[train_mask].mean(axis=0)
    stddev = features[train_mask].std(axis=0)
    active = np.isfinite(stddev) & (stddev > 1e-9)
    if not np.any(active):
        raise ValueError("training fold has no active feature columns")
    return (features[:, active] - means[active]) / stddev[active], active


def ridge_predict_split(
    features: np.ndarray,
    truths: list[str],
    train_mask: np.ndarray,
) -> tuple[np.ndarray, list[str]]:
    test_indices = np.where(~train_mask)[0]
    fold_features, _ = standardize_fold(features, train_mask)
    x_train = np.column_stack(
        [np.ones(int(np.sum(train_mask))), fold_features[train_mask]]
    )
    x_test = np.column_stack([np.ones(len(test_indices)), fold_features[test_indices]])
    train_truths = [truth for index, truth in enumerate(truths) if train_mask[index]]
    classes = sorted(set(train_truths), key=base.CANONICAL_INDEX.__getitem__)
    class_index = {genre: index for index, genre in enumerate(classes)}
    targets = np.zeros((len(train_truths), len(classes)), dtype=np.float64)
    for row_index, truth in enumerate(train_truths):
        targets[row_index, class_index[truth]] = 1.0
    weights = balanced_weights(train_truths)
    root_weights = np.sqrt(weights)
    weighted_x = x_train * root_weights[:, None]
    weighted_y = targets * root_weights[:, None]
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * RIDGE_PENALTY
    penalty[0, 0] = 0.0
    coefficients = np.linalg.solve(
        weighted_x.T @ weighted_x + penalty,
        weighted_x.T @ weighted_y,
    )
    scores = x_test @ coefficients
    predictions = [classes[int(selected)] for selected in np.argmax(scores, axis=1)]
    return test_indices, predictions


def ridge_predictions(
    features: np.ndarray,
    truths: list[str],
    folds: np.ndarray,
) -> list[str]:
    predictions = [""] * len(truths)
    for held_out_fold in sorted(set(int(value) for value in folds)):
        test_indices, fold_predictions = ridge_predict_split(
            features, truths, folds != held_out_fold
        )
        for row_index, prediction in zip(test_indices, fold_predictions, strict=True):
            predictions[int(row_index)] = prediction
    if any(not prediction for prediction in predictions):
        raise ValueError("adapter did not produce every held-out prediction")
    return predictions


def evaluate_configuration(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    embeddings: np.ndarray,
    truths: list[str],
    folds: np.ndarray,
    include_embedding: bool,
) -> dict[str, Any]:
    predictions = [""] * len(truths)
    for held_out_fold in sorted(set(int(value) for value in folds)):
        train_mask = folds != held_out_fold
        arrangement_fold = impute_arrangement(arrangement, train_mask)
        parts = [style_scores, baseline_features, arrangement_fold]
        if include_embedding:
            parts.append(pca_projection(embeddings, train_mask, PCA_COMPONENTS))
        features = np.column_stack(parts)
        test_indices, fold_predictions = ridge_predict_split(features, truths, train_mask)
        for row_index, prediction in zip(test_indices, fold_predictions, strict=True):
            predictions[int(row_index)] = prediction
    return base.aggregate_metrics(truths, predictions, folds)


def run(args: argparse.Namespace) -> dict[str, Any]:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    stage_a = json.loads(args.stage_a.read_text(encoding="utf-8"))
    stage_b = json.loads(args.stage_b.read_text(encoding="utf-8"))
    if manifest["corpus_fingerprint"] != stage_a["corpus_fingerprint"]:
        raise ValueError("manifest and Stage A corpus fingerprints differ")
    if manifest["corpus_fingerprint"] != stage_b["corpus_fingerprint"]:
        raise ValueError("manifest and Stage B corpus fingerprints differ")
    feature_sha = sha256_file(args.feature_artifact)
    if feature_sha != stage_b["feature_artifact"]["sha256"]:
        raise ValueError("feature artifact SHA-256 differs from Stage B result")

    rows = manifest["rows"]
    artifact = np.load(args.feature_artifact, allow_pickle=False)
    style_scores = artifact["style_scores"].astype(np.float64)
    embeddings = artifact["embeddings"].astype(np.float64)
    arrangement = artifact["arrangement"].astype(np.float64)
    folds = artifact["folds"].astype(np.int64)
    truths = [row["truth"] for row in rows]
    if len(rows) != len(folds):
        raise ValueError("manifest and feature row counts differ")
    if not np.array_equal(folds, [row["fold"] for row in rows]):
        raise ValueError("manifest and feature fold assignments differ")
    expected_truth_indices = np.asarray(
        [base.CANONICAL_INDEX[truth] for truth in truths], dtype=np.int16
    )
    if not np.array_equal(expected_truth_indices, artifact["truth_indices"]):
        raise ValueError("manifest and feature truth ordering differ")
    baseline_predictions = [row["baseline_recommendation"] for row in rows]
    invalid = sorted(
        {
            prediction
            for prediction in baseline_predictions
            if prediction is not None and prediction not in base.CANONICAL
        }
    )
    if invalid:
        raise ValueError(f"unknown baseline predictions: {invalid}")
    baseline_features = baseline_one_hot(baseline_predictions)

    configurations = {}
    for name, include_embedding in [
        ("style_baseline_arrangement", False),
        ("style_baseline_arrangement_embedding_pca64", True),
    ]:
        metrics = evaluate_configuration(
            style_scores,
            baseline_features,
            arrangement,
            embeddings,
            truths,
            folds,
            include_embedding,
        )
        configurations[name] = {
            "metrics": metrics,
            "gate": base.gate(metrics, stage_a["baseline"]),
        }

    passing = [
        name for name, result in configurations.items() if result["gate"]["passed"]
    ]
    passing.sort(
        key=lambda name: (
            -configurations[name]["metrics"]["macro_f1"],
            -configurations[name]["metrics"]["exact_accuracy"],
            name == "style_baseline_arrangement_embedding_pca64",
        )
    )
    selected = passing[0] if passing else None
    return {
        "experiment_id": "discogs-effnet-supervised-adapter-v2-expanded-corpus",
        "method_status": "pre_registered_expanded_corpus_development_evaluation",
        "rows": len(rows),
        "fold_count": manifest["fold_count"],
        "adapter": {
            "estimator": "class-balanced one-versus-rest ridge least squares",
            "ridge_penalty": RIDGE_PENALTY,
            "unpenalized_intercept": True,
            "pca_components": PCA_COMPONENTS,
            "pca_whitened": False,
        },
        "inputs": {
            "feature_artifact_sha256": feature_sha,
            "stage_a_result_sha256": sha256_file(args.stage_a),
            "stage_b_result_sha256": sha256_file(args.stage_b),
        },
        "baseline": {
            key: stage_a["baseline"][key]
            for key in [
                "support",
                "exact_accuracy",
                "macro_recall",
                "macro_f1",
                "same_family_accuracy",
                "folds",
            ]
        },
        "configurations": configurations,
        "selected_configuration": selected,
        "outcome": "development_candidate_for_new_holdout"
        if selected
        else "bounded_negative",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--stage-a", required=True, type=Path)
    parser.add_argument("--stage-b", required=True, type=Path)
    parser.add_argument("--feature-artifact", required=True, type=Path)
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
                "outcome": result["outcome"],
                "selected_configuration": result["selected_configuration"],
                "configurations": {
                    name: {
                        "exact_accuracy": value["metrics"]["exact_accuracy"],
                        "macro_recall": value["metrics"]["macro_recall"],
                        "macro_f1": value["metrics"]["macro_f1"],
                        "same_family_accuracy": value["metrics"][
                            "same_family_accuracy"
                        ],
                        "gate": value["gate"],
                    }
                    for name, value in result["configurations"].items()
                },
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
