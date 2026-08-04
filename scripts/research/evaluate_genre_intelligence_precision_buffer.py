#!/usr/bin/env python3
"""Evaluate the sole preregistered Plan 072 precision-buffered candidate."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_open_set as plan071


EXPERIMENT_ID = "genre-intelligence-v1-precision-buffer"
METHOD_STATUS = "preregistered_precision_buffered_open_set_evaluation"
EXPECTED_PLAN071_SOURCE_SHA256 = (
    "8f5d97c80fcd08e49a8062556cceec7cd48f5452ff265359446ae9ff479452d2"
)
CALIBRATION_PRECISION = 0.95


def choose_threshold(
    values: np.ndarray, correct: np.ndarray
) -> dict[str, float | int] | None:
    best: tuple[int, float, float] | None = None
    for threshold in sorted(float(value) for value in np.unique(values)):
        offered = values >= threshold
        offers = int(np.sum(offered))
        if offers < plan071.MINIMUM_TARGET_OFFERS:
            continue
        precision = plan071.safe_fraction(int(np.sum(offered & correct)), offers)
        if precision < CALIBRATION_PRECISION - 1e-12:
            continue
        candidate = (offers, precision, threshold)
        if best is None or candidate > best:
            best = candidate
    if best is None:
        return None
    offers, precision, threshold = best
    return {"threshold": threshold, "offers": offers, "offered_precision": precision}


def calibrate(
    scores: np.ndarray, truths: np.ndarray, mask: np.ndarray
) -> list[dict[str, Any]]:
    details = []
    for target, parent in enumerate(plan071.preparation.OUTPUT_PARENTS):
        selected = choose_threshold(scores[mask, target], truths[mask] == target)
        details.append(
            {
                "parent": parent,
                "threshold": float(selected["threshold"]) if selected else None,
                "calibration_rows": int(np.sum(mask)),
                "calibration_offers": int(selected["offers"]) if selected else 0,
                "calibration_precision": (
                    float(selected["offered_precision"]) if selected else 0.0
                ),
            }
        )
    return details


def nested_predictions(
    base: np.ndarray, clap: np.ndarray, truths: np.ndarray, folds: np.ndarray
) -> dict[str, Any]:
    scores = np.full(
        (len(truths), len(plan071.preparation.OUTPUT_PARENTS)),
        np.nan,
        dtype=np.float64,
    )
    predictions = np.full(len(truths), -1, dtype=np.int64)
    offered = np.zeros(len(truths), dtype=bool)
    fold_details = []
    total_counts = Counter()
    for outer_fold in range(plan071.preparation.FOLD_COUNT):
        outer_train = folds != outer_fold
        outer_test = folds == outer_fold
        inner_scores = np.full(scores.shape, np.nan, dtype=np.float64)
        for inner_fold in range(plan071.preparation.FOLD_COUNT):
            if inner_fold == outer_fold:
                continue
            inner_test = outer_train & (folds == inner_fold)
            inner_train = outer_train & (folds != inner_fold)
            indices, values = plan071.score_o2_partition(
                base, clap, truths, inner_train, inner_test
            )
            inner_scores[indices] = values
        if not np.all(np.isfinite(inner_scores[outer_train])):
            raise ValueError("O3 inner evaluation did not score every training row")
        thresholds = calibrate(inner_scores, truths, outer_train)
        indices, values = plan071.score_o2_partition(
            base, clap, truths, outer_train, outer_test
        )
        scores[indices] = values
        fold_predictions, fold_offered, counts = plan071.apply_o2(
            scores, thresholds, outer_test
        )
        predictions[outer_test] = fold_predictions[outer_test]
        offered |= fold_offered
        total_counts.update(counts)
        fold_details.append(
            {"fold": outer_fold, "thresholds": thresholds, "qualified": counts}
        )
    if not np.all(np.isfinite(scores)):
        raise ValueError("O3 outer evaluation did not score every development row")
    return {
        "scores": scores,
        "predictions": predictions,
        "offered": offered,
        "fold_thresholds": fold_details,
        "qualification_counts": dict(total_counts),
    }


def deployment_calibration(
    nested: dict[str, Any],
    truth_names: list[str],
    truths: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    mask = np.ones(len(truths), dtype=bool)
    thresholds = calibrate(nested["scores"], truths, mask)
    predictions, offered, counts = plan071.apply_o2(
        nested["scores"], thresholds, mask
    )
    metrics = plan071.metrics(truth_names, truths, predictions, offered, folds)
    deployable = [
        parent
        for parent, row in metrics["per_target"].items()
        if row["offers"] >= plan071.MINIMUM_TARGET_OFFERS
        and row["offered_precision"] >= plan071.TARGET_PRECISION - 1e-12
    ]
    return {
        "thresholds": thresholds,
        "qualification_counts": counts,
        "metrics": metrics,
        "deployable_parents": deployable,
        "passed": len(deployable) >= plan071.MINIMUM_DEPLOYABLE_PARENTS,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    if plan071.sha256_file(Path(plan071.__file__)) != EXPECTED_PLAN071_SOURCE_SHA256:
        raise ValueError("Plan 071 evaluator source differs from the frozen adapter")
    truth_names, truths, folds, base, clap = plan071.validate_inputs(args)
    baseline = plan071.baseline_predictions(base)
    nested = nested_predictions(base, clap, truths, folds)
    metrics = plan071.metrics(
        truth_names, truths, nested["predictions"], nested["offered"], folds
    )
    paired = plan071.paired_baseline(
        truths, nested["predictions"], nested["offered"], baseline
    )
    gate = plan071.gate(metrics, paired)
    deployment = deployment_calibration(nested, truth_names, truths, folds)
    passed = bool(gate["passed"] and deployment["passed"])
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "evaluator_source_sha256": plan071.sha256_file(Path(__file__)),
        "plan071_evaluator_source_sha256": EXPECTED_PLAN071_SOURCE_SHA256,
        "inputs": plan071.EXPECTED_INPUT_SHA256,
        "source_corpus_sha256": plan071.preparation.EXPECTED_CORPUS_SHA256,
        "accepted_corpus_fingerprint": (
            plan071.preparation.EXPECTED_ACCEPTED_FINGERPRINT
        ),
        "rows": len(truths),
        "target_rows": int(np.sum(truths >= 0)),
        "non_target_rows": int(np.sum(truths < 0)),
        "output_parents": plan071.preparation.OUTPUT_PARENTS,
        "fold_count": plan071.preparation.FOLD_COUNT,
        "adapter": {
            "formulation": "seven binary ridge models with collision abstention",
            "base_feature_schema": plan071.base_features.FEATURE_SCHEMA,
            "clap_projection": "training-partition-only PCA64",
            "ridge_penalty": plan071.RIDGE_PENALTY,
            "inner_calibration_precision": CALIBRATION_PRECISION,
            "minimum_parent_calibration_offers": (
                plan071.MINIMUM_TARGET_OFFERS
            ),
        },
        "v033_baseline": plan071.metrics(
            truth_names, truths, baseline, baseline >= 0, folds
        ),
        "candidate": {
            "name": "O3",
            "nested": {
                "metrics": metrics,
                "paired_v033": paired,
                "fold_thresholds": nested["fold_thresholds"],
                "qualification_counts": nested["qualification_counts"],
                "gate": gate,
            },
            "deployment_calibration": deployment,
            "passed": passed,
        },
        "passed": passed,
        "outcome": (
            "candidate_ready_for_fresh_independent_holdout"
            if passed
            else "bounded_development_negative_fresh_holdout_untouched"
        ),
    }


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
    plan071.atomic_write(args.output, result)
    metrics = result["candidate"]["nested"]["metrics"]
    print(
        json.dumps(
            {
                "output_sha256": plan071.sha256_file(args.output),
                "offers": metrics["offers"],
                "coverage": metrics["coverage"],
                "offered_precision": metrics["offered_precision"],
                "non_target_false_offer_rate": metrics["non_target"][
                    "false_offer_rate"
                ],
                "gate": result["candidate"]["nested"]["gate"],
                "deployment_passed": result["candidate"][
                    "deployment_calibration"
                ]["passed"],
                "outcome": result["outcome"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
