#!/usr/bin/env python3
"""Fit and infer the sealed Plan 070 supported-parent holdout."""

from __future__ import annotations

import argparse
import json
import os
import tempfile
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_candidate as candidate
import evaluate_genre_intelligence_clap as clap_evaluation
import evaluate_genre_intelligence_openl3 as representation_evaluation
import evaluate_genre_intelligence_supported_preview as preview
import extract_genre_intelligence_features as base_features


EXPERIMENT_ID = preview.EXPERIMENT_ID
METHOD_STATUS = "frozen_full_fit_label_blind_holdout_inference"
PCA_COMPONENTS = representation_evaluation.PCA_COMPONENTS
CONFIG_FIELDS = {
    "schema_version",
    "experiment_id",
    "supported_parents",
    "supported_development_config_sha256",
    "supported_development_result_sha256",
    "candidate_c_config_sha256",
    "development_manifest_sha256",
    "development_feature_manifest_sha256",
    "development_base_features_sha256",
    "development_clap_features_sha256",
    "development_exclusions_sha256",
    "holdout_input_summary_sha256",
    "holdout_feature_manifest_sha256",
    "holdout_representation_manifest_sha256",
    "holdout_base_features_sha256",
    "holdout_base_summary_sha256",
    "holdout_clap_features_sha256",
    "holdout_clap_summary_sha256",
    "decoded_audio_isolation_sha256",
    "clap_model_sha256",
    "inference_source_sha256",
}


def observed_hashes(args: argparse.Namespace) -> dict[str, str]:
    return {
        "supported_development_config_sha256": candidate.sha256_file(
            args.supported_development_config
        ),
        "supported_development_result_sha256": candidate.sha256_file(
            args.supported_development_result
        ),
        "candidate_c_config_sha256": candidate.sha256_file(args.candidate_config),
        "development_manifest_sha256": candidate.sha256_file(
            args.development_manifest
        ),
        "development_feature_manifest_sha256": candidate.sha256_file(
            args.development_feature_manifest
        ),
        "development_base_features_sha256": candidate.sha256_file(
            args.development_base_features
        ),
        "development_clap_features_sha256": candidate.sha256_file(
            args.development_clap_features
        ),
        "development_exclusions_sha256": candidate.sha256_file(
            args.development_exclusions
        ),
        "holdout_input_summary_sha256": candidate.sha256_file(
            args.holdout_input_summary
        ),
        "holdout_feature_manifest_sha256": candidate.sha256_file(
            args.holdout_feature_manifest
        ),
        "holdout_representation_manifest_sha256": candidate.sha256_file(
            args.holdout_representation_manifest
        ),
        "holdout_base_features_sha256": candidate.sha256_file(
            args.holdout_base_features
        ),
        "holdout_base_summary_sha256": candidate.sha256_file(
            args.holdout_base_summary
        ),
        "holdout_clap_features_sha256": candidate.sha256_file(
            args.holdout_clap_features
        ),
        "holdout_clap_summary_sha256": candidate.sha256_file(
            args.holdout_clap_summary
        ),
        "decoded_audio_isolation_sha256": candidate.sha256_file(
            args.decoded_audio_isolation
        ),
        "inference_source_sha256": candidate.sha256_file(Path(__file__)),
    }


def clap_model_sha256(path: Path) -> str:
    summary = json.loads(path.read_text(encoding="utf-8"))
    value = str(summary.get("model_sha256", ""))
    if len(value) != 64:
        raise ValueError("holdout CLAP summary has no model SHA-256")
    return value


