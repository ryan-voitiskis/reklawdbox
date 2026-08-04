#!/usr/bin/env python3
"""Run the frozen O3 model on the powered Plan 073 holdout."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import numpy as np

import evaluate_genre_intelligence_open_set as plan071
import infer_genre_intelligence_precision_buffer_holdout as frozen


EXPERIMENT_ID = "genre-intelligence-precision-first-holdout-v1"
METHOD_STATUS = "frozen_model_powered_holdout_inference"
EXPECTED_FROZEN_INFERENCE_SOURCE_SHA256 = (
    "9d1675acdb0cf5bb532bcf7763a628f4a14bfff7caeb9d1320ca9d087672746f"
)
EXPECTED_FROZEN_MODEL_SHA256 = (
    "2633db33edc2e2af4e3e42adae9cea945f4453f7524522c4f4fecca8530f30df"
)
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
    "frozen_model": EXPECTED_FROZEN_MODEL_SHA256,
    "holdout_input_summary": (
        "9bdc70094721f827ae4b461ced24f96227ce3a5377bc414e6a916bf981e8d2a3"
    ),
    "holdout_feature_manifest": (
        "a324393657531919df7b143b400c329e5b74288aebdb0eaae4871457169f9380"
    ),
    "holdout_representation_manifest": (
        "aa717ac333772395902cf4b376fca3d2f3a1c8c138357092f2f0306feec2858a"
    ),
    "holdout_base_features": (
        "369050fb519340a888794965f8ff41aa2e69bb14af2490043f43921df1cd351a"
    ),
    "holdout_base_summary": (
        "f6238b6c4ac3b14f40078c17bc81d07dce275806ea7acd4cdc6311e0d67a13f0"
    ),
    "holdout_clap_features": (
        "c137bce90faef12683644d03c05adb5dc40387fccba5ad24da4b10e82f962f3f"
    ),
    "holdout_clap_summary": (
        "73114ab7a5816315332cd02a8aa02a060ddc5d1e5a7b89230233ab26fd87740d"
    ),
    "decoded_audio_isolation": (
        "3b11b896511c0571aec64f23f4ed7711fb11b5738d0cbb95b12609374168b3ac"
    ),
}
CONFIG_FIELDS = {
    "schema_version",
    "experiment_id",
    "inputs",
    "inference_source_sha256",
    "frozen_inference_source_sha256",
    "clap_model_sha256",
}


def argument_paths(args: argparse.Namespace) -> dict[str, Path]:
    return {
        "development_result": args.development_result,
        "development_manifest": args.development_manifest,
        "development_feature_manifest": args.development_feature_manifest,
        "development_base_features": args.development_base_features,
        "development_clap_features": args.development_clap_features,
        "frozen_model": args.frozen_model,
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


def validate_bound_inputs(args: argparse.Namespace) -> str:
    if observed_hashes(args) != EXPECTED_INPUT_SHA256:
        raise ValueError("powered holdout inference inputs differ")
    if plan071.sha256_file(Path(frozen.__file__)) != (
        EXPECTED_FROZEN_INFERENCE_SOURCE_SHA256
    ):
        raise ValueError("frozen O3 inference implementation differs")
    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    if not development_result.get("passed") or not development_result.get(
        "candidate", {}
    ).get("deployment_calibration", {}).get("passed"):
        raise ValueError("O3 development result has not passed")
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
        raise ValueError("powered holdout manifests differ from their audit")
    base_summary = json.loads(args.holdout_base_summary.read_text(encoding="utf-8"))
    if base_summary.get("artifact_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_base_features"
    ] or base_summary.get("manifest_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_feature_manifest"
    ]:
        raise ValueError("powered holdout base features differ from their summary")
    clap_summary = json.loads(args.holdout_clap_summary.read_text(encoding="utf-8"))
    if clap_summary.get("feature_artifact_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_clap_features"
    ] or clap_summary.get("manifest_sha256") != EXPECTED_INPUT_SHA256[
        "holdout_representation_manifest"
    ]:
        raise ValueError("powered holdout CLAP differs from its summary")
    isolation = json.loads(
        args.decoded_audio_isolation.read_text(encoding="utf-8")
    )
    if not isolation.get("passed") or isolation.get(
        "cross_partition_decoded_audio_overlap"
    ) != 0:
        raise ValueError("powered holdout decoded-audio isolation has not passed")
    model_sha = str(clap_summary.get("model_sha256", ""))
    if len(model_sha) != 64:
        raise ValueError("powered holdout CLAP summary has no model SHA-256")
    return model_sha


def prepare_config(args: argparse.Namespace) -> dict[str, Any]:
    model_sha = validate_bound_inputs(args)
    config = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "inputs": EXPECTED_INPUT_SHA256,
        "inference_source_sha256": plan071.sha256_file(Path(__file__)),
        "frozen_inference_source_sha256": (
            EXPECTED_FROZEN_INFERENCE_SOURCE_SHA256
        ),
        "clap_model_sha256": model_sha,
    }
    plan071.atomic_write(args.output, config)
    return config


def load_config(path: Path) -> dict[str, Any]:
    config = json.loads(path.read_text(encoding="utf-8"))
    if set(config) != CONFIG_FIELDS:
        raise ValueError("powered holdout inference config fields differ")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("powered holdout inference config identity differs")
    return config


def baseline_predictions(base: np.ndarray) -> list[str | None]:
    values = base[:, -len(plan071.base_features.BASELINE_FEATURES) :]
    selected = np.argmax(values, axis=1)
    return [
        plan071.preparation.OUTPUT_PARENTS[int(index)]
        if index < len(plan071.preparation.OUTPUT_PARENTS)
        else None
        for index in selected
    ]


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(args.config)
    model_sha = validate_bound_inputs(args)
    current = {
        "inference_source_sha256": plan071.sha256_file(Path(__file__)),
        "frozen_inference_source_sha256": (
            EXPECTED_FROZEN_INFERENCE_SOURCE_SHA256
        ),
        "clap_model_sha256": model_sha,
    }
    if config["inputs"] != EXPECTED_INPUT_SHA256 or any(
        config[key] != value for key, value in current.items()
    ):
        raise ValueError("powered holdout config or implementation differs")

    development = json.loads(
        args.development_manifest.read_text(encoding="utf-8")
    )
    development_features = json.loads(
        args.development_feature_manifest.read_text(encoding="utf-8")
    )
    if [row["row_id"] for row in development["rows"]] != [
        row["row_id"] for row in development_features["rows"]
    ]:
        raise ValueError("development identity order differs")
    truths = frozen.truth_targets(development)
    development_base = np.load(args.development_base_features, allow_pickle=False)
    development_clap = frozen.load_clap(
        args.development_clap_features, len(truths)
    )
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
        raise ValueError("powered holdout identity order differs")
    if len(feature_ids) != 150 or any(
        set(row) != {"row_id", "file_path"} for row in holdout_features["rows"]
    ):
        raise ValueError("powered holdout feature manifest differs")
    holdout_base = np.load(args.holdout_base_features, allow_pickle=False)
    if holdout_base.shape != (
        len(feature_ids),
        len(plan071.base_features.FEATURE_NAMES),
    ):
        raise ValueError("powered holdout base feature shape differs")
    holdout_clap = frozen.load_clap(args.holdout_clap_features, len(feature_ids))

    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    threshold_rows = development_result["candidate"]["deployment_calibration"][
        "thresholds"
    ]
    if [row["parent"] for row in threshold_rows] != plan071.preparation.OUTPUT_PARENTS:
        raise ValueError("O3 deployment threshold order differs")
    thresholds = np.asarray(
        [
            float(row["threshold"]) if row["threshold"] is not None else np.nan
            for row in threshold_rows
        ],
        dtype=np.float64,
    )
    model = frozen.fit_full_model(development_base, development_clap, truths)
    model["thresholds"] = thresholds.astype("<f8")
    frozen.atomic_save_model(args.model_output, model)
    if plan071.sha256_file(args.model_output) != EXPECTED_FROZEN_MODEL_SHA256:
        raise ValueError("refitted O3 model differs from the frozen model")
    scores = frozen.score_model(model, holdout_base, holdout_clap)

    combined_base = np.vstack([development_base, holdout_base])
    combined_clap = np.vstack([development_clap, holdout_clap])
    combined_truths = np.concatenate(
        [truths, np.full(len(holdout_base), -1, dtype=np.int64)]
    )
    train_mask = np.arange(len(combined_base)) < len(development_base)
    _, reference_scores = plan071.score_o2_partition(
        combined_base,
        combined_clap,
        combined_truths,
        train_mask,
        ~train_mask,
    )
    if not np.allclose(scores, reference_scores, rtol=1e-12, atol=1e-12):
        raise ValueError("powered holdout scores differ from the frozen evaluator")

    qualified = np.isfinite(thresholds)[None, :] & (scores >= thresholds[None, :])
    qualified_counts = np.sum(qualified, axis=1)
    offered = qualified_counts == 1
    predictions = np.full(len(feature_ids), -1, dtype=np.int64)
    predictions[offered] = np.argmax(qualified[offered], axis=1)
    baselines = baseline_predictions(holdout_base)
    rows = []
    for (
        row_id,
        row_scores,
        row_qualified,
        count,
        prediction,
        is_offered,
        baseline,
    ) in zip(
        feature_ids,
        scores,
        qualified,
        qualified_counts,
        predictions,
        offered,
        baselines,
        strict=True,
    ):
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
                "suggested_parent": (
                    plan071.preparation.OUTPUT_PARENTS[int(prediction)]
                    if is_offered
                    else None
                ),
                "v033_parent": baseline,
            }
        )
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": plan071.sha256_file(args.config),
        "inputs": EXPECTED_INPUT_SHA256,
        "model_artifact_sha256": plan071.sha256_file(args.model_output),
        "rows": rows,
        "holdout_rows": len(rows),
        "offers": int(np.sum(offered)),
        "coverage": float(np.mean(offered)),
        "abstentions": int(np.sum(~offered)),
        "zero_qualified": int(np.sum(qualified_counts == 0)),
        "one_qualified": int(np.sum(qualified_counts == 1)),
        "multi_qualified": int(np.sum(qualified_counts > 1)),
        "all_predictions_frozen_before_review": True,
        "identity_values_exposed": False,
        "reference_implementation_match": True,
        "frozen_model_byte_match": True,
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
    parser.add_argument("--frozen-model", required=True, type=Path)
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
                "coverage": result["coverage"],
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
