#!/usr/bin/env python3
"""Evaluate the two preregistered Plan 071 open-set candidates."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as thresholding
import discogs_effnet_supervised_broad_eval as supervised
import extract_genre_intelligence_features as base_features
import prepare_genre_intelligence_open_set as preparation


EXPERIMENT_ID = "genre-intelligence-v1-open-set"
METHOD_STATUS = "preregistered_nested_open_set_development_evaluation"
RIDGE_PENALTY = 10.0
PCA_COMPONENTS = 64
CLAP_DIMENSION = 512
OTHER_INDEX = len(preparation.OUTPUT_PARENTS)
TARGET_PRECISION = 0.90
MINIMUM_GLOBAL_OFFERS = 180
MINIMUM_COVERAGE = 0.25
MAXIMUM_NON_TARGET_FALSE_OFFER_RATE = 0.10
MINIMUM_FOLD_OFFERS = 20
MINIMUM_FOLD_PRECISION = 0.85
MINIMUM_TARGET_OFFERS = 8
MINIMUM_TARGET_PRECISION = 0.80
MINIMUM_DEPLOYABLE_PARENTS = 4
MINIMUM_BASELINE_IMPROVEMENT = 0.05
EXPECTED_INPUT_SHA256 = {
    "development_manifest": (
        "dfd11addd96a2e7b5727700594b337aaacfc19bdd97db408e1ba0955f80853bd"
    ),
    "feature_manifest": (
        "6bf80b80f060649877a90a5d6dfa8188c9549eaa0986f1667d611e115689b682"
    ),
    "base_features": (
        "f3e615a89f5b3770e170f0b7ddafd29e87052fcbf6a44c333ba0f9aced331365"
    ),
    "base_summary": (
        "87fbdb446ca18dd251f4596cbd5879d999b33d0e3463b257b1fe25ee9f043c16"
    ),
    "clap_features": (
        "72fbace49fdcb2885d4dce78fac3f1212baac1742718d903c6203314f4e4ffc9"
    ),
    "clap_summary": (
        "764da176061e9087d3d5d5498b17cb24fd1897aaa4ee163b5901234bca2de41b"
    ),
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_fraction(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 0.0


def augmented_matrix(
    base: np.ndarray, clap: np.ndarray, train_mask: np.ndarray
) -> np.ndarray:
    projection = supervised.pca_projection(clap, train_mask, PCA_COMPONENTS)
    if projection.shape[0] != base.shape[0]:
        raise ValueError("CLAP and base feature row counts differ")
    return np.column_stack([base, projection])


def weighted_ridge(
    matrix: np.ndarray,
    targets: np.ndarray,
    weights: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> np.ndarray:
    if (
        np.any(train_mask & test_mask)
        or not np.any(train_mask)
        or not np.any(test_mask)
    ):
        raise ValueError("training and test partitions must be disjoint and non-empty")
    scaled, _ = base_features_for_fold(matrix, train_mask)
    x_train = np.column_stack([np.ones(int(np.sum(train_mask))), scaled[train_mask]])
    x_test = np.column_stack([np.ones(int(np.sum(test_mask))), scaled[test_mask]])
    root_weights = np.sqrt(weights)
    weighted_x = x_train * root_weights[:, None]
    weighted_y = targets * root_weights[:, None]
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * RIDGE_PENALTY
    penalty[0, 0] = 0.0
    coefficients = np.linalg.solve(
        weighted_x.T @ weighted_x + penalty,
        weighted_x.T @ weighted_y,
    )
    return x_test @ coefficients


def base_features_for_fold(
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
    return (filled[:, active] - means[active]) / stddev[active], active


def multiclass_weights(truths: np.ndarray) -> np.ndarray:
    counts = Counter(int(value) for value in truths)
    return np.asarray(
        [len(truths) / (len(counts) * counts[int(value)]) for value in truths],
        dtype=np.float64,
    )


def binary_weights(truths: np.ndarray) -> np.ndarray:
    positives = int(np.sum(truths))
    negatives = len(truths) - positives
    if positives == 0 or negatives == 0:
        raise ValueError("binary training partition needs both classes")
    return np.where(
        truths,
        len(truths) / (2.0 * positives),
        len(truths) / (2.0 * negatives),
    )


def score_o1_partition(
    base: np.ndarray,
    clap: np.ndarray,
    truth_targets: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    matrix = augmented_matrix(base, clap, train_mask)
    train_truths = np.where(
        truth_targets[train_mask] >= 0, truth_targets[train_mask], OTHER_INDEX
    )
    if sorted(set(int(value) for value in train_truths)) != list(
        range(OTHER_INDEX + 1)
    ):
        raise ValueError("O1 training partition does not contain every class")
    targets = np.zeros((len(train_truths), OTHER_INDEX + 1), dtype=np.float64)
    targets[np.arange(len(train_truths)), train_truths] = 1.0
    scores = weighted_ridge(
        matrix,
        targets,
        multiclass_weights(train_truths),
        train_mask,
        test_mask,
    )
    selected = np.argmax(scores, axis=1).astype(np.int64)
    top = scores[np.arange(len(scores)), selected]
    second = np.partition(scores, -2, axis=1)[:, -2]
    return np.where(test_mask)[0], selected, np.maximum(0.0, top - second)


def score_o2_partition(
    base: np.ndarray,
    clap: np.ndarray,
    truth_targets: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray]:
    matrix = augmented_matrix(base, clap, train_mask)
    scores = np.zeros((int(np.sum(test_mask)), OTHER_INDEX), dtype=np.float64)
    for target in range(OTHER_INDEX):
        train_truths = truth_targets[train_mask] == target
        targets = train_truths.astype(np.float64)[:, None]
        scores[:, target] = weighted_ridge(
            matrix,
            targets,
            binary_weights(train_truths),
            train_mask,
            test_mask,
        )[:, 0]
    return np.where(test_mask)[0], scores


def threshold_detail(
    values: np.ndarray, correct: np.ndarray, eligible: np.ndarray | None = None
) -> dict[str, Any]:
    if eligible is None:
        eligible = np.ones(len(values), dtype=bool)
    selected = thresholding.choose_threshold(
        values[eligible], correct[eligible], MINIMUM_TARGET_OFFERS
    )
    return {
        "threshold": float(selected["threshold"]) if selected else None,
        "calibration_rows": int(np.sum(eligible)),
        "calibration_offers": int(selected["offers"]) if selected else 0,
        "calibration_precision": (
            float(selected["offered_precision"]) if selected else 0.0
        ),
    }


def calibrate_o1(
    predictions: np.ndarray, margins: np.ndarray, truths: np.ndarray, mask: np.ndarray
) -> list[dict[str, Any]]:
    return [
        {
            "parent": parent,
            **threshold_detail(
                margins[mask],
                truths[mask] == target,
                predictions[mask] == target,
            ),
        }
        for target, parent in enumerate(preparation.OUTPUT_PARENTS)
    ]


def apply_o1(
    predictions: np.ndarray,
    margins: np.ndarray,
    thresholds: list[dict[str, Any]],
    mask: np.ndarray,
) -> np.ndarray:
    offered = np.zeros(len(predictions), dtype=bool)
    for target, detail in enumerate(thresholds):
        threshold = detail["threshold"]
        if threshold is not None:
            offered |= mask & (predictions == target) & (margins >= threshold)
    return offered


def calibrate_o2(
    scores: np.ndarray, truths: np.ndarray, mask: np.ndarray
) -> list[dict[str, Any]]:
    return [
        {
            "parent": parent,
            **threshold_detail(scores[mask, target], truths[mask] == target),
        }
        for target, parent in enumerate(preparation.OUTPUT_PARENTS)
    ]


def apply_o2(
    scores: np.ndarray,
    thresholds: list[dict[str, Any]],
    mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, dict[str, int]]:
    qualified = np.zeros(scores.shape, dtype=bool)
    for target, detail in enumerate(thresholds):
        threshold = detail["threshold"]
        if threshold is not None:
            qualified[:, target] = mask & (scores[:, target] >= threshold)
    counts = np.sum(qualified, axis=1)
    offered = mask & (counts == 1)
    predictions = np.full(len(scores), -1, dtype=np.int64)
    predictions[offered] = np.argmax(qualified[offered], axis=1)
    scoped_counts = counts[mask]
    return predictions, offered, {
        "zero_qualified": int(np.sum(scoped_counts == 0)),
        "one_qualified": int(np.sum(scoped_counts == 1)),
        "multi_qualified": int(np.sum(scoped_counts > 1)),
    }


def nested_o1(
    base: np.ndarray, clap: np.ndarray, truths: np.ndarray, folds: np.ndarray
) -> dict[str, Any]:
    predictions = np.full(len(truths), -1, dtype=np.int64)
    margins = np.zeros(len(truths), dtype=np.float64)
    offered = np.zeros(len(truths), dtype=bool)
    fold_details = []
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
            indices, values, confidence = score_o1_partition(
                base, clap, truths, inner_train, inner_test
            )
            inner_predictions[indices] = values
            inner_margins[indices] = confidence
        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("O1 inner evaluation did not score every training row")
        thresholds = calibrate_o1(
            inner_predictions, inner_margins, truths, outer_train
        )
        indices, values, confidence = score_o1_partition(
            base, clap, truths, outer_train, outer_test
        )
        predictions[indices] = values
        margins[indices] = confidence
        offered |= apply_o1(predictions, margins, thresholds, outer_test)
        fold_details.append({"fold": outer_fold, "thresholds": thresholds})
    if np.any(predictions < 0):
        raise ValueError("O1 outer evaluation did not score every development row")
    return {
        "predictions": predictions,
        "margins": margins,
        "offered": offered,
        "fold_thresholds": fold_details,
    }


def nested_o2(
    base: np.ndarray, clap: np.ndarray, truths: np.ndarray, folds: np.ndarray
) -> dict[str, Any]:
    scores = np.full((len(truths), OTHER_INDEX), np.nan, dtype=np.float64)
    predictions = np.full(len(truths), -1, dtype=np.int64)
    offered = np.zeros(len(truths), dtype=bool)
    fold_details = []
    total_counts = Counter()
    for outer_fold in range(preparation.FOLD_COUNT):
        outer_train = folds != outer_fold
        outer_test = folds == outer_fold
        inner_scores = np.full(scores.shape, np.nan, dtype=np.float64)
        for inner_fold in range(preparation.FOLD_COUNT):
            if inner_fold == outer_fold:
                continue
            inner_test = outer_train & (folds == inner_fold)
            inner_train = outer_train & (folds != inner_fold)
            indices, values = score_o2_partition(
                base, clap, truths, inner_train, inner_test
            )
            inner_scores[indices] = values
        if not np.all(np.isfinite(inner_scores[outer_train])):
            raise ValueError("O2 inner evaluation did not score every training row")
        thresholds = calibrate_o2(inner_scores, truths, outer_train)
        indices, values = score_o2_partition(
            base, clap, truths, outer_train, outer_test
        )
        scores[indices] = values
        fold_predictions, fold_offered, counts = apply_o2(
            scores, thresholds, outer_test
        )
        predictions[outer_test] = fold_predictions[outer_test]
        offered |= fold_offered
        total_counts.update(counts)
        fold_details.append(
            {"fold": outer_fold, "thresholds": thresholds, "qualified": counts}
        )
    if not np.all(np.isfinite(scores)):
        raise ValueError("O2 outer evaluation did not score every development row")
    return {
        "scores": scores,
        "predictions": predictions,
        "offered": offered,
        "fold_thresholds": fold_details,
        "qualification_counts": dict(total_counts),
    }


def metrics(
    truth_names: list[str],
    truth_targets: np.ndarray,
    predictions: np.ndarray,
    offered: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    correct = truth_targets == predictions
    per_target = {}
    for target, parent in enumerate(preparation.OUTPUT_PARENTS):
        truth_mask = truth_targets == target
        predicted_mask = offered & (predictions == target)
        support = int(np.sum(truth_mask))
        offers = int(np.sum(predicted_mask))
        correct_offers = int(np.sum(truth_mask & predicted_mask))
        per_target[parent] = {
            "support": support,
            "offers": offers,
            "correct_offers": correct_offers,
            "offered_precision": safe_fraction(correct_offers, offers),
            "recall": safe_fraction(correct_offers, support),
        }
    fold_metrics = []
    for fold in range(preparation.FOLD_COUNT):
        mask = folds == fold
        fold_offers = int(np.sum(mask & offered))
        fold_correct = int(np.sum(mask & offered & correct))
        fold_metrics.append(
            {
                "fold": fold,
                "rows": int(np.sum(mask)),
                "offers": fold_offers,
                "correct_offers": fold_correct,
                "coverage": safe_fraction(fold_offers, int(np.sum(mask))),
                "offered_precision": safe_fraction(fold_correct, fold_offers),
            }
        )
    non_target = truth_targets < 0
    false_offers = int(np.sum(non_target & offered))
    offers = int(np.sum(offered))
    correct_offers = int(np.sum(offered & correct))
    return {
        "rows": len(truth_names),
        "offers": offers,
        "correct_offers": correct_offers,
        "abstentions": len(truth_names) - offers,
        "coverage": safe_fraction(offers, len(truth_names)),
        "offered_precision": safe_fraction(correct_offers, offers),
        "accuracy": safe_fraction(correct_offers, len(truth_names)),
        "per_target": per_target,
        "folds": fold_metrics,
        "non_target": {
            "support": int(np.sum(non_target)),
            "false_offers": false_offers,
            "false_offer_rate": safe_fraction(false_offers, int(np.sum(non_target))),
            "truth_parent_counts": dict(
                sorted(
                    Counter(
                        name
                        for name, value in zip(
                            truth_names, non_target, strict=True
                        )
                        if value
                    ).items()
                )
            ),
        },
    }


def paired_baseline(
    truth_targets: np.ndarray,
    predictions: np.ndarray,
    offered: np.ndarray,
    baseline_predictions: np.ndarray,
) -> dict[str, Any]:
    paired = offered & (baseline_predictions >= 0)
    rows = int(np.sum(paired))
    candidate_precision = safe_fraction(
        int(np.sum(paired & (predictions == truth_targets))), rows
    )
    baseline_precision = safe_fraction(
        int(np.sum(paired & (baseline_predictions == truth_targets))), rows
    )
    return {
        "paired_offers": rows,
        "candidate_offered_precision": candidate_precision,
        "v033_offered_precision": baseline_precision,
        "precision_improvement": candidate_precision - baseline_precision,
    }


def gate(candidate: dict[str, Any], paired: dict[str, Any]) -> dict[str, Any]:
    target_failures = [
        parent
        for parent, row in candidate["per_target"].items()
        if row["offers"] >= MINIMUM_TARGET_OFFERS
        and row["offered_precision"] < MINIMUM_TARGET_PRECISION - 1e-12
    ]
    supported_targets = [
        parent
        for parent, row in candidate["per_target"].items()
        if row["offers"] >= MINIMUM_TARGET_OFFERS
    ]
    checks = {
        "offers_at_least_180": candidate["offers"] >= MINIMUM_GLOBAL_OFFERS,
        "coverage_at_least_0_25": candidate["coverage"] >= MINIMUM_COVERAGE - 1e-12,
        "offered_precision_at_least_0_90": candidate["offered_precision"]
        >= TARGET_PRECISION - 1e-12,
        "non_target_false_offer_rate_at_most_0_10": candidate["non_target"][
            "false_offer_rate"
        ]
        <= MAXIMUM_NON_TARGET_FALSE_OFFER_RATE + 1e-12,
        "every_fold_at_least_20_offers_and_0_85_precision": all(
            row["offers"] >= MINIMUM_FOLD_OFFERS
            and row["offered_precision"] >= MINIMUM_FOLD_PRECISION - 1e-12
            for row in candidate["folds"]
        ),
        "targets_with_support_at_least_0_80_precision": not target_failures,
        "at_least_four_targets_with_eight_offers": len(supported_targets)
        >= MINIMUM_DEPLOYABLE_PARENTS,
        "paired_v033_precision_improvement_at_least_0_05": paired[
            "precision_improvement"
        ]
        >= MINIMUM_BASELINE_IMPROVEMENT - 1e-12,
        "target_failures": target_failures,
        "targets_with_eight_offers": supported_targets,
    }
    checks["passed"] = all(
        value
        for key, value in checks.items()
        if key not in {"target_failures", "targets_with_eight_offers"}
    )
    return checks


def validate_inputs(
    args: argparse.Namespace,
) -> tuple[list[str], np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    observed = {
        "development_manifest": sha256_file(args.development_manifest),
        "feature_manifest": sha256_file(args.feature_manifest),
        "base_features": sha256_file(args.base_features),
        "base_summary": sha256_file(args.base_summary),
        "clap_features": sha256_file(args.clap_features),
        "clap_summary": sha256_file(args.clap_summary),
    }
    if observed != EXPECTED_INPUT_SHA256:
        raise ValueError("open-set evaluator input hashes differ from frozen inputs")
    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    feature_manifest = json.loads(args.feature_manifest.read_text(encoding="utf-8"))
    development_rows = development.get("rows")
    feature_rows = feature_manifest.get("rows")
    if not isinstance(development_rows, list) or not isinstance(feature_rows, list):
        raise ValueError("open-set manifests have no rows")
    if (
        len(development_rows) != preparation.EXPECTED_ACCEPTED_ROWS
        or len(feature_rows) != len(development_rows)
    ):
        raise ValueError("open-set manifest row counts differ")
    for index, (truth_row, feature_row) in enumerate(
        zip(development_rows, feature_rows, strict=True)
    ):
        if truth_row["row_id"] != feature_row["row_id"]:
            raise ValueError(f"open-set manifest identity differs at row {index}")
        if set(feature_row) != {"row_id", "file_path"}:
            raise ValueError("feature manifest row contains truth or grouping fields")
    truth_names = [str(row["canonical_parent_genre"]) for row in development_rows]
    truth_targets = np.asarray(
        [
            preparation.OUTPUT_PARENTS.index(value)
            if value in preparation.OUTPUT_PARENTS
            else -1
            for value in truth_names
        ],
        dtype=np.int64,
    )
    folds = np.asarray([int(row["fold"]) for row in development_rows], dtype=np.int64)
    artists: dict[str, int] = {}
    releases: dict[str, int] = {}
    for row in development_rows:
        fold = int(row["fold"])
        for group, assignments in (
            (str(row["artist_group"]), artists),
            (str(row["release_group"]), releases),
        ):
            if group in assignments and assignments[group] != fold:
                raise ValueError("artist or release group crosses open-set folds")
            assignments[group] = fold
    base = np.load(args.base_features, allow_pickle=False)
    if base.shape != (
        len(development_rows),
        len(base_features.FEATURE_NAMES),
    ):
        raise ValueError("base feature artifact shape differs")
    clap_artifact = np.load(args.clap_features, allow_pickle=False)
    if list(clap_artifact.files) != ["embeddings"]:
        raise ValueError("CLAP artifact arrays differ")
    clap = np.asarray(clap_artifact["embeddings"], dtype=np.float64)
    if clap.shape != (len(development_rows), CLAP_DIMENSION) or not np.all(
        np.isfinite(clap)
    ):
        raise ValueError("CLAP feature artifact shape or values differ")
    return truth_names, truth_targets, folds, base, clap


def baseline_predictions(base: np.ndarray) -> np.ndarray:
    values = base[:, -len(base_features.BASELINE_FEATURES) :]
    selected = np.argmax(values, axis=1)
    return np.where(selected < OTHER_INDEX, selected, -1).astype(np.int64)


def deployment_o1(
    nested: dict[str, Any],
    truth_names: list[str],
    truths: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    mask = np.ones(len(truths), dtype=bool)
    thresholds = calibrate_o1(
        nested["predictions"], nested["margins"], truths, mask
    )
    offered = apply_o1(nested["predictions"], nested["margins"], thresholds, mask)
    result = metrics(truth_names, truths, nested["predictions"], offered, folds)
    deployable = [
        parent
        for parent, row in result["per_target"].items()
        if row["offers"] >= MINIMUM_TARGET_OFFERS
        and row["offered_precision"] >= TARGET_PRECISION - 1e-12
    ]
    return {
        "thresholds": thresholds,
        "metrics": result,
        "deployable_parents": deployable,
        "passed": len(deployable) >= MINIMUM_DEPLOYABLE_PARENTS,
    }


def deployment_o2(
    nested: dict[str, Any],
    truth_names: list[str],
    truths: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    mask = np.ones(len(truths), dtype=bool)
    thresholds = calibrate_o2(nested["scores"], truths, mask)
    predictions, offered, counts = apply_o2(nested["scores"], thresholds, mask)
    result = metrics(truth_names, truths, predictions, offered, folds)
    deployable = [
        parent
        for parent, row in result["per_target"].items()
        if row["offers"] >= MINIMUM_TARGET_OFFERS
        and row["offered_precision"] >= TARGET_PRECISION - 1e-12
    ]
    return {
        "thresholds": thresholds,
        "qualification_counts": counts,
        "metrics": result,
        "deployable_parents": deployable,
        "passed": len(deployable) >= MINIMUM_DEPLOYABLE_PARENTS,
    }


def candidate_result(
    name: str,
    nested: dict[str, Any],
    truth_names: list[str],
    truths: np.ndarray,
    folds: np.ndarray,
    baseline: np.ndarray,
) -> dict[str, Any]:
    result_metrics = metrics(
        truth_names, truths, nested["predictions"], nested["offered"], folds
    )
    paired = paired_baseline(
        truths, nested["predictions"], nested["offered"], baseline
    )
    development_gate = gate(result_metrics, paired)
    deployment = (
        deployment_o1(nested, truth_names, truths, folds)
        if name == "O1"
        else deployment_o2(nested, truth_names, truths, folds)
    )
    payload = {
        "candidate": name,
        "formulation": (
            "pooled Other eight-class ridge with per-parent margin thresholds"
            if name == "O1"
            else "seven binary ridge models with collision abstention"
        ),
        "nested": {
            "metrics": result_metrics,
            "paired_v033": paired,
            "fold_thresholds": nested["fold_thresholds"],
            "gate": development_gate,
        },
        "deployment_calibration": deployment,
        "passed": bool(development_gate["passed"] and deployment["passed"]),
    }
    if name == "O2":
        payload["nested"]["qualification_counts"] = nested["qualification_counts"]
    return payload


def run(args: argparse.Namespace) -> dict[str, Any]:
    truth_names, truths, folds, base, clap = validate_inputs(args)
    baseline = baseline_predictions(base)
    baseline_metrics = metrics(
        truth_names, truths, baseline, baseline >= 0, folds
    )
    o1 = candidate_result(
        "O1", nested_o1(base, clap, truths, folds), truth_names, truths, folds, baseline
    )
    o2 = candidate_result(
        "O2", nested_o2(base, clap, truths, folds), truth_names, truths, folds, baseline
    )
    passing = [candidate for candidate in (o1, o2) if candidate["passed"]]
    passing.sort(
        key=lambda candidate: (
            candidate["nested"]["metrics"]["offered_precision"],
            candidate["nested"]["metrics"]["coverage"],
            candidate["candidate"] == "O1",
        ),
        reverse=True,
    )
    selected = passing[0]["candidate"] if passing else None
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "evaluator_source_sha256": sha256_file(Path(__file__)),
        "inputs": EXPECTED_INPUT_SHA256,
        "source_corpus_sha256": preparation.EXPECTED_CORPUS_SHA256,
        "accepted_corpus_fingerprint": preparation.EXPECTED_ACCEPTED_FINGERPRINT,
        "rows": len(truths),
        "target_rows": int(np.sum(truths >= 0)),
        "non_target_rows": int(np.sum(truths < 0)),
        "output_parents": preparation.OUTPUT_PARENTS,
        "fold_count": preparation.FOLD_COUNT,
        "adapter": {
            "base_feature_schema": base_features.FEATURE_SCHEMA,
            "base_feature_count": len(base_features.FEATURE_NAMES),
            "clap_dimension": CLAP_DIMENSION,
            "clap_projection": "training-partition-only PCA64",
            "ridge_penalty": RIDGE_PENALTY,
            "unpenalized_intercept": True,
            "artist_and_release_isolation": True,
        },
        "v033_baseline": baseline_metrics,
        "candidates": {"O1": o1, "O2": o2},
        "selected_candidate": selected,
        "outcome": (
            "candidate_ready_for_fresh_independent_holdout"
            if selected is not None
            else "bounded_development_negative_fresh_holdout_untouched"
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
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--feature-manifest", required=True, type=Path)
    parser.add_argument("--base-features", required=True, type=Path)
    parser.add_argument("--base-summary", required=True, type=Path)
    parser.add_argument("--clap-features", required=True, type=Path)
    parser.add_argument("--clap-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    atomic_write(args.output, result)
    print(
        json.dumps(
            {
                "output_sha256": sha256_file(args.output),
                "selected_candidate": result["selected_candidate"],
                "outcome": result["outcome"],
                "candidates": {
                    name: {
                        "offers": candidate["nested"]["metrics"]["offers"],
                        "coverage": candidate["nested"]["metrics"]["coverage"],
                        "offered_precision": candidate["nested"]["metrics"][
                            "offered_precision"
                        ],
                        "non_target_false_offer_rate": candidate["nested"][
                            "metrics"
                        ]["non_target"]["false_offer_rate"],
                        "passed": candidate["passed"],
                    }
                    for name, candidate in result["candidates"].items()
                },
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
