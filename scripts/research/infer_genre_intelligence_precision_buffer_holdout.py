#!/usr/bin/env python3
"""Fit O3 and infer the sealed Plan 071/072 holdout label-blind."""

from __future__ import annotations

import argparse
import json
import os
import tempfile
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_open_set as plan071
import evaluate_genre_intelligence_precision_buffer as plan072


EXPERIMENT_ID = "genre-intelligence-v1-precision-buffer-holdout"
METHOD_STATUS = "frozen_full_fit_label_blind_holdout_inference"
EXPECTED_INPUT_SHA256 = {
    "development_result": (
        "9d1683960d2ace05aa553965e4bf486c46df509eb9147ed0d202eff02ec6eb5d"
    ),
    "development_manifest": (
        "dfd11addd96a2e7b5727700594b337aaacfc19bdd97db408e1ba0955f80853bd"
    ),
    "development_feature_manifest": (
        "6bf80b80f060649877a90a5d6dfa8188c9549eaa0986f1667d611e115689b682"
    ),
    "development_base_features": (
        "f3e615a89f5b3770e170f0b7ddafd29e87052fcbf6a44c333ba0f9aced331365"
    ),
    "development_clap_features": (
        "72fbace49fdcb2885d4dce78fac3f1212baac1742718d903c6203314f4e4ffc9"
    ),
    "holdout_input_summary": (
        "a780db141c4e75e02b36f0f9133d8f0ade955ab765c72b7d83ce66aacb93c407"
    ),
    "holdout_feature_manifest": (
        "1629f8db56e0c84b9900e8fe0ea6656d38dcdfbd8e3cc6d80585abf650e3a0d2"
    ),
    "holdout_representation_manifest": (
        "56c7be498fc6cd86a479652a93db59f6da4a0aaa83950bd52a5becc751e9e6d1"
    ),
    "holdout_base_features": (
        "033b256fe5f41565c43cb86775689fab1fb43969137f0729e8d7c42c85be1818"
    ),
    "holdout_base_summary": (
        "39be523c1bcdf8533dafefcb5344d339e489e7c660f00011cd3fcf9c56b78a60"
    ),
    "holdout_clap_features": (
        "ef20a57dbc4b7bd96414a574a1d2d667ded404f6492a852d9a89c3eab549047c"
    ),
    "holdout_clap_summary": (
        "7242c46ebe1f281ab583f9e7c15d09e6b4a8c6b0f7e7dde57e33a73187cb0bb9"
    ),
    "decoded_audio_isolation": (
        "c54dbc921041b3cf9d530dd8e2fb7d7c3f7fa701978051213be77081443dfa4c"
    ),
}
CONFIG_FIELDS = {
    "schema_version",
    "experiment_id",
    "inputs",
    "inference_source_sha256",
    "plan071_evaluator_source_sha256",
    "plan072_evaluator_source_sha256",
    "clap_model_sha256",
}


def argument_paths(args: argparse.Namespace) -> dict[str, Path]:
    return {
        "development_result": args.development_result,
        "development_manifest": args.development_manifest,
        "development_feature_manifest": args.development_feature_manifest,
        "development_base_features": args.development_base_features,
        "development_clap_features": args.development_clap_features,
        "holdout_input_summary": args.holdout_input_summary,
        "holdout_feature_manifest": args.holdout_feature_manifest,
        "holdout_representation_manifest": args.holdout_representation_manifest,
        "holdout_base_features": args.holdout_base_features,
        "holdout_base_summary": args.holdout_base_summary,
        "holdout_clap_features": args.holdout_clap_features,
        "holdout_clap_summary": args.holdout_clap_summary,
        "decoded_audio_isolation": args.decoded_audio_isolation,
    }


def observed_hashes(args: argparse.Namespace) -> dict[str, str]:
    return {
        name: plan071.sha256_file(path) for name, path in argument_paths(args).items()
    }


def load_clap(path: Path, rows: int) -> np.ndarray:
    artifact = np.load(path, allow_pickle=False)
    if list(artifact.files) != ["embeddings"]:
        raise ValueError("CLAP artifact arrays differ")
    values = np.asarray(artifact["embeddings"], dtype=np.float64)
    if values.shape != (rows, plan071.CLAP_DIMENSION) or not np.all(
        np.isfinite(values)
    ):
        raise ValueError("CLAP feature shape or values differ")
    return values


