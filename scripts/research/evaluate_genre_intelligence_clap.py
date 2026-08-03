#!/usr/bin/env python3
"""Evaluate frozen Plan 068 candidate C with CLAP PCA64."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as thresholding
import evaluate_genre_intelligence_candidate as candidate_a
import evaluate_genre_intelligence_openl3 as representation_evaluation
import extract_genre_intelligence_features as base_features
import prepare_genre_intelligence_candidate as preparation


EXPERIMENT_ID = "genre-intelligence-v1-candidate-c-clap"
METHOD_STATUS = "pre_registered_artist_isolated_clap_development_evaluation"
CLAP_DIMENSION = 512


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
        "clap_artifact_sha256",
        "clap_summary_sha256",
        "source_corpus_sha256",
        "accepted_rows",
        "model_ready_rows",
        "release_scope",
    }
    if set(config) != required:
        raise ValueError("candidate-C config fields differ from the frozen schema")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("candidate-C config identity differs")
    if config["release_scope"] != candidate_a.RELEASE_SCOPE:
        raise ValueError("candidate-C release scope differs")
    if config["source_corpus_sha256"] != preparation.EXPECTED_CORPUS_SHA256:
        raise ValueError("candidate-C source corpus differs")
    return config


def load_representation(path: Path, expected_rows: int) -> np.ndarray:
    artifact = np.load(path, allow_pickle=False)
    if list(artifact.files) != ["embeddings"]:
        raise ValueError("CLAP artifact arrays differ")
    result = np.asarray(artifact["embeddings"], dtype=np.float64)
    if result.shape != (expected_rows, CLAP_DIMENSION) or not np.all(
        np.isfinite(result)
    ):
        raise ValueError("CLAP feature shape or values differ")
    return result


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
        "clap_artifact_sha256": candidate_a.sha256_file(args.clap_features),
        "clap_summary_sha256": candidate_a.sha256_file(args.clap_summary),
    }
    if any(config[key] != value for key, value in observed_hashes.items()):
        raise ValueError("candidate-C input hashes differ from the frozen config")

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
        raise ValueError("candidate-C base feature shape differs")
    clap = load_representation(args.clap_features, len(truths))

    predictions, margins, nested_offered, thresholds = (
        representation_evaluation.nested_predictions(base, clap, truths, folds)
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
            "additional_feature": "training-partition PCA64 of frozen CLAP",
            "clap_dimension": CLAP_DIMENSION,
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
            else "candidate_c_bounded_negative"
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
