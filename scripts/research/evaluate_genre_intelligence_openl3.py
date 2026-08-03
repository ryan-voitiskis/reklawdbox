#!/usr/bin/env python3
"""Evaluate frozen Plan 068 candidate B with OpenL3 PCA64."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as thresholding
import discogs_effnet_supervised_broad_eval as supervised
import evaluate_genre_intelligence_candidate as candidate_a
import extract_genre_intelligence_features as base_features
import prepare_genre_intelligence_candidate as preparation


EXPERIMENT_ID = "genre-intelligence-v1-candidate-b-openl3"
METHOD_STATUS = "pre_registered_artist_isolated_openl3_development_evaluation"
PCA_COMPONENTS = 64
OPENL3_DIMENSION = 512


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    required = {
        "schema_version",
        "experiment_id",
        "development_manifest_sha256",
        "feature_manifest_sha256",
        "base_feature_artifact_sha256",
        "base_feature_summary_sha256",
        "representation_manifest_sha256",
        "openl3_artifact_sha256",
        "openl3_summary_sha256",
        "source_corpus_sha256",
        "accepted_rows",
        "model_ready_rows",
        "release_scope",
    }
    if set(config) != required:
        raise ValueError("candidate-B config fields differ from the frozen schema")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("candidate-B config identity differs")
    if config["release_scope"] != candidate_a.RELEASE_SCOPE:
        raise ValueError("candidate-B release scope differs")
    if config["source_corpus_sha256"] != preparation.EXPECTED_CORPUS_SHA256:
        raise ValueError("candidate-B source corpus differs")
    return config


def augmented_matrix(
    base: np.ndarray, representation: np.ndarray, train_mask: np.ndarray
) -> np.ndarray:
    projection = supervised.pca_projection(
        representation, train_mask, PCA_COMPONENTS
    )
    if projection.shape[0] != base.shape[0]:
        raise ValueError("OpenL3 and base feature row counts differ")
    return np.column_stack([base, projection])


def score_partition(
    base: np.ndarray,
    representation: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    return candidate_a.score_partition(
        augmented_matrix(base, representation, train_mask),
        truths,
        train_mask,
        test_mask,
    )


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
            indices, fold_predictions, fold_margins = score_partition(
                base,
                representation,
                truths,
                inner_train,
                inner_test,
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins
        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("inner OpenL3 evaluation did not score every row")
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(outer_train))))
        selected = thresholding.choose_threshold(
            inner_margins[outer_train],
            inner_predictions[outer_train] == truths[outer_train],
            minimum_offers,
        )
        indices, fold_predictions, fold_margins = score_partition(
            base, representation, truths, outer_train, outer_test
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
        raise ValueError("outer OpenL3 evaluation did not score every row")
    return predictions, margins, offered, details


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(args.config)
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
        "openl3_artifact_sha256": candidate_a.sha256_file(args.openl3_features),
        "openl3_summary_sha256": candidate_a.sha256_file(args.openl3_summary),
    }
    if any(config[key] != value for key, value in observed_hashes.items()):
        raise ValueError("candidate-B input hashes differ from the frozen config")

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
        raise ValueError("candidate-B base feature shape differs")
    openl3_artifact = np.load(args.openl3_features, allow_pickle=False)
    if list(openl3_artifact.files) != ["embeddings"]:
        raise ValueError("OpenL3 artifact arrays differ")
    openl3 = np.asarray(openl3_artifact["embeddings"], dtype=np.float64)
    if openl3.shape != (len(truths), OPENL3_DIMENSION) or not np.all(
        np.isfinite(openl3)
    ):
        raise ValueError("OpenL3 feature shape or values differ")

    predictions, margins, nested_offered, thresholds = nested_predictions(
        base, openl3, truths, folds
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

    deployment_selected = thresholding.choose_threshold(
        margins, predictions == truths, candidate_a.MINIMUM_GLOBAL_OFFERS
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
    deployment = candidate_a.metrics(truths, predictions, deployment_offered, folds)
    deployment_paired = candidate_a.paired_baseline(
        truths, predictions, deployment_offered, baseline_predictions
    )
    deployment_gate = candidate_a.gate(deployment, deployment_paired)
    accepted_rows = int(config["accepted_rows"])
    passed = bool(nested_gate["passed"] and deployment_gate["passed"])
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": candidate_a.sha256_file(args.config),
        "inputs": observed_hashes,
        "source_corpus_sha256": config["source_corpus_sha256"],
        "accepted_rows": accepted_rows,
        "model_ready_rows": len(truths),
        "release_scope": candidate_a.RELEASE_SCOPE,
        "adapter": {
            "base_feature_schema": base_features.FEATURE_SCHEMA,
            "additional_feature": "training-partition PCA64 of frozen OpenL3",
            "openl3_dimension": OPENL3_DIMENSION,
            "ridge_penalty": candidate_a.RIDGE_PENALTY,
            "confidence": "top ridge score minus second ridge score",
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
            "fold_thresholds": thresholds,
            "gate": nested_gate,
        },
        "deployment": {
            "minimum_calibration_offers": candidate_a.MINIMUM_GLOBAL_OFFERS,
            "threshold": deployment_threshold,
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
            else "candidate_b_bounded_negative"
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
    parser.add_argument("--openl3-features", required=True, type=Path)
    parser.add_argument("--openl3-summary", required=True, type=Path)
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
