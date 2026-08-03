#!/usr/bin/env python3
"""Evaluate frozen Plan 068 candidate A on artist-isolated development folds."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as thresholding
import extract_genre_intelligence_features as features
import prepare_genre_intelligence_candidate as preparation


EXPERIMENT_ID = "genre-intelligence-v1-candidate-a"
METHOD_STATUS = "pre_registered_artist_isolated_development_evaluation"
RIDGE_PENALTY = 10.0
TARGET_PRECISION = 0.90
MINIMUM_SCOPE_COVERAGE = 0.65
MINIMUM_FOLD_PRECISION = 0.85
MINIMUM_TARGET_PRECISION = 0.80
MINIMUM_TARGET_OFFERS = 8
MINIMUM_BASELINE_IMPROVEMENT = 0.05
MINIMUM_GLOBAL_OFFERS = 60
RELEASE_SCOPE = preparation.RELEASE_SCOPE


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    required = {
        "schema_version",
        "experiment_id",
        "development_manifest_sha256",
        "feature_manifest_sha256",
        "feature_artifact_sha256",
        "feature_summary_sha256",
        "source_corpus_sha256",
        "accepted_rows",
        "model_ready_rows",
        "release_scope",
    }
    if set(config) != required:
        raise ValueError("candidate config fields differ from the frozen schema")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("candidate config identity differs")
    if config["release_scope"] != RELEASE_SCOPE:
        raise ValueError("candidate config release scope differs")
    if config["source_corpus_sha256"] != preparation.EXPECTED_CORPUS_SHA256:
        raise ValueError("candidate config source corpus differs")
    return config


def impute_and_standardize(
    matrix: np.ndarray, train_mask: np.ndarray
) -> tuple[np.ndarray, np.ndarray]:
    if matrix.ndim != 2 or not np.any(train_mask):
        raise ValueError("feature matrix and training mask must be non-empty")
    means = np.zeros(matrix.shape[1], dtype=np.float64)
    for column in range(matrix.shape[1]):
        observed = matrix[train_mask, column]
        observed = observed[np.isfinite(observed)]
        means[column] = float(np.mean(observed)) if len(observed) else 0.0
    filled = np.where(np.isfinite(matrix), matrix, means)
    stddev = filled[train_mask].std(axis=0)
    active = np.isfinite(stddev) & (stddev > 1e-9)
    if not np.any(active):
        raise ValueError("training partition has no active feature columns")
    scaled = (filled[:, active] - means[active]) / stddev[active]
    return scaled, active


def balanced_weights(truths: np.ndarray) -> np.ndarray:
    counts = Counter(int(value) for value in truths)
    return np.asarray(
        [len(truths) / (len(counts) * counts[int(value)]) for value in truths],
        dtype=np.float64,
    )


def score_partition(
    matrix: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    if np.any(train_mask & test_mask) or not np.any(test_mask):
        raise ValueError("training and test partitions must be disjoint and non-empty")
    scaled, _ = impute_and_standardize(matrix, train_mask)
    train_truths = truths[train_mask]
    classes = sorted(set(int(value) for value in train_truths))
    if classes != list(range(len(RELEASE_SCOPE))):
        raise ValueError("training partition does not contain every release target")
    targets = np.zeros((len(train_truths), len(classes)), dtype=np.float64)
    for row_index, truth in enumerate(train_truths):
        targets[row_index, int(truth)] = 1.0
    x_train = np.column_stack(
        [np.ones(int(np.sum(train_mask))), scaled[train_mask]]
    )
    x_test = np.column_stack([np.ones(int(np.sum(test_mask))), scaled[test_mask]])
    root_weights = np.sqrt(balanced_weights(train_truths))
    weighted_x = x_train * root_weights[:, None]
    weighted_y = targets * root_weights[:, None]
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * RIDGE_PENALTY
    penalty[0, 0] = 0.0
    coefficients = np.linalg.solve(
        weighted_x.T @ weighted_x + penalty,
        weighted_x.T @ weighted_y,
    )
    scores = x_test @ coefficients
    selected = np.argmax(scores, axis=1)
    top = scores[np.arange(len(scores)), selected]
    second = np.partition(scores, -2, axis=1)[:, -2]
    return (
        np.where(test_mask)[0],
        selected.astype(np.int64),
        np.maximum(0.0, top - second),
    )


def nested_predictions(
    matrix: np.ndarray, truths: np.ndarray, folds: np.ndarray
) -> tuple[np.ndarray, np.ndarray, np.ndarray, list[dict[str, Any]]]:
    predictions = np.full(len(truths), -1, dtype=np.int64)
    margins = np.zeros(len(truths), dtype=np.float64)
    offered = np.zeros(len(truths), dtype=bool)
    details = []
    for outer_fold in range(preparation.FOLD_COUNT):
        outer_train = folds != outer_fold
        outer_test = folds == outer_fold
        inner_predictions = np.full(len(truths), -1, dtype=np.int64)
        inner_margins = np.zeros(len(truths), dtype=np.float64)
        for inner_fold in range(preparation.FOLD_COUNT):
            if inner_fold == outer_fold:
                continue
            inner_test = outer_train & (folds == inner_fold)
            inner_train = outer_train & (folds != inner_fold)
            indices, fold_predictions, fold_margins = score_partition(
                matrix, truths, inner_train, inner_test
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins
        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("inner evaluation did not score every outer-training row")
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(outer_train))))
        selected = thresholding.choose_threshold(
            inner_margins[outer_train],
            inner_predictions[outer_train] == truths[outer_train],
            minimum_offers,
        )
        indices, fold_predictions, fold_margins = score_partition(
            matrix, truths, outer_train, outer_test
        )
        predictions[indices] = fold_predictions
        margins[indices] = fold_margins
        threshold = float(selected["threshold"]) if selected is not None else None
        if threshold is not None:
            offered[indices] = fold_margins >= threshold
        details.append(
            {
                "fold": outer_fold,
                "threshold": threshold,
                "minimum_inner_offers": minimum_offers,
                "inner_offers": int(selected["offers"]) if selected else 0,
                "inner_offered_precision": (
                    float(selected["offered_precision"]) if selected else 0.0
                ),
            }
        )
    if np.any(predictions < 0):
        raise ValueError("outer evaluation did not score every development row")
    return predictions, margins, offered, details


def safe_fraction(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 0.0


def metrics(
    truths: np.ndarray,
    predictions: np.ndarray,
    offered: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    correct = truths == predictions
    per_target = {}
    for target_index, target in enumerate(RELEASE_SCOPE):
        truth_mask = truths == target_index
        predicted_mask = offered & (predictions == target_index)
        support = int(np.sum(truth_mask))
        offers = int(np.sum(predicted_mask))
        correct_offers = int(np.sum(truth_mask & predicted_mask))
        per_target[target] = {
            "support": support,
            "offers": offers,
            "correct_offers": correct_offers,
            "offered_precision": safe_fraction(correct_offers, offers),
            "recall": safe_fraction(correct_offers, support),
        }
    fold_metrics = []
    for fold in range(preparation.FOLD_COUNT):
        mask = folds == fold
        offers = int(np.sum(mask & offered))
        correct_offers = int(np.sum(mask & offered & correct))
        fold_metrics.append(
            {
                "fold": fold,
                "eligible_rows": int(np.sum(mask)),
                "offers": offers,
                "correct_offers": correct_offers,
                "coverage": safe_fraction(offers, int(np.sum(mask))),
                "offered_precision": safe_fraction(correct_offers, offers),
            }
        )
    offers = int(np.sum(offered))
    correct_offers = int(np.sum(offered & correct))
    recalls = [row["recall"] for row in per_target.values()]
    return {
        "eligible_rows": len(truths),
        "offers": offers,
        "correct_offers": correct_offers,
        "abstentions": len(truths) - offers,
        "coverage": safe_fraction(offers, len(truths)),
        "offered_precision": safe_fraction(correct_offers, offers),
        "accuracy": safe_fraction(correct_offers, len(truths)),
        "macro_recall": float(np.mean(recalls)),
        "per_target": per_target,
        "folds": fold_metrics,
    }


def paired_baseline(
    truths: np.ndarray,
    candidate_predictions: np.ndarray,
    candidate_offered: np.ndarray,
    baseline_predictions: np.ndarray,
) -> dict[str, Any]:
    paired = candidate_offered & (baseline_predictions >= 0)
    paired_rows = int(np.sum(paired))
    candidate_precision = safe_fraction(
        int(np.sum(paired & (candidate_predictions == truths))), paired_rows
    )
    baseline_precision = safe_fraction(
        int(np.sum(paired & (baseline_predictions == truths))), paired_rows
    )
    return {
        "paired_offers": paired_rows,
        "candidate_offered_precision": candidate_precision,
        "v033_offered_precision": baseline_precision,
        "precision_improvement": candidate_precision - baseline_precision,
    }


def gate(candidate: dict[str, Any], paired: dict[str, Any]) -> dict[str, Any]:
    target_failures = [
        target
        for target, row in candidate["per_target"].items()
        if row["offers"] >= MINIMUM_TARGET_OFFERS
        and row["offered_precision"] < MINIMUM_TARGET_PRECISION - 1e-12
    ]
    checks = {
        "offered_precision_at_least_0_90": candidate["offered_precision"]
        >= TARGET_PRECISION - 1e-12,
        "scope_coverage_at_least_0_65": candidate["coverage"]
        >= MINIMUM_SCOPE_COVERAGE - 1e-12,
        "every_fold_precision_at_least_0_85": all(
            row["offers"] > 0
            and row["offered_precision"] >= MINIMUM_FOLD_PRECISION - 1e-12
            for row in candidate["folds"]
        ),
        "target_precision_at_least_0_80": not target_failures,
        "v033_paired_precision_improvement_at_least_0_05": paired[
            "precision_improvement"
        ]
        >= MINIMUM_BASELINE_IMPROVEMENT - 1e-12,
        "target_failures": target_failures,
    }
    checks["passed"] = all(
        value for key, value in checks.items() if key != "target_failures"
    )
    return checks


def validate_manifests(
    development: dict[str, Any], feature_manifest: dict[str, Any]
) -> tuple[np.ndarray, np.ndarray]:
    development_rows = development.get("rows")
    feature_rows = feature_manifest.get("rows")
    if not isinstance(development_rows, list) or not isinstance(feature_rows, list):
        raise ValueError("candidate manifests have no rows")
    if len(development_rows) != len(feature_rows):
        raise ValueError("candidate manifest row counts differ")
    for index, (truth_row, feature_row) in enumerate(
        zip(development_rows, feature_rows, strict=True)
    ):
        if truth_row["row_id"] != feature_row["row_id"]:
            raise ValueError(f"candidate manifest identity differs at row {index}")
    truths = np.asarray(
        [RELEASE_SCOPE.index(row["canonical_parent_genre"]) for row in development_rows],
        dtype=np.int64,
    )
    folds = np.asarray([int(row["fold"]) for row in development_rows], dtype=np.int64)
    artists: dict[str, int] = {}
    releases: dict[str, int] = {}
    for row in development_rows:
        fold = int(row["fold"])
        for key, values in (
            (str(row["artist_group"]), artists),
            (str(row["release_group"]), releases),
        ):
            if key in values and values[key] != fold:
                raise ValueError("artist or release group crosses candidate folds")
            values[key] = fold
    return truths, folds


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(args.config)
    observed_hashes = {
        "development_manifest_sha256": sha256_file(args.development_manifest),
        "feature_manifest_sha256": sha256_file(args.feature_manifest),
        "feature_artifact_sha256": sha256_file(args.features),
        "feature_summary_sha256": sha256_file(args.feature_summary),
    }
    if any(config[key] != value for key, value in observed_hashes.items()):
        raise ValueError("candidate input hashes differ from the frozen config")
    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    feature_manifest = json.loads(args.feature_manifest.read_text(encoding="utf-8"))
    summary = json.loads(args.feature_summary.read_text(encoding="utf-8"))
    if summary.get("feature_names") != features.FEATURE_NAMES:
        raise ValueError("candidate feature names differ")
    truths, folds = validate_manifests(development, feature_manifest)
    matrix = np.load(args.features, allow_pickle=False)
    if matrix.shape != (len(truths), len(features.FEATURE_NAMES)):
        raise ValueError("candidate feature artifact shape differs")

    predictions, margins, nested_offered, thresholds = nested_predictions(
        matrix, truths, folds
    )
    baseline_slice = matrix[:, -len(features.BASELINE_FEATURES) :]
    baseline_selected = np.argmax(baseline_slice, axis=1)
    baseline_predictions = np.where(
        baseline_selected < len(RELEASE_SCOPE), baseline_selected, -1
    ).astype(np.int64)

    unselective = metrics(
        truths, predictions, np.ones(len(truths), dtype=bool), folds
    )
    nested = metrics(truths, predictions, nested_offered, folds)
    nested_paired = paired_baseline(
        truths, predictions, nested_offered, baseline_predictions
    )
    nested_gate = gate(nested, nested_paired)

    deployment_selected = thresholding.choose_threshold(
        margins, predictions == truths, MINIMUM_GLOBAL_OFFERS
    )
    deployment_threshold = (
        float(deployment_selected["threshold"])
        if deployment_selected is not None
        else None
    )
    deployment_offered = (
        margins >= deployment_threshold
        if deployment_threshold is not None
        else np.zeros(len(truths), dtype=bool)
    )
    deployment = metrics(truths, predictions, deployment_offered, folds)
    deployment_paired = paired_baseline(
        truths, predictions, deployment_offered, baseline_predictions
    )
    deployment_gate = gate(deployment, deployment_paired)
    baseline_metrics = metrics(
        truths, baseline_predictions, baseline_predictions >= 0, folds
    )
    accepted_rows = int(config["accepted_rows"])
    passed = bool(nested_gate["passed"] and deployment_gate["passed"])
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": sha256_file(args.config),
        "inputs": observed_hashes,
        "source_corpus_sha256": config["source_corpus_sha256"],
        "accepted_rows": accepted_rows,
        "model_ready_rows": len(truths),
        "release_scope": RELEASE_SCOPE,
        "fold_count": preparation.FOLD_COUNT,
        "adapter": {
            "estimator": "class-balanced one-versus-rest ridge least squares",
            "ridge_penalty": RIDGE_PENALTY,
            "unpenalized_intercept": True,
            "feature_schema": features.FEATURE_SCHEMA,
            "feature_count": len(features.FEATURE_NAMES),
            "confidence": "top ridge score minus second ridge score",
            "threshold_calibration": "nested inner OOF plus one global outer-OOF deployment threshold",
            "artist_and_release_isolation": True,
        },
        "v033_baseline": baseline_metrics,
        "unselective": unselective,
        "nested": {
            "metrics": nested,
            "paired_v033": nested_paired,
            "whole_accepted_coverage": safe_fraction(nested["offers"], accepted_rows),
            "fold_thresholds": thresholds,
            "gate": nested_gate,
        },
        "deployment": {
            "minimum_calibration_offers": MINIMUM_GLOBAL_OFFERS,
            "threshold": deployment_threshold,
            "metrics": deployment,
            "paired_v033": deployment_paired,
            "whole_accepted_coverage": safe_fraction(
                deployment["offers"], accepted_rows
            ),
            "gate": deployment_gate,
        },
        "passed": passed,
        "outcome": (
            "candidate_ready_for_sealed_holdout"
            if passed
            else "candidate_a_bounded_negative"
        ),
    }


def atomic_write(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--feature-manifest", required=True, type=Path)
    parser.add_argument("--features", required=True, type=Path)
    parser.add_argument("--feature-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    atomic_write(args.output, result)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "v033": {
                    "offers": result["v033_baseline"]["offers"],
                    "coverage": result["v033_baseline"]["coverage"],
                    "offered_precision": result["v033_baseline"][
                        "offered_precision"
                    ],
                },
                "nested": {
                    "offers": result["nested"]["metrics"]["offers"],
                    "coverage": result["nested"]["metrics"]["coverage"],
                    "offered_precision": result["nested"]["metrics"][
                        "offered_precision"
                    ],
                    "gate": result["nested"]["gate"],
                },
                "deployment": {
                    "threshold": result["deployment"]["threshold"],
                    "offers": result["deployment"]["metrics"]["offers"],
                    "coverage": result["deployment"]["metrics"]["coverage"],
                    "offered_precision": result["deployment"]["metrics"][
                        "offered_precision"
                    ],
                    "gate": result["deployment"]["gate"],
                },
                "outcome": result["outcome"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
