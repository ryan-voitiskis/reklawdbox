#!/usr/bin/env python3
"""Evaluate the two frozen Plan 066 broad-genre representation candidates."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as broad
import discogs_effnet_kick_broad_eval as kick_eval
import discogs_effnet_supervised_broad_eval as supervised


EXPERIMENT_ID = "permissive-representation-broad-genre-v1"
METHOD_STATUS = "pre_registered_nested_cross_fitted_development_evaluation"
CURRENT_MANIFEST_SHA256 = (
    "1e877734477c25dcd622837bea8c2a0d1ae84f44d4526ef71d4645aa3fe54c3f"
)
CURRENT_CORPUS_FINGERPRINT = (
    "sha256:b88911c7b24bbeecd1d59607ceb5e873ca29ff6f15052e77913635e5832471f1"
)
SOURCE_MANIFEST_SHA256 = (
    "a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9"
)
SOURCE_FEATURE_SHA256 = (
    "5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd"
)
KICK_FEATURE_SHA256 = (
    "0b5842935ddbf09e58321a10dce97811790fd77465246cb4eef27a8e9b9d341e"
)
EXTRACTOR_SOURCE_SHA256 = (
    "5d09431f18e77320a8f0c77b0af393cce4ea8979275cfb6adc2b7b5f44fd7e5c"
)
SUPPORT_SOURCE_SHA256 = {
    "broad": "25bc75ccca3c2122be1ec3037054e8f934b8cd4438b67d9a0166a34d77661558",
    "supervised": "ade670e4b25689e0f3175829300744715e8d15f2803ddf5e63c41f092e2d8ce2",
    "kick": "da2ad49e9797a75c4f5573c5be9e9138708306b218ab59480ab74172df6c5746",
}
REPRESENTATION_INPUTS = {
    "openl3": {
        "feature_sha256": (
            "d9c06b2df65199d98e17277a268e69732e41c7e7b76d6f9e2c82824461b8097c"
        ),
        "summary_sha256": (
            "82951bbc023d49cea1c1ade10d7808da95778397fc871063646e043e65393b09"
        ),
        "ordered_source_sha256": (
            "b4c9a9df9516bd0819ac8f3687f9087814d7bc5995d2ca7bb00c7fc28484d2c4"
        ),
        "model_sha256": (
            "81c24c8a723054717fdea5c7448acb6023baaf70a0fc526deb030c2032db0ed3"
        ),
    },
    "clap": {
        "feature_sha256": (
            "097443ac6ec6f0195ce8904643ec74703b3a81c50ed9d0610213b7674970d59a"
        ),
        "summary_sha256": (
            "ccaf9fbbda54c086faf2b1856b27cf4194e830998635a30e292eb830eb2da745"
        ),
        "ordered_source_sha256": (
            "b4c9a9df9516bd0819ac8f3687f9087814d7bc5995d2ca7bb00c7fc28484d2c4"
        ),
        "model_sha256": (
            "1cd3c601bc4afe0fa87be3de4c13dd2cfadd249fac1e29acf74a9b296c3219bb"
        ),
    },
}
PCA_COMPONENTS = 64


def current_source_indices(
    current_rows: list[dict[str, Any]], source_rows: list[dict[str, Any]]
) -> np.ndarray:
    source_by_path = {
        str(row["file_path"]): index for index, row in enumerate(source_rows)
    }
    if len(source_by_path) != len(source_rows):
        raise ValueError("source manifest contains duplicate paths")
    indices = []
    previous = -1
    for row_index, current in enumerate(current_rows):
        path = str(current["file_path"])
        if path not in source_by_path:
            raise ValueError(f"current row {row_index} is absent from source manifest")
        source_index = source_by_path[path]
        source = source_rows[source_index]
        if source_index <= previous:
            raise ValueError("current manifest is not an ordered source subset")
        if current["truth"] != source["truth"] or int(current["fold"]) != int(
            source["fold"]
        ):
            raise ValueError(f"truth or fold differs at current row {row_index}")
        indices.append(source_index)
        previous = source_index
    return np.asarray(indices, dtype=np.int64)


def augmented_fold_features(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    effnet_embeddings: np.ndarray,
    kick_features: np.ndarray,
    representation_embeddings: np.ndarray,
    train_mask: np.ndarray,
) -> np.ndarray:
    base = kick_eval.augmented_fold_features(
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        train_mask,
    )
    if representation_embeddings.shape[0] != base.shape[0]:
        raise ValueError("representation and base feature row counts differ")
    projected = supervised.pca_projection(
        representation_embeddings, train_mask, PCA_COMPONENTS
    )
    return np.column_stack([base, projected])


def score_partition(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    effnet_embeddings: np.ndarray,
    kick_features: np.ndarray,
    representation_embeddings: np.ndarray,
    truths: np.ndarray,
    train_mask: np.ndarray,
    test_mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    features = augmented_fold_features(
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        representation_embeddings,
        train_mask,
    )
    indices, scores, classes = supervised.ridge_score_split(
        features, truths, train_mask, test_mask
    )
    predictions, margins = supervised.predictions_and_margins(scores, classes)
    return indices, predictions, margins


def nested_cross_fitted_offers(
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    effnet_embeddings: np.ndarray,
    kick_features: np.ndarray,
    representation_embeddings: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray, list[dict[str, Any]]]:
    predictions = np.full(len(truths), -1, dtype=np.int64)
    margins = np.zeros(len(truths), dtype=np.float64)
    offered = np.zeros(len(truths), dtype=bool)
    details: list[dict[str, Any]] = []

    for outer_fold in sorted(int(value) for value in np.unique(folds)):
        outer_train = folds != outer_fold
        outer_test = folds == outer_fold
        inner_predictions = np.full(len(truths), -1, dtype=np.int64)
        inner_margins = np.zeros(len(truths), dtype=np.float64)

        for inner_fold in sorted(int(value) for value in np.unique(folds[outer_train])):
            inner_test = outer_train & (folds == inner_fold)
            inner_train = outer_train & (folds != inner_fold)
            indices, fold_predictions, fold_margins = score_partition(
                style_scores,
                baseline_features,
                arrangement,
                effnet_embeddings,
                kick_features,
                representation_embeddings,
                truths,
                inner_train,
                inner_test,
            )
            inner_predictions[indices] = fold_predictions
            inner_margins[indices] = fold_margins

        if np.any(inner_predictions[outer_train] < 0):
            raise ValueError("nested adapter did not score every training row")
        minimum_offers = max(40, math.ceil(0.10 * int(np.sum(outer_train))))
        selected = broad.choose_threshold(
            inner_margins[outer_train],
            inner_predictions[outer_train] == truths[outer_train],
            minimum_offers,
        )
        indices, fold_predictions, fold_margins = score_partition(
            style_scores,
            baseline_features,
            arrangement,
            effnet_embeddings,
            kick_features,
            representation_embeddings,
            truths,
            outer_train,
            outer_test,
        )
        predictions[indices] = fold_predictions
        margins[indices] = fold_margins

        if selected is None:
            details.append(
                {
                    "fold": outer_fold,
                    "threshold": None,
                    "minimum_inner_offers": minimum_offers,
                    "inner_offers": 0,
                    "inner_coverage": 0.0,
                    "inner_offered_precision": 0.0,
                }
            )
            continue
        threshold = float(selected["threshold"])
        offered[indices] = fold_margins >= threshold
        details.append(
            {
                "fold": outer_fold,
                "threshold": threshold,
                "minimum_inner_offers": minimum_offers,
                "inner_offers": int(selected["offers"]),
                "inner_coverage": broad.safe_fraction(
                    int(selected["offers"]), int(np.sum(outer_train))
                ),
                "inner_offered_precision": float(selected["offered_precision"]),
            }
        )

    if np.any(predictions < 0):
        raise ValueError("adapter did not score every outer held-out row")
    return predictions, margins, offered, details


def minimum_supported_target_precision(metrics: dict[str, Any]) -> float:
    supported = [
        float(values["offered_precision"])
        for values in metrics["per_target"].values()
        if int(values["support"]) >= 10 and int(values["offers"]) >= 5
    ]
    return min(supported) if supported else 0.0


def candidate_tie_key(name: str, candidate: dict[str, Any]) -> tuple[float, ...]:
    metrics = candidate["deployment"]["metrics"]
    return (
        min(float(fold["offered_precision"]) for fold in metrics["folds"]),
        minimum_supported_target_precision(metrics),
        float(metrics["offered_precision"]),
        float(metrics["coverage"]),
        1.0 if name == "openl3" else 0.0,
    )


def evaluate_candidate(
    representation_embeddings: np.ndarray,
    style_scores: np.ndarray,
    baseline_features: np.ndarray,
    arrangement: np.ndarray,
    effnet_embeddings: np.ndarray,
    kick_features: np.ndarray,
    truths: np.ndarray,
    folds: np.ndarray,
) -> dict[str, Any]:
    predictions, margins, offered, thresholds = nested_cross_fitted_offers(
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        representation_embeddings,
        truths,
        folds,
    )
    unselective = broad.metrics(
        truths, predictions, np.ones(len(truths), dtype=bool), folds
    )
    selective = broad.metrics(truths, predictions, offered, folds)
    for detail, fold_metric in zip(thresholds, selective["folds"], strict=True):
        detail["held_out_eligible_rows"] = fold_metric["eligible_rows"]
        detail["held_out_offers"] = fold_metric["offers"]
        detail["held_out_coverage"] = fold_metric["coverage"]
        detail["held_out_offered_precision"] = fold_metric["offered_precision"]
    gate = broad.gate(unselective, selective)
    minimum_deployment_offers = max(60, math.ceil(0.10 * len(truths)))
    calibrated = broad.choose_threshold(
        margins,
        predictions == truths,
        minimum_deployment_offers,
    )
    if calibrated is None:
        deployment_threshold = None
        deployment_offered = np.zeros(len(truths), dtype=bool)
    else:
        deployment_threshold = float(calibrated["threshold"])
        deployment_offered = margins >= deployment_threshold
    deployment_metrics = broad.metrics(
        truths, predictions, deployment_offered, folds
    )
    deployment_gate = broad.gate(unselective, deployment_metrics)
    return {
        "unselective": unselective,
        "selective": selective,
        "fold_thresholds": thresholds,
        "nested_gate": gate,
        "deployment": {
            "minimum_calibration_offers": minimum_deployment_offers,
            "threshold": deployment_threshold,
            "metrics": deployment_metrics,
            "gate": deployment_gate,
        },
        "passed": bool(gate["passed"] and deployment_gate["passed"]),
    }


def load_representation(
    name: str,
    feature_path: Path,
    summary_path: Path,
    expected_rows: int,
) -> tuple[np.ndarray, dict[str, Any]]:
    expected = REPRESENTATION_INPUTS[name]
    feature_sha = broad.sha256_file(feature_path)
    summary_sha = broad.sha256_file(summary_path)
    if feature_sha != expected["feature_sha256"]:
        raise ValueError(f"{name} feature artifact SHA-256 changed")
    if summary_sha != expected["summary_sha256"]:
        raise ValueError(f"{name} summary SHA-256 changed")
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    if summary["ordered_source_sha256"] != expected["ordered_source_sha256"]:
        raise ValueError(f"{name} ordered source SHA-256 changed")
    if summary["model_sha256"] != expected["model_sha256"]:
        raise ValueError(f"{name} model SHA-256 changed")
    if summary["extractor_source_sha256"] != EXTRACTOR_SOURCE_SHA256:
        raise ValueError(f"{name} extractor source SHA-256 changed")
    if int(summary["rows"]) != expected_rows:
        raise ValueError(f"{name} summary row count changed")
    artifact = np.load(feature_path, allow_pickle=False)
    if list(artifact.files) != ["embeddings"]:
        raise ValueError(f"{name} feature artifact arrays changed")
    embeddings = np.asarray(artifact["embeddings"], dtype=np.float64)
    if embeddings.shape != (expected_rows, 512):
        raise ValueError(f"{name} embedding shape {embeddings.shape} is invalid")
    if not np.all(np.isfinite(embeddings)):
        raise ValueError(f"{name} embeddings contain non-finite values")
    return embeddings, {
        "feature_sha256": feature_sha,
        "summary_sha256": summary_sha,
        "ordered_source_sha256": summary["ordered_source_sha256"],
        "model_sha256": summary["model_sha256"],
        "extractor_source_sha256": summary["extractor_source_sha256"],
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    support_sources = {
        "broad": broad.sha256_file(Path(broad.__file__)),
        "supervised": broad.sha256_file(Path(supervised.__file__)),
        "kick": broad.sha256_file(Path(kick_eval.__file__)),
    }
    if support_sources != SUPPORT_SOURCE_SHA256:
        raise ValueError(f"supporting evaluator sources changed: {support_sources}")
    hashes = {
        "current_manifest_sha256": broad.sha256_file(args.current_manifest),
        "source_manifest_sha256": broad.sha256_file(args.source_manifest),
        "source_feature_sha256": broad.sha256_file(args.source_features),
        "kick_feature_sha256": broad.sha256_file(args.kick_features),
    }
    expected_hashes = {
        "current_manifest_sha256": CURRENT_MANIFEST_SHA256,
        "source_manifest_sha256": SOURCE_MANIFEST_SHA256,
        "source_feature_sha256": SOURCE_FEATURE_SHA256,
        "kick_feature_sha256": KICK_FEATURE_SHA256,
    }
    if hashes != expected_hashes:
        raise ValueError(f"input hashes differ from frozen values: {hashes}")
    if broad.broad_semantic_sha256() != broad.EXPECTED_BROAD_SEMANTIC_SHA256:
        raise ValueError("broad taxonomy semantic checksum changed")

    current_manifest = json.loads(args.current_manifest.read_text(encoding="utf-8"))
    source_manifest = json.loads(args.source_manifest.read_text(encoding="utf-8"))
    if current_manifest["corpus_fingerprint"] != CURRENT_CORPUS_FINGERPRINT:
        raise ValueError("current corpus fingerprint changed")
    indices = current_source_indices(
        current_manifest["rows"], source_manifest["rows"]
    )
    source_artifact = np.load(args.source_features, allow_pickle=False)
    all_kick = kick_eval.load_kick_artifact(
        args.kick_features, len(source_manifest["rows"])
    )
    for name in ["style_scores", "embeddings", "arrangement", "truth_indices", "folds"]:
        if name not in source_artifact or len(source_artifact[name]) != len(
            source_manifest["rows"]
        ):
            raise ValueError(f"source feature array {name!r} is missing or misaligned")

    truth_indices = np.asarray(source_artifact["truth_indices"], dtype=np.int64)[
        indices
    ]
    truths = np.asarray(
        [
            broad.BROAD_INDEX[broad.FINE_TO_BROAD[broad.CANONICAL[int(index)]]]
            for index in truth_indices
        ],
        dtype=np.int64,
    )
    style_scores = broad.broad_scores(
        np.asarray(source_artifact["style_scores"], dtype=np.float64)[indices]
    )
    baseline_features = supervised.baseline_broad_one_hot(
        [row["baseline_recommendation"] for row in current_manifest["rows"]]
    )
    arrangement = np.asarray(source_artifact["arrangement"], dtype=np.float64)[
        indices
    ]
    effnet_embeddings = np.asarray(
        source_artifact["embeddings"], dtype=np.float64
    )[indices]
    kick_features = np.asarray(all_kick, dtype=np.float64)[indices]
    folds = np.asarray(source_artifact["folds"], dtype=np.int64)[indices]
    for row_index, row in enumerate(current_manifest["rows"]):
        if broad.CANONICAL[int(truth_indices[row_index])] != row["truth"]:
            raise ValueError(f"truth alignment differs at current row {row_index}")
        if int(folds[row_index]) != int(row["fold"]):
            raise ValueError(f"fold alignment differs at current row {row_index}")

    representation_paths = {
        "openl3": (args.openl3_features, args.openl3_summary),
        "clap": (args.clap_features, args.clap_summary),
    }
    candidates = {}
    representation_inputs = {}
    for name in ["openl3", "clap"]:
        embeddings, frozen_inputs = load_representation(
            name, *representation_paths[name], len(current_manifest["rows"])
        )
        representation_inputs[name] = frozen_inputs
        candidates[name] = evaluate_candidate(
            embeddings,
            style_scores,
            baseline_features,
            arrangement,
            effnet_embeddings,
            kick_features,
            truths,
            folds,
        )

    passing = [name for name, value in candidates.items() if value["passed"]]
    selected = (
        max(passing, key=lambda name: candidate_tie_key(name, candidates[name]))
        if passing
        else None
    )
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "corpus_fingerprint": CURRENT_CORPUS_FINGERPRINT,
        "rows": len(current_manifest["rows"]),
        "broad_targets": len(broad.BROAD_TARGETS),
        "inputs": hashes,
        "evaluation_source_sha256": broad.sha256_file(Path(__file__)),
        "support_source_sha256": support_sources,
        "representation_inputs": representation_inputs,
        "broad_semantic_sha256": broad.EXPECTED_BROAD_SEMANTIC_SHA256,
        "adapter": {
            "base": "Plan 065 kick-augmented supervised broad adapter",
            "additional_feature": "training-partition PCA64 of one frozen representation",
            "ridge_penalty": supervised.RIDGE_PENALTY,
            "confidence": "top ridge score minus second ridge score",
            "threshold_calibration": "nested out-of-fold within each outer training partition",
            "candidate_isolation": "OpenL3 and CLAP are never concatenated",
        },
        "candidates": candidates,
        "selection_tie_break": [
            "minimum outer-fold offered precision",
            "minimum supported-target offered precision",
            "overall offered precision",
            "coverage",
            "OpenL3 runtime preference",
        ],
        "selected_candidate": selected,
        "outcome": (
            "representation_candidate_passed_development_gate"
            if selected is not None
            else "bounded_negative"
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--current-manifest", required=True, type=Path)
    parser.add_argument("--source-manifest", required=True, type=Path)
    parser.add_argument("--source-features", required=True, type=Path)
    parser.add_argument("--kick-features", required=True, type=Path)
    parser.add_argument("--openl3-features", required=True, type=Path)
    parser.add_argument("--openl3-summary", required=True, type=Path)
    parser.add_argument("--clap-features", required=True, type=Path)
    parser.add_argument("--clap-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "rows": result["rows"],
                "candidates": {
                    name: {
                        "offers": value["selective"]["offers"],
                        "coverage": value["selective"]["coverage"],
                        "offered_precision": value["selective"][
                            "offered_precision"
                        ],
                        "accuracy": value["selective"]["accuracy"],
                        "nested_gate": value["nested_gate"],
                        "deployment": {
                            "threshold": value["deployment"]["threshold"],
                            "offers": value["deployment"]["metrics"]["offers"],
                            "coverage": value["deployment"]["metrics"][
                                "coverage"
                            ],
                            "offered_precision": value["deployment"]["metrics"][
                                "offered_precision"
                            ],
                            "gate": value["deployment"]["gate"],
                        },
                    }
                    for name, value in result["candidates"].items()
                },
                "selected_candidate": result["selected_candidate"],
                "outcome": result["outcome"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
