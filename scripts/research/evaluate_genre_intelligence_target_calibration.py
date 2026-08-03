#!/usr/bin/env python3
"""Evaluate Plan 069 target-aware CLAP confidence calibration."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_candidate as candidate_a
import evaluate_genre_intelligence_clap as candidate_c
import evaluate_genre_intelligence_openl3 as representation_evaluation
import extract_genre_intelligence_features as base_features
import prepare_genre_intelligence_candidate as preparation


EXPERIMENT_ID = "genre-intelligence-v1-target-aware-calibration"
METHOD_STATUS = "pre_registered_nested_target_aware_development_evaluation"
MINIMUM_TARGET_CALIBRATION_OFFERS = 8
MINIMUM_CALIBRATION_PRECISION = 0.90
MINIMUM_DEPLOYMENT_TARGETS = 4


def choose_target_thresholds(
    predictions: np.ndarray,
    margins: np.ndarray,
    truths: np.ndarray,
) -> dict[int, dict[str, float | int] | None]:
    result: dict[int, dict[str, float | int] | None] = {}
    for target_index in range(len(candidate_a.RELEASE_SCOPE)):
        predicted = predictions == target_index
        best: tuple[int, float, float] | None = None
        for threshold in sorted(float(value) for value in np.unique(margins[predicted])):
            offered = predicted & (margins >= threshold)
            offers = int(np.sum(offered))
            if offers < MINIMUM_TARGET_CALIBRATION_OFFERS:
                continue
            precision = candidate_a.safe_fraction(
                int(np.sum(offered & (truths == target_index))), offers
            )
            if precision < MINIMUM_CALIBRATION_PRECISION - 1e-12:
                continue
            candidate = (offers, precision, threshold)
            if best is None or candidate > best:
                best = candidate
        result[target_index] = (
            {
                "threshold": best[2],
                "offers": best[0],
                "offered_precision": best[1],
            }
            if best is not None
            else None
        )
    return result


def apply_target_thresholds(
    predictions: np.ndarray,
    margins: np.ndarray,
    thresholds: dict[int, dict[str, float | int] | None],
) -> np.ndarray:
    offered = np.zeros(len(predictions), dtype=bool)
    for target_index, selected in thresholds.items():
        if selected is None:
            continue
        offered |= (predictions == target_index) & (
            margins >= float(selected["threshold"])
        )
    return offered


def serialized_thresholds(
    thresholds: dict[int, dict[str, float | int] | None],
) -> dict[str, dict[str, float | int] | None]:
    return {
        candidate_a.RELEASE_SCOPE[index]: value
        for index, value in sorted(thresholds.items())
    }


def nested_predictions(
    base: np.ndarray,
    representation: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
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
            indices, fold_predictions, fold_margins = (
                representation_evaluation.score_partition(
                    base,
                    representation,
                    truths,
                    inner_train,
                    inner_test,
                )
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins
        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("inner target-aware evaluation did not score every row")
        selected = choose_target_thresholds(
            inner_predictions[outer_train],
            inner_margins[outer_train],
            truths[outer_train],
        )
        indices, fold_predictions, fold_margins = (
            representation_evaluation.score_partition(
                base, representation, truths, outer_train, outer_test
            )
        )
        predictions[indices] = fold_predictions
        margins[indices] = fold_margins
        offered[indices] = apply_target_thresholds(
            fold_predictions, fold_margins, selected
        )
        details.append(
            {
                "fold": outer_fold,
                "thresholds": serialized_thresholds(selected),
                "calibrated_targets": sum(
                    value is not None for value in selected.values()
                ),
            }
        )
    if np.any(predictions < 0):
        raise ValueError("outer target-aware evaluation did not score every row")
    return predictions, margins, offered, details


def extended_gate(
    metrics: dict[str, Any],
    paired: dict[str, Any],
    calibrated_targets: int,
) -> dict[str, Any]:
    result = candidate_a.gate(metrics, paired)
    result["at_least_four_calibrated_targets"] = (
        calibrated_targets >= MINIMUM_DEPLOYMENT_TARGETS
    )
    result["calibrated_targets"] = calibrated_targets
    result["passed"] = bool(
        result["passed"] and result["at_least_four_calibrated_targets"]
    )
    return result


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = candidate_c.load_config(args.config)
    observed_hashes = {
        "development_manifest_sha256": candidate_a.sha256_file(
            args.development_manifest
        ),
        "feature_manifest_sha256": candidate_a.sha256_file(args.feature_manifest),
        "base_feature_artifact_sha256": candidate_a.sha256_file(args.base_features),
        "base_feature_summary_sha256": candidate_a.sha256_file(
            args.base_feature_summary
        ),
        "representation_manifest_sha256": candidate_a.sha256_file(
            args.representation_manifest
        ),
        "clap_artifact_sha256": candidate_a.sha256_file(args.clap_features),
        "clap_summary_sha256": candidate_a.sha256_file(args.clap_summary),
    }
    if any(config[key] != value for key, value in observed_hashes.items()):
        raise ValueError("target-aware input hashes differ from candidate-C config")

    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    feature_manifest = json.loads(args.feature_manifest.read_text(encoding="utf-8"))
    representation_manifest = json.loads(
        args.representation_manifest.read_text(encoding="utf-8")
    )
    if [row["row_id"] for row in feature_manifest["rows"]] != [
        row["row_id"] for row in representation_manifest["rows"]
    ]:
        raise ValueError("base and representation manifest identity order differs")
    truths, folds = candidate_a.validate_manifests(development, feature_manifest)
    base = np.load(args.base_features, allow_pickle=False)
    if base.shape != (len(truths), len(base_features.FEATURE_NAMES)):
        raise ValueError("target-aware base feature shape differs")
    clap = candidate_c.load_representation(args.clap_features, len(truths))

    predictions, margins, nested_offered, fold_thresholds = nested_predictions(
        base, clap, truths, folds
    )
    baseline_slice = base[:, -len(base_features.BASELINE_FEATURES) :]
    baseline_selected = np.argmax(baseline_slice, axis=1)
    baseline_predictions = np.where(
        baseline_selected < len(candidate_a.RELEASE_SCOPE), baseline_selected, -1
    ).astype(np.int64)
    nested = candidate_a.metrics(truths, predictions, nested_offered, folds)
    nested_paired = candidate_a.paired_baseline(
        truths, predictions, nested_offered, baseline_predictions
    )
    nested_gate = candidate_a.gate(nested, nested_paired)

    deployment_thresholds = choose_target_thresholds(predictions, margins, truths)
    deployment_offered = apply_target_thresholds(
        predictions, margins, deployment_thresholds
    )
    deployment = candidate_a.metrics(truths, predictions, deployment_offered, folds)
    deployment_paired = candidate_a.paired_baseline(
        truths, predictions, deployment_offered, baseline_predictions
    )
    deployment_target_count = sum(
        value is not None for value in deployment_thresholds.values()
    )
    deployment_gate = extended_gate(
        deployment, deployment_paired, deployment_target_count
    )
    accepted_rows = int(config["accepted_rows"])
    passed = bool(nested_gate["passed"] and deployment_gate["passed"])
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "source_candidate_config_sha256": candidate_a.sha256_file(args.config),
        "inputs": observed_hashes,
        "accepted_rows": accepted_rows,
        "model_ready_rows": len(truths),
        "release_scope": candidate_a.RELEASE_SCOPE,
        "calibration": {
            "minimum_target_offers": MINIMUM_TARGET_CALIBRATION_OFFERS,
            "minimum_calibration_precision": MINIMUM_CALIBRATION_PRECISION,
            "selection": "maximum offers, higher precision, higher threshold",
            "pooled_fallback": False,
        },
        "unselective": candidate_a.metrics(
            truths, predictions, np.ones(len(truths), dtype=bool), folds
        ),
        "nested": {
            "metrics": nested,
            "paired_v033": nested_paired,
            "whole_accepted_coverage": candidate_a.safe_fraction(
                nested["offers"], accepted_rows
            ),
            "fold_thresholds": fold_thresholds,
            "gate": nested_gate,
        },
        "deployment": {
            "thresholds": serialized_thresholds(deployment_thresholds),
            "metrics": deployment,
            "paired_v033": deployment_paired,
            "whole_accepted_coverage": candidate_a.safe_fraction(
                deployment["offers"], accepted_rows
            ),
            "gate": deployment_gate,
        },
        "passed": passed,
        "outcome": (
            "candidate_ready_for_sealed_holdout"
            if passed
            else "target_aware_calibration_bounded_negative"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--feature-manifest", required=True, type=Path)
    parser.add_argument("--base-features", required=True, type=Path)
    parser.add_argument("--base-feature-summary", required=True, type=Path)
    parser.add_argument("--representation-manifest", required=True, type=Path)
    parser.add_argument("--clap-features", required=True, type=Path)
    parser.add_argument("--clap-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    candidate_a.atomic_write(args.output, result)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "nested": {
                    "offers": result["nested"]["metrics"]["offers"],
                    "coverage": result["nested"]["metrics"]["coverage"],
                    "offered_precision": result["nested"]["metrics"][
                        "offered_precision"
                    ],
                    "gate": result["nested"]["gate"],
                },
                "deployment": {
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