def validate_bound_inputs(args: argparse.Namespace) -> str:
    if observed_hashes(args) != EXPECTED_INPUT_SHA256:
        raise ValueError("holdout inference inputs differ from the frozen artifacts")
    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    if not development_result.get("passed") or not development_result.get(
        "candidate", {}
    ).get("deployment_calibration", {}).get("passed"):
        raise ValueError("O3 development and deployment calibration have not passed")
    if development_result.get("adapter", {}).get(
        "inner_calibration_precision"
    ) != plan072.CALIBRATION_PRECISION:
        raise ValueError("O3 calibration precision differs")

    holdout_summary = json.loads(
        args.holdout_input_summary.read_text(encoding="utf-8")
    )
    if holdout_summary.get("feature_manifest_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_feature_manifest"
    ] or holdout_summary.get(
        "representation_manifest_sha256"
    ) != EXPECTED_INPUT_SHA256[
        "holdout_representation_manifest"
    ]:
        raise ValueError("holdout manifests differ from their audited summary")
    base_summary = json.loads(args.holdout_base_summary.read_text(encoding="utf-8"))
    if base_summary.get("artifact_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_base_features"
    ] or base_summary.get("manifest_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_feature_manifest"
    ]:
        raise ValueError("holdout base features differ from their summary")
    clap_summary = json.loads(args.holdout_clap_summary.read_text(encoding="utf-8"))
    if clap_summary.get("feature_artifact_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_clap_features"
    ] or clap_summary.get("manifest_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_representation_manifest"
    ]:
        raise ValueError("holdout CLAP features differ from their summary")
    isolation = json.loads(
        args.decoded_audio_isolation.read_text(encoding="utf-8")
    )
    if not isolation.get("passed") or isolation.get(
        "cross_partition_decoded_audio_overlap"
    ) != 0:
        raise ValueError("decoded-audio isolation has not passed")
    model_sha = str(clap_summary.get("model_sha256", ""))
    if len(model_sha) != 64:
        raise ValueError("holdout CLAP summary has no model SHA-256")
    return model_sha


def prepare_config(args: argparse.Namespace) -> dict[str, Any]:
    model_sha = validate_bound_inputs(args)
    config = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "inputs": EXPECTED_INPUT_SHA256,
        "inference_source_sha256": plan071.sha256_file(Path(__file__)),
        "plan071_evaluator_source_sha256": plan071.sha256_file(
            Path(plan071.__file__)
        ),
        "plan072_evaluator_source_sha256": plan071.sha256_file(
            Path(plan072.__file__)
        ),
        "clap_model_sha256": model_sha,
    }
    plan071.atomic_write(args.output, config)
    return config


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    if set(config) != CONFIG_FIELDS:
        raise ValueError("holdout inference config fields differ")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("holdout inference config identity differs")
    return config


def truth_targets(development: dict[str, Any]) -> np.ndarray:
    rows = development.get("rows")
    if (
        not isinstance(rows, list)
        or len(rows) != plan071.preparation.EXPECTED_ACCEPTED_ROWS
    ):
        raise ValueError("development truth row count differs")
    return np.asarray(
        [
            plan071.preparation.OUTPUT_PARENTS.index(parent)
            if (parent := str(row["canonical_parent_genre"]))
            in plan071.preparation.OUTPUT_PARENTS
            else -1
            for row in rows
        ],
        dtype=np.int64,
    )


def fit_full_model(
    base: np.ndarray, clap: np.ndarray, truths: np.ndarray
) -> dict[str, np.ndarray]:
    if len(base) != len(clap) or len(base) != len(truths) or not len(base):
        raise ValueError("full-fit training inputs have different row counts")
    pca_mean = clap.mean(axis=0)
    centered = clap - pca_mean
    component_count = min(
        plan071.PCA_COMPONENTS, centered.shape[0], centered.shape[1]
    )
    _, _, right = np.linalg.svd(centered, full_matrices=False)
    pca_components = right[:component_count]
    augmented = np.column_stack([base, centered @ pca_components.T])

    feature_means = np.zeros(augmented.shape[1], dtype=np.float64)
    for column in range(augmented.shape[1]):
        observed = augmented[:, column]
        observed = observed[np.isfinite(observed)]
        feature_means[column] = float(np.mean(observed)) if len(observed) else 0.0
    filled = np.where(np.isfinite(augmented), augmented, feature_means)
    feature_stddev = filled.std(axis=0)
    active = np.isfinite(feature_stddev) & (feature_stddev > 1e-9)
    if not np.any(active):
        raise ValueError("full-fit training data has no active features")
    scaled = (filled[:, active] - feature_means[active]) / feature_stddev[active]
    x_train = np.column_stack([np.ones(len(truths)), scaled])
    coefficients = np.zeros(
        (x_train.shape[1], len(plan071.preparation.OUTPUT_PARENTS)),
        dtype=np.float64,
    )
    penalty = np.eye(x_train.shape[1], dtype=np.float64) * plan071.RIDGE_PENALTY
    penalty[0, 0] = 0.0
    for target in range(len(plan071.preparation.OUTPUT_PARENTS)):
        binary_truth = truths == target
        root_weights = np.sqrt(plan071.binary_weights(binary_truth))
        weighted_x = x_train * root_weights[:, None]
        weighted_y = binary_truth.astype(np.float64) * root_weights
        coefficients[:, target] = np.linalg.solve(
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
    }