def validate_bound_inputs(args: argparse.Namespace, observed: dict[str, str]) -> None:
    supported_config = preview.load_config(args.supported_development_config)
    if supported_config["candidate_c_config_sha256"] != observed[
        "candidate_c_config_sha256"
    ]:
        raise ValueError("supported development config binds another candidate C")
    supported_result = json.loads(
        args.supported_development_result.read_text(encoding="utf-8")
    )
    if supported_result.get("config_sha256") != observed[
        "supported_development_config_sha256"
    ]:
        raise ValueError("supported development result binds another config")
    if not supported_result.get("passed"):
        raise ValueError("supported development calibration has not passed")

    candidate_config = clap_evaluation.load_config(args.candidate_config)
    candidate_bindings = {
        "development_manifest_sha256": "development_manifest_sha256",
        "feature_manifest_sha256": "development_feature_manifest_sha256",
        "base_feature_artifact_sha256": "development_base_features_sha256",
        "clap_artifact_sha256": "development_clap_features_sha256",
    }
    if any(
        candidate_config[source] != observed[target]
        for source, target in candidate_bindings.items()
    ):
        raise ValueError("inference development inputs differ from candidate C")

    input_summary = json.loads(args.holdout_input_summary.read_text(encoding="utf-8"))
    if input_summary.get("feature_manifest_sha256") != observed[
        "holdout_feature_manifest_sha256"
    ] or input_summary.get("representation_manifest_sha256") != observed[
        "holdout_representation_manifest_sha256"
    ]:
        raise ValueError("holdout manifests differ from their input summary")
    base_summary = json.loads(args.holdout_base_summary.read_text(encoding="utf-8"))
    if base_summary.get("artifact_sha256") != observed[
        "holdout_base_features_sha256"
    ] or base_summary.get("manifest_sha256") != observed[
        "holdout_feature_manifest_sha256"
    ]:
        raise ValueError("holdout base features differ from their summary")
    clap_summary = json.loads(args.holdout_clap_summary.read_text(encoding="utf-8"))
    if clap_summary.get("feature_artifact_sha256") != observed[
        "holdout_clap_features_sha256"
    ] or clap_summary.get("manifest_sha256") != observed[
        "holdout_representation_manifest_sha256"
    ]:
        raise ValueError("holdout CLAP features differ from their summary")
    audio_audit = json.loads(args.decoded_audio_isolation.read_text(encoding="utf-8"))
    if not audio_audit.get("passed") or audio_audit.get(
        "holdout_manifest_sha256"
    ) != observed["holdout_feature_manifest_sha256"]:
        raise ValueError("decoded-audio isolation is absent or binds another holdout")


def prepare_config(args: argparse.Namespace) -> dict[str, Any]:
    hashes = observed_hashes(args)
    validate_bound_inputs(args, hashes)
    config = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "supported_parents": preview.SUPPORTED_PARENTS,
        **hashes,
        "clap_model_sha256": clap_model_sha256(args.holdout_clap_summary),
    }
    candidate.atomic_write(args.output, config)
    return config


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    if set(config) != CONFIG_FIELDS:
        raise ValueError("supported-preview inference config fields differ")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("supported-preview inference config identity differs")
    if config["supported_parents"] != preview.SUPPORTED_PARENTS:
        raise ValueError("supported-preview inference parent scope differs")
    return config


def fit_full_model(
    base: np.ndarray, clap: np.ndarray, truths: np.ndarray
) -> dict[str, np.ndarray]:
    if len(base) != len(clap) or len(base) != len(truths) or not len(base):
        raise ValueError("full-fit training inputs have different row counts")
    pca_mean = clap.mean(axis=0)
    centered = clap - pca_mean
    component_count = min(PCA_COMPONENTS, centered.shape[0], centered.shape[1])
    _, _, right = np.linalg.svd(centered, full_matrices=False)
    pca_components = right[:component_count]
    projected = centered @ pca_components.T
    augmented = np.column_stack([base, projected])

    feature_means = np.zeros(augmented.shape[1], dtype=np.float64)
    for column in range(augmented.shape[1]):
        observed = augmented[:, column]
        observed = observed[np.isfinite(observed)]
        feature_means[column] = float(np.mean(observed)) if len(observed) else 0.0
    filled = np.where(np.isfinite(augmented), augmented, feature_means)
    feature_stddev = filled.std(axis=0)
    active = np.isfinite(feature_stddev) & (feature_stddev > 1e-9)
    scaled = (filled[:, active] - feature_means[active]) / feature_stddev[active]

    classes = sorted(set(int(value) for value in truths))
    if classes != list(range(len(candidate.RELEASE_SCOPE))):
        raise ValueError("full-fit training data does not contain every internal parent")
    targets = np.zeros((len(truths), len(classes)), dtype=np.float64)
    targets[np.arange(len(truths)), truths] = 1.0
    x_train = np.column_stack([np.ones(len(truths)), scaled])
    root_weights = np.sqrt(candidate.balanced_weights(truths))
    weighted_x = x_train * root_weights[:, None]
    weighted_y = targets * root_weights[:, None]
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * candidate.RIDGE_PENALTY
    penalty[0, 0] = 0.0
    coefficients = np.linalg.solve(
        weighted_x.T @ weighted_x + penalty,
        weighted_x.T @ weighted_y,
    )
    return {
        "pca_mean": pca_mean.astype("<f8"),
        "pca_components": pca_components.astype("<f8"),
        "feature_means": feature_means.astype("<f8"),
        "feature_stddev": feature_stddev.astype("<f8"),
        "active_features": active,
        "ridge_coefficients": coefficients.astype("<f8"),
        "class_indices": np.asarray(classes, dtype="<i8"),
    }


