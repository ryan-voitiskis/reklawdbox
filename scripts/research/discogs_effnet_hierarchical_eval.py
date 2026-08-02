#!/usr/bin/env python3
"""Evaluate the frozen Plan 057 family-router candidate offline."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_genre_eval as base


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def hierarchical_predictions(
    effnet_predictions: list[str], baseline_predictions: list[str | None]
) -> list[str]:
    if len(effnet_predictions) != len(baseline_predictions):
        raise ValueError("prediction vectors differ in length")
    return [
        baseline
        if baseline is not None and base.family(baseline) == base.family(effnet)
        else effnet
        for effnet, baseline in zip(
            effnet_predictions, baseline_predictions, strict=True
        )
    ]


def development_gate(
    metrics: dict[str, Any], baseline: dict[str, Any]
) -> dict[str, Any]:
    baseline_folds = {row["fold"]: row for row in baseline["folds"]}
    every_fold_improves = all(
        row["macro_f1"] > baseline_folds[row["fold"]]["macro_f1"]
        for row in metrics["folds"]
    )
    recall_losses = []
    for genre, baseline_genre in baseline["per_genre"].items():
        if baseline_genre["support"] < 10:
            continue
        if metrics["per_genre"][genre]["recall"] < baseline_genre["recall"] - 0.10 - 1e-12:
            recall_losses.append(genre)
    measured_failure_genres = {}
    for genre in ["Breakbeat", "Deep Techno", "Electro"]:
        loss = baseline["per_genre"][genre]["recall"] - metrics["per_genre"][genre]["recall"]
        measured_failure_genres[genre] = {
            "recall_loss": loss,
            "passed": loss <= 0.10 + 1e-12,
        }
    checks: dict[str, Any] = {
        "exact_accuracy_at_least_0_60": metrics["exact_accuracy"] >= 0.60 - 1e-12,
        "macro_recall_at_least_0_45": metrics["macro_recall"] >= 0.45 - 1e-12,
        "macro_f1_at_least_0_40": metrics["macro_f1"] >= 0.40 - 1e-12,
        "same_family_accuracy_at_least_0_78": metrics["same_family_accuracy"]
        >= 0.78 - 1e-12,
        "every_fold_macro_f1_improves": every_fold_improves,
        "per_genre_recall_non_regression": not recall_losses,
        "genres_losing_more_than_0_10_recall": recall_losses,
        "measured_failure_genres": measured_failure_genres,
    }
    boolean_checks = [
        value
        for key, value in checks.items()
        if key not in {"genres_losing_more_than_0_10_recall", "measured_failure_genres"}
    ]
    checks["passed"] = all(boolean_checks) and all(
        row["passed"] for row in measured_failure_genres.values()
    )
    return checks


def run(args: argparse.Namespace) -> dict[str, Any]:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    stage_a = json.loads(args.stage_a.read_text(encoding="utf-8"))
    stage_b = json.loads(args.stage_b.read_text(encoding="utf-8"))
    if manifest["corpus_fingerprint"] != stage_a["corpus_fingerprint"]:
        raise ValueError("manifest and Stage A corpus fingerprints differ")
    if manifest["corpus_fingerprint"] != stage_b["corpus_fingerprint"]:
        raise ValueError("manifest and Stage B corpus fingerprints differ")
    feature_sha = sha256_file(args.feature_artifact)
    if feature_sha != stage_b["feature_artifact"]["sha256"]:
        raise ValueError("feature artifact SHA-256 differs from Stage B result")

    rows = manifest["rows"]
    features = np.load(args.feature_artifact, allow_pickle=False)
    style_scores = features["style_scores"]
    folds = features["folds"].astype(np.int64)
    if style_scores.shape != (len(rows), len(base.CANONICAL)):
        raise ValueError("style score matrix shape differs from manifest")
    if len(folds) != len(rows):
        raise ValueError("fold vector length differs from manifest")
    manifest_folds = np.asarray([row["fold"] for row in rows], dtype=np.int64)
    if not np.array_equal(folds, manifest_folds):
        raise ValueError("feature and manifest fold assignments differ")

    truths = [row["truth"] for row in rows]
    baseline_predictions = [row["baseline_recommendation"] for row in rows]
    invalid_baseline = sorted(
        {
            prediction
            for prediction in baseline_predictions
            if prediction is not None and prediction not in base.CANONICAL
        }
    )
    if invalid_baseline:
        raise ValueError(f"unknown baseline predictions: {invalid_baseline}")

    effnet_predictions = base.predictions_from_style(style_scores)
    direct_metrics = base.aggregate_metrics(truths, effnet_predictions, folds)
    recorded_direct = stage_b["configurations"]["style_projection"]["metrics"]
    for key in ["exact_accuracy", "macro_recall", "macro_f1", "same_family_accuracy"]:
        if abs(direct_metrics[key] - recorded_direct[key]) > 1e-12:
            raise ValueError(f"direct style metric {key} differs from Stage B result")

    predictions = hierarchical_predictions(effnet_predictions, baseline_predictions)
    metrics = base.aggregate_metrics(truths, predictions, folds)
    gate = development_gate(metrics, stage_a["baseline"])
    return {
        "experiment_id": "discogs-effnet-hierarchical-router-v1",
        "method_status": "post_hoc_grouped_development_evaluation",
        "rows": len(rows),
        "fold_count": manifest["fold_count"],
        "candidate": {
            "rule": "retain baseline fine label only when its taxonomy family equals the direct Discogs-EffNet prediction family; otherwise use the Discogs-EffNet label",
            "genre_specific_overrides": 0,
            "thresholds": 0,
        },
        "inputs": {
            "feature_artifact_sha256": feature_sha,
            "stage_a_result_sha256": sha256_file(args.stage_a),
            "stage_b_result_sha256": sha256_file(args.stage_b),
        },
        "baseline": {
            key: stage_a["baseline"][key]
            for key in [
                "support",
                "exact_accuracy",
                "macro_recall",
                "macro_f1",
                "same_family_accuracy",
                "per_genre",
                "folds",
            ]
        },
        "direct_style_projection": {
            key: direct_metrics[key]
            for key in [
                "exact_accuracy",
                "macro_recall",
                "macro_f1",
                "same_family_accuracy",
            ]
        },
        "hierarchical_router": metrics,
        "gate": gate,
        "outcome": "development_candidate_for_new_holdout"
        if gate["passed"]
        else "bounded_negative",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--stage-a", required=True, type=Path)
    parser.add_argument("--stage-b", required=True, type=Path)
    parser.add_argument("--feature-artifact", required=True, type=Path)
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
                "outcome": result["outcome"],
                "metrics": {
                    key: result["hierarchical_router"][key]
                    for key in [
                        "exact_accuracy",
                        "macro_recall",
                        "macro_f1",
                        "same_family_accuracy",
                    ]
                },
                "gate": result["gate"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