def score_model(
    model: dict[str, np.ndarray], base: np.ndarray, clap: np.ndarray
) -> np.ndarray:
    projected = (clap - model["pca_mean"]) @ model["pca_components"].T
    augmented = np.column_stack([base, projected])
    filled = np.where(np.isfinite(augmented), augmented, model["feature_means"])
    active = model["active_features"]
    scaled = (filled[:, active] - model["feature_means"][active]) / model[
        "feature_stddev"
    ][active]
    x_test = np.column_stack([np.ones(len(base)), scaled])
    return x_test @ model["ridge_coefficients"]


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
    model_sha = validate_bound_inputs(args)
    if config["inputs"] != EXPECTED_INPUT_SHA256:
        raise ValueError("holdout inference config binds different inputs")
    current_sources = {
        "inference_source_sha256": plan071.sha256_file(Path(__file__)),
        "plan071_evaluator_source_sha256": plan071.sha256_file(
            Path(plan071.__file__)
        ),
        "plan072_evaluator_source_sha256": plan071.sha256_file(
            Path(plan072.__file__)
        ),
        "clap_model_sha256": model_sha,
    }
    if any(config[key] != value for key, value in current_sources.items()):
        raise ValueError("holdout inference implementation or CLAP model differs")

    development = json.loads(
        args.development_manifest.read_text(encoding="utf-8")
    )
    development_features = json.loads(
        args.development_feature_manifest.read_text(encoding="utf-8")
    )
    if [row["row_id"] for row in development["rows"]] != [
        row["row_id"] for row in development_features["rows"]
    ]:
        raise ValueError("development truth and feature order differs")
    truths = truth_targets(development)
    development_base = np.load(args.development_base_features, allow_pickle=False)
    development_clap = load_clap(args.development_clap_features, len(truths))

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
    if any(
        set(row) != {"row_id", "file_path"} for row in holdout_features["rows"]
    ):
        raise ValueError("holdout feature manifest contains non-identity fields")
    holdout_base = np.load(args.holdout_base_features, allow_pickle=False)
    if holdout_base.shape != (
        len(feature_ids),
        len(plan071.base_features.FEATURE_NAMES),
    ):
        raise ValueError("holdout base feature shape differs")
    holdout_clap = load_clap(args.holdout_clap_features, len(feature_ids))

    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    threshold_rows = development_result["candidate"]["deployment_calibration"][
        "thresholds"
    ]
    if [row["parent"] for row in threshold_rows] != plan071.preparation.OUTPUT_PARENTS:
        raise ValueError("deployment threshold parent order differs")
    thresholds = np.asarray(
        [
            float(row["threshold"]) if row["threshold"] is not None else np.nan
            for row in threshold_rows
        ],
        dtype=np.float64,
    )

    model = fit_full_model(development_base, development_clap, truths)
    model["thresholds"] = thresholds.astype("<f8")
    scores = score_model(model, holdout_base, holdout_clap)
    combined_base = np.vstack([development_base, holdout_base])
    combined_clap = np.vstack([development_clap, holdout_clap])
    combined_truths = np.concatenate(
        [truths, np.full(len(holdout_base), -1, dtype=np.int64)]
    )
    train_mask = np.arange(len(combined_base)) < len(development_base)
    test_mask = ~train_mask
    _, reference_scores = plan071.score_o2_partition(
        combined_base,
        combined_clap,
        combined_truths,
        train_mask,
        test_mask,
    )
    if not np.allclose(scores, reference_scores, rtol=1e-12, atol=1e-12):
        raise ValueError("serialized full-fit O3 differs from the frozen evaluator")

    qualified = np.isfinite(thresholds)[None, :] & (scores >= thresholds[None, :])
    qualified_counts = np.sum(qualified, axis=1)
    offered = qualified_counts == 1
    predictions = np.full(len(feature_ids), -1, dtype=np.int64)
    predictions[offered] = np.argmax(qualified[offered], axis=1)
    atomic_save_model(args.model_output, model)
    rows = []
    for row_id, row_scores, row_qualified, count, prediction, is_offered in zip(
        feature_ids,
        scores,
        qualified,
        qualified_counts,
        predictions,
        offered,
        strict=True,
    ):
        suggested = (
            plan071.preparation.OUTPUT_PARENTS[int(prediction)]
            if is_offered
            else None
        )
        rows.append(
            {
                "row_id": row_id,
                "scores": {
                    parent: float(row_scores[index])
                    for index, parent in enumerate(
                        plan071.preparation.OUTPUT_PARENTS
                    )
                },
                "qualified_parents": [
                    parent
                    for parent, value in zip(
                        plan071.preparation.OUTPUT_PARENTS,
                        row_qualified,
                        strict=True,
                    )
                    if value
                ],
                "qualified_count": int(count),
                "offered": bool(is_offered),
                "suggested_parent": suggested,
            }
        )
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": plan071.sha256_file(args.config),
        "inputs": EXPECTED_INPUT_SHA256,
        "model_artifact_sha256": plan071.sha256_file(args.model_output),
        "model_schema": {
            "formulation": "seven binary ridge models with collision abstention",
            "base_feature_schema": plan071.base_features.FEATURE_SCHEMA,
            "clap_dimension": plan071.CLAP_DIMENSION,
            "pca_components": int(model["pca_components"].shape[0]),
            "ridge_penalty": plan071.RIDGE_PENALTY,
            "output_parents": plan071.preparation.OUTPUT_PARENTS,
            "thresholds": {
                parent: (float(value) if np.isfinite(value) else None)
                for parent, value in zip(
                    plan071.preparation.OUTPUT_PARENTS, thresholds, strict=True
                )
            },
        },
        "rows": rows,
        "holdout_rows": len(rows),
        "offers": int(np.sum(offered)),
        "abstentions": int(np.sum(~offered)),
        "zero_qualified": int(np.sum(qualified_counts == 0)),
        "one_qualified": int(np.sum(qualified_counts == 1)),
        "multi_qualified": int(np.sum(qualified_counts > 1)),
        "all_predictions_frozen_before_review": True,
        "identity_values_exposed": False,
        "reference_implementation_match": True,
        "score_matrix_shape": list(scores.shape),
    }
    plan071.atomic_write(args.output, result)
    return result


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--development-result", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument(
        "--development-feature-manifest", required=True, type=Path
    )
    parser.add_argument("--development-base-features", required=True, type=Path)
    parser.add_argument("--development-clap-features", required=True, type=Path)
    parser.add_argument("--holdout-input-summary", required=True, type=Path)
    parser.add_argument("--holdout-feature-manifest", required=True, type=Path)
    parser.add_argument(
        "--holdout-representation-manifest", required=True, type=Path
    )
    parser.add_argument("--holdout-base-features", required=True, type=Path)
    parser.add_argument("--holdout-base-summary", required=True, type=Path)
    parser.add_argument("--holdout-clap-features", required=True, type=Path)
    parser.add_argument("--holdout-clap-summary", required=True, type=Path)
    parser.add_argument("--decoded-audio-isolation", required=True, type=Path)


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser("prepare-config")
    add_common_arguments(prepare)
    prepare.add_argument("--output", required=True, type=Path)
    infer = subparsers.add_parser("run")
    add_common_arguments(infer)
    infer.add_argument("--config", required=True, type=Path)
    infer.add_argument("--model-output", required=True, type=Path)
    infer.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "prepare-config":
        result = prepare_config(args)
        print(
            json.dumps(
                {
                    "config_sha256": plan071.sha256_file(args.output),
                    "inference_source_sha256": result[
                        "inference_source_sha256"
                    ],
                    "inputs_bound": len(result["inputs"]),
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
                "output_sha256": plan071.sha256_file(args.output),
                "model_artifact_sha256": result["model_artifact_sha256"],
                "holdout_rows": result["holdout_rows"],
                "offers": result["offers"],
                "abstentions": result["abstentions"],
                "zero_qualified": result["zero_qualified"],
                "one_qualified": result["one_qualified"],
                "multi_qualified": result["multi_qualified"],
                "identity_values_exposed": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