def score_model(
    model: dict[str, np.ndarray], base: np.ndarray, clap: np.ndarray
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    projected = (clap - model["pca_mean"]) @ model["pca_components"].T
    augmented = np.column_stack([base, projected])
    filled = np.where(np.isfinite(augmented), augmented, model["feature_means"])
    active = model["active_features"]
    scaled = (filled[:, active] - model["feature_means"][active]) / model[
        "feature_stddev"
    ][active]
    x_test = np.column_stack([np.ones(len(base)), scaled])
    scores = x_test @ model["ridge_coefficients"]
    selected = np.argmax(scores, axis=1)
    predictions = model["class_indices"][selected].astype(np.int64)
    top = scores[np.arange(len(scores)), selected]
    second = np.partition(scores, -2, axis=1)[:, -2]
    return scores, predictions, np.maximum(0.0, top - second)


def atomic_save_model(path: Path, model: dict[str, np.ndarray]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".npz"
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        np.savez_compressed(temporary, **model)
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(args.config)
    observed = observed_hashes(args)
    if any(config[key] != value for key, value in observed.items()):
        raise ValueError("supported-preview inference inputs differ from config")
    if config["clap_model_sha256"] != clap_model_sha256(args.holdout_clap_summary):
        raise ValueError("supported-preview CLAP model SHA-256 differs")
    validate_bound_inputs(args, observed)
    development_result = json.loads(
        args.supported_development_result.read_text(encoding="utf-8")
    )

    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    development_features = json.loads(
        args.development_feature_manifest.read_text(encoding="utf-8")
    )
    truths, _ = candidate.validate_manifests(development, development_features)
    exclusions = json.loads(args.development_exclusions.read_text(encoding="utf-8"))
    retained = preview.exclusion_mask(development, exclusions)
    truths = truths[retained]
    development_base = np.load(args.development_base_features, allow_pickle=False)[
        retained
    ]
    development_clap = clap_evaluation.load_representation(
        args.development_clap_features, len(retained)
    )[retained]

    holdout_features = json.loads(
        args.holdout_feature_manifest.read_text(encoding="utf-8")
    )
    holdout_representation = json.loads(
        args.holdout_representation_manifest.read_text(encoding="utf-8")
    )
    feature_ids = [str(row["row_id"]) for row in holdout_features["rows"]]
    if feature_ids != [
        str(row["row_id"]) for row in holdout_representation["rows"]
    ]:
        raise ValueError("holdout base and CLAP identity order differs")
    holdout_base = np.load(args.holdout_base_features, allow_pickle=False)
    if holdout_base.shape != (len(feature_ids), len(base_features.FEATURE_NAMES)):
        raise ValueError("holdout base feature shape differs")
    holdout_clap = clap_evaluation.load_representation(
        args.holdout_clap_features, len(feature_ids)
    )

    thresholds_by_name = development_result["deployment"]["thresholds"]
    thresholds = np.full(len(candidate.RELEASE_SCOPE), np.nan, dtype=np.float64)
    for index, parent in enumerate(candidate.RELEASE_SCOPE):
        selected = thresholds_by_name[parent]
        if parent in preview.SUPPORTED_PARENTS:
            if selected is None:
                raise ValueError("supported parent has no frozen threshold")
            thresholds[index] = float(selected["threshold"])
        elif selected is not None:
            raise ValueError("unsupported parent has a frozen threshold")

    model = fit_full_model(development_base, development_clap, truths)
    model["supported_indices"] = np.asarray(
        sorted(preview.SUPPORTED_INDICES), dtype="<i8"
    )
    model["thresholds"] = thresholds.astype("<f8")
    scores, predictions, margins = score_model(model, holdout_base, holdout_clap)

    combined_base = np.row_stack([development_base, holdout_base])
    combined_clap = np.row_stack([development_clap, holdout_clap])
    combined_truths = np.concatenate(
        [truths, np.full(len(holdout_base), -1, dtype=np.int64)]
    )
    train_mask = np.arange(len(combined_base)) < len(development_base)
    test_mask = ~train_mask
    _, reference_predictions, reference_margins = (
        representation_evaluation.score_partition(
            combined_base,
            combined_clap,
            combined_truths,
            train_mask,
            test_mask,
        )
    )
    if not np.array_equal(predictions, reference_predictions) or not np.allclose(
        margins, reference_margins, rtol=1e-12, atol=1e-12
    ):
        raise ValueError("serialized full-fit model differs from frozen evaluator")

    offered = np.asarray(
        [
            prediction in preview.SUPPORTED_INDICES
            and margin >= thresholds[prediction]
            for prediction, margin in zip(predictions, margins, strict=True)
        ],
        dtype=bool,
    )
    atomic_save_model(args.model_output, model)
    rows = []
    for row_id, prediction, margin, is_offered in zip(
        feature_ids, predictions, margins, offered, strict=True
    ):
        internal_parent = candidate.RELEASE_SCOPE[int(prediction)]
        rows.append(
            {
                "row_id": row_id,
                "internal_parent": internal_parent,
                "margin": float(margin),
                "offered": bool(is_offered),
                "suggested_parent": internal_parent if is_offered else None,
            }
        )
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": candidate.sha256_file(args.config),
        "inputs": observed,
        "model_artifact_sha256": candidate.sha256_file(args.model_output),
        "model_schema": {
            "base_feature_schema": base_features.FEATURE_SCHEMA,
            "clap_dimension": clap_evaluation.CLAP_DIMENSION,
            "pca_components": int(model["pca_components"].shape[0]),
            "ridge_penalty": candidate.RIDGE_PENALTY,
            "internal_parents": candidate.RELEASE_SCOPE,
            "supported_parents": preview.SUPPORTED_PARENTS,
            "thresholds": {
                parent: (
                    float(thresholds[index])
                    if np.isfinite(thresholds[index])
                    else None
                )
                for index, parent in enumerate(candidate.RELEASE_SCOPE)
            },
        },
        "rows": rows,
        "holdout_rows": len(rows),
        "offers": int(np.sum(offered)),
        "abstentions": int(np.sum(~offered)),
        "all_predictions_frozen_before_review": True,
        "identity_values_exposed": False,
        "reference_implementation_match": True,
        "score_matrix_shape": list(scores.shape),
    }
    candidate.atomic_write(args.output, result)
    return result


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--supported-development-config", required=True, type=Path)
    parser.add_argument("--supported-development-result", required=True, type=Path)
    parser.add_argument("--candidate-config", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--development-feature-manifest", required=True, type=Path)
    parser.add_argument("--development-base-features", required=True, type=Path)
    parser.add_argument("--development-clap-features", required=True, type=Path)
    parser.add_argument("--development-exclusions", required=True, type=Path)
    parser.add_argument("--holdout-input-summary", required=True, type=Path)
    parser.add_argument("--holdout-feature-manifest", required=True, type=Path)
    parser.add_argument("--holdout-representation-manifest", required=True, type=Path)
    parser.add_argument("--holdout-base-features", required=True, type=Path)
    parser.add_argument("--holdout-base-summary", required=True, type=Path)
    parser.add_argument("--holdout-clap-features", required=True, type=Path)
    parser.add_argument("--holdout-clap-summary", required=True, type=Path)
    parser.add_argument("--decoded-audio-isolation", required=True, type=Path)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser("prepare-config")
    add_common_arguments(prepare_parser)
    prepare_parser.add_argument("--output", required=True, type=Path)
    infer_parser = subparsers.add_parser("infer")
    add_common_arguments(infer_parser)
    infer_parser.add_argument("--config", required=True, type=Path)
    infer_parser.add_argument("--model-output", required=True, type=Path)
    infer_parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "prepare-config":
        config = prepare_config(args)
        print(
            json.dumps(
                {
                    "output": str(args.output),
                    "output_sha256": candidate.sha256_file(args.output),
                    "clap_model_sha256": config["clap_model_sha256"],
                },
                indent=2,
                sort_keys=True,
            )
        )
        return 0
    result = run(args)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "model_output": str(args.model_output),
                "holdout_rows": result["holdout_rows"],
                "offers": result["offers"],
                "abstentions": result["abstentions"],
                "all_predictions_frozen_before_review": result[
                    "all_predictions_frozen_before_review"
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
