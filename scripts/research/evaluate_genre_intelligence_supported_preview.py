#!/usr/bin/env python3
"""Calibrate the Plan 070 supported-parent preview after holdout isolation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_candidate as candidate_a
import evaluate_genre_intelligence_clap as candidate_c
import evaluate_genre_intelligence_target_calibration as target_calibration
import extract_genre_intelligence_features as base_features


EXPERIMENT_ID = "genre-intelligence-v1-supported-parent-preview"
METHOD_STATUS = "pre_registered_holdout_isolated_supported_parent_calibration"
SUPPORTED_PARENTS = ["Ambient", "House", "Reggae", "Techno"]
SUPPORTED_INDICES = {
    candidate_a.RELEASE_SCOPE.index(parent) for parent in SUPPORTED_PARENTS
}
CONFIG_FIELDS = {
    "schema_version",
    "experiment_id",
    "supported_parents",
    "candidate_c_config_sha256",
    "development_manifest_sha256",
    "feature_manifest_sha256",
    "base_feature_artifact_sha256",
    "base_feature_summary_sha256",
    "representation_manifest_sha256",
    "clap_artifact_sha256",
    "clap_summary_sha256",
    "development_exclusions_sha256",
    "evaluator_source_sha256",
}


def observed_hashes(args: argparse.Namespace) -> dict[str, str]:
    return {
        "candidate_c_config_sha256": candidate_a.sha256_file(args.candidate_config),
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
        "development_exclusions_sha256": candidate_a.sha256_file(
            args.development_exclusions
        ),
        "evaluator_source_sha256": candidate_a.sha256_file(Path(__file__)),
    }


def validate_candidate_inputs(args: argparse.Namespace, observed: dict[str, str]) -> None:
    source = candidate_c.load_config(args.candidate_config)
    inherited = {
        key: value
        for key, value in observed.items()
        if key
        in {
            "development_manifest_sha256",
            "feature_manifest_sha256",
            "base_feature_artifact_sha256",
            "base_feature_summary_sha256",
            "representation_manifest_sha256",
            "clap_artifact_sha256",
            "clap_summary_sha256",
        }
    }
    if any(source[key] != value for key, value in inherited.items()):
        raise ValueError("supported-preview inputs differ from candidate C")


def prepare_config(args: argparse.Namespace) -> dict[str, Any]:
    observed = observed_hashes(args)
    validate_candidate_inputs(args, observed)
    config = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "supported_parents": SUPPORTED_PARENTS,
        **observed,
    }
    candidate_a.atomic_write(args.output, config)
    return config


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    if set(config) != CONFIG_FIELDS:
        raise ValueError("supported-preview config fields differ")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("supported-preview config identity differs")
    if config["supported_parents"] != SUPPORTED_PARENTS:
        raise ValueError("supported-preview parent scope differs")
    return config


def exclusion_mask(
    development: dict[str, Any], exclusions: dict[str, Any]
) -> np.ndarray:
    if exclusions.get("stage") != "private_holdout_group_development_exclusions":
        raise ValueError("development exclusions have the wrong stage")
    excluded_ids = [str(row["row_id"]) for row in exclusions.get("rows", [])]
    if len(set(excluded_ids)) != len(excluded_ids):
        raise ValueError("development exclusions contain duplicate row identities")
    development_ids = [str(row["row_id"]) for row in development["rows"]]
    unknown = set(excluded_ids) - set(development_ids)
    if unknown:
        raise ValueError("development exclusions contain unknown row identities")
    return np.asarray([row_id not in set(excluded_ids) for row_id in development_ids])


def restricted_thresholds(
    values: dict[int, dict[str, float | int] | None],
) -> dict[int, dict[str, float | int] | None]:
    return {
        index: value if index in SUPPORTED_INDICES else None
        for index, value in values.items()
    }


def restricted_nested_predictions(
    base: np.ndarray,
    clap: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, list[dict[str, Any]]]:
    predictions, margins, _, details = target_calibration.nested_predictions(
        base, clap, truths, folds
    )
    offered = np.zeros(len(truths), dtype=bool)
    restricted_details = []
    for detail in details:
        thresholds = {
            index: detail["thresholds"][target]
            if index in SUPPORTED_INDICES
            else None
            for index, target in enumerate(candidate_a.RELEASE_SCOPE)
        }
        fold_mask = folds == int(detail["fold"])
        offered[fold_mask] = target_calibration.apply_target_thresholds(
            predictions[fold_mask], margins[fold_mask], thresholds
        )
        restricted_details.append(
            {
                "fold": int(detail["fold"]),
                "thresholds": target_calibration.serialized_thresholds(thresholds),
                "calibrated_supported_parents": sum(
                    value is not None for value in thresholds.values()
                ),
            }
        )
    return predictions, margins, offered, restricted_details


def quality_gate(metrics: dict[str, Any], paired: dict[str, Any]) -> dict[str, Any]:
    target_failures = [
        parent
        for parent in SUPPORTED_PARENTS
        if metrics["per_target"][parent]["offers"]
        >= candidate_a.MINIMUM_TARGET_OFFERS
        and metrics["per_target"][parent]["offered_precision"]
        < candidate_a.MINIMUM_TARGET_PRECISION
    ]
    fold_passed = all(
        fold["offers"] == 0
        or fold["offered_precision"] >= candidate_a.MINIMUM_FOLD_PRECISION
        for fold in metrics["folds"]
    )
    result = {
        "offered_precision_at_least_0_90": metrics["offered_precision"]
        >= candidate_a.TARGET_PRECISION,
        "every_fold_precision_at_least_0_85": fold_passed,
        "target_precision_at_least_0_80": not target_failures,
        "target_failures": target_failures,
        "v033_paired_precision_improvement_at_least_0_05": paired[
            "precision_improvement"
        ]
        >= candidate_a.MINIMUM_BASELINE_IMPROVEMENT,
    }
    result["passed"] = all(
        value for key, value in result.items() if key not in {"target_failures"}
    )
    return result


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(args.config)
    observed = observed_hashes(args)
    if any(config[key] != value for key, value in observed.items()):
        raise ValueError("supported-preview inputs differ from the frozen config")
    validate_candidate_inputs(args, observed)

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
        raise ValueError("supported-preview base feature shape differs")
    clap = candidate_c.load_representation(args.clap_features, len(truths))
    exclusions = json.loads(args.development_exclusions.read_text(encoding="utf-8"))
    retained = exclusion_mask(development, exclusions)
    if np.all(retained):
        raise ValueError("supported preview expected at least one isolation exclusion")
    truths = truths[retained]
    folds = folds[retained]
    base = base[retained]
    clap = clap[retained]

    predictions, margins, nested_offered, fold_thresholds = (
        restricted_nested_predictions(base, clap, truths, folds)
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
    nested_gate = quality_gate(nested, nested_paired)

    selected = restricted_thresholds(
        target_calibration.choose_target_thresholds(predictions, margins, truths)
    )
    deployment_offered = target_calibration.apply_target_thresholds(
        predictions, margins, selected
    )
    deployment = candidate_a.metrics(
        truths, predictions, deployment_offered, folds
    )
    deployment_paired = candidate_a.paired_baseline(
        truths, predictions, deployment_offered, baseline_predictions
    )
    deployment_gate = quality_gate(deployment, deployment_paired)
    calibrated_supported = sum(
        selected[index] is not None for index in SUPPORTED_INDICES
    )
    deployment_gate["all_four_supported_parents_calibrated"] = (
        calibrated_supported == len(SUPPORTED_PARENTS)
    )
    deployment_gate["calibrated_supported_parents"] = calibrated_supported
    deployment_gate["passed"] = bool(
        deployment_gate["passed"]
        and deployment_gate["all_four_supported_parents_calibrated"]
    )
    passed = bool(nested_gate["passed"] and deployment_gate["passed"])
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": candidate_a.sha256_file(args.config),
        "inputs": observed,
        "internal_parents": candidate_a.RELEASE_SCOPE,
        "supported_parents": SUPPORTED_PARENTS,
        "source_development_rows": len(retained),
        "excluded_development_rows": int(np.sum(~retained)),
        "retained_development_rows": int(np.sum(retained)),
        "nested": {
            "metrics": nested,
            "paired_v033": nested_paired,
            "fold_thresholds": fold_thresholds,
            "gate": nested_gate,
        },
        "deployment": {
            "thresholds": target_calibration.serialized_thresholds(selected),
            "metrics": deployment,
            "paired_v033": deployment_paired,
            "gate": deployment_gate,
        },
        "passed": passed,
        "outcome": (
            "candidate_ready_for_holdout_inference"
            if passed
            else "supported_parent_preview_bounded_negative"
        ),
    }


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--candidate-config", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--feature-manifest", required=True, type=Path)
    parser.add_argument("--base-features", required=True, type=Path)
    parser.add_argument("--base-feature-summary", required=True, type=Path)
    parser.add_argument("--representation-manifest", required=True, type=Path)
    parser.add_argument("--clap-features", required=True, type=Path)
    parser.add_argument("--clap-summary", required=True, type=Path)
    parser.add_argument("--development-exclusions", required=True, type=Path)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser("prepare-config")
    add_common_arguments(prepare_parser)
    prepare_parser.add_argument("--output", required=True, type=Path)
    evaluate_parser = subparsers.add_parser("evaluate")
    add_common_arguments(evaluate_parser)
    evaluate_parser.add_argument("--config", required=True, type=Path)
    evaluate_parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "prepare-config":
        config = prepare_config(args)
        print(
            json.dumps(
                {
                    "output": str(args.output),
                    "output_sha256": candidate_a.sha256_file(args.output),
                    "supported_parents": config["supported_parents"],
                },
                indent=2,
                sort_keys=True,
            )
        )
        return 0
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
