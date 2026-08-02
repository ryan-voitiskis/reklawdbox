#!/usr/bin/env python3
"""Evaluate the two frozen Plan 067 scoped broad-root candidates."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as broad
import discogs_effnet_kick_broad_eval as kick_eval
import discogs_effnet_supervised_broad_eval as supervised
import evaluate_permissive_broad_genre_representations as plan066


EXPERIMENT_ID = "scoped-broad-genre-mvp-v1"
METHOD_STATUS = "post_development_scope_independent_holdout_required"
PLAN066_EVALUATOR_SHA256 = (
    "4bb3e1c21bc5a7a95fe172b92657f319fdf4fff1930b49890b16c2ede67ed770"
)
PLAN066_RESULT_SHA256 = (
    "265df46ff10873355f933b0347229b8c2c282de0f3745c07be08e6599fc433dd"
)
ALLOWLIST = ("Ambient", "House", "Techno")
DEPLOYMENT_THRESHOLDS = {
    "openl3": 0.25702530873209417,
    "clap": 0.18173795988087316,
}


def json_sha256(value: Any) -> str:
    payload = json.dumps(value, indent=2, sort_keys=True) + "\n"
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def scoped_offer_mask(
    predictions: np.ndarray,
    threshold_offers: np.ndarray,
) -> np.ndarray:
    allowed = np.asarray(
        [broad.BROAD_INDEX[target] for target in ALLOWLIST], dtype=np.int64
    )
    return np.asarray(threshold_offers, dtype=bool) & np.isin(predictions, allowed)


def scoped_gate(
    unselective_all_targets: dict[str, Any], candidate: dict[str, Any]
) -> dict[str, Any]:
    insufficient_offers = [
        target for target in ALLOWLIST if candidate["per_target"][target]["offers"] < 5
    ]
    precision_failures = [
        target
        for target in ALLOWLIST
        if candidate["per_target"][target]["offers"] >= 5
        and candidate["per_target"][target]["offered_precision"] < 0.85 - 1e-12
    ]
    checks = {
        "offered_precision_at_least_0_90": candidate["offered_precision"]
        >= 0.90 - 1e-12,
        "coverage_at_least_0_40": candidate["coverage"] >= 0.40 - 1e-12,
        "every_fold_precision_at_least_0_85": all(
            row["offers"] > 0 and row["offered_precision"] >= 0.85 - 1e-12
            for row in candidate["folds"]
        ),
        "every_allowlisted_target_has_five_offers": not insufficient_offers,
        "allowlisted_target_precision_at_least_0_85": not precision_failures,
        "precision_improvement_at_least_0_10": candidate["offered_precision"]
        >= unselective_all_targets["offered_precision"] + 0.10 - 1e-12,
        "insufficient_offer_targets": insufficient_offers,
        "precision_failure_targets": precision_failures,
    }
    checks["passed"] = all(
        value
        for key, value in checks.items()
        if key not in {"insufficient_offer_targets", "precision_failure_targets"}
    )
    return checks


def development_arrays(
    args: argparse.Namespace,
) -> tuple[
    dict[str, Any],
    np.ndarray,
    np.ndarray,
    np.ndarray,
    np.ndarray,
    np.ndarray,
    np.ndarray,
    np.ndarray,
]:
    source_result = plan066.run(args)
    if json_sha256(source_result) != PLAN066_RESULT_SHA256:
        raise ValueError("Plan 066 aggregate replay changed")

    current_manifest = json.loads(args.current_manifest.read_text(encoding="utf-8"))
    source_manifest = json.loads(args.source_manifest.read_text(encoding="utf-8"))
    indices = plan066.current_source_indices(
        current_manifest["rows"], source_manifest["rows"]
    )
    source_artifact = np.load(args.source_features, allow_pickle=False)
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
        [
            source_manifest["rows"][int(index)]["baseline_recommendation"]
            for index in indices
        ]
    )
    arrangement = np.asarray(source_artifact["arrangement"], dtype=np.float64)[
        indices
    ]
    effnet_embeddings = np.asarray(
        source_artifact["embeddings"], dtype=np.float64
    )[indices]
    kick_features = kick_eval.load_kick_artifact(
        args.kick_features, len(source_manifest["rows"])
    )[indices]
    folds = np.asarray(source_artifact["folds"], dtype=np.int64)[indices]
    return (
        source_result,
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        truths,
        folds,
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
    deployment_threshold: float,
) -> dict[str, Any]:
    predictions, margins, nested_offers, fold_thresholds = (
        plan066.nested_cross_fitted_offers(
            style_scores,
            baseline_features,
            arrangement,
            effnet_embeddings,
            kick_features,
            representation_embeddings,
            truths,
            folds,
        )
    )
    all_rows = np.ones(len(truths), dtype=bool)
    unselective_all = broad.metrics(truths, predictions, all_rows, folds)
    unthresholded_scoped = broad.metrics(
        truths,
        predictions,
        scoped_offer_mask(predictions, all_rows),
        folds,
    )
    nested_scoped = broad.metrics(
        truths,
        predictions,
        scoped_offer_mask(predictions, nested_offers),
        folds,
    )
    deployment_scoped = broad.metrics(
        truths,
        predictions,
        scoped_offer_mask(predictions, margins >= deployment_threshold),
        folds,
    )
    nested_gate = scoped_gate(unselective_all, nested_scoped)
    deployment_gate = scoped_gate(unselective_all, deployment_scoped)
    return {
        "unselective_all_targets": unselective_all,
        "unthresholded_scoped": unthresholded_scoped,
        "nested_scoped": nested_scoped,
        "fold_thresholds": fold_thresholds,
        "nested_gate": nested_gate,
        "deployment": {
            "threshold": deployment_threshold,
            "metrics": deployment_scoped,
            "gate": deployment_gate,
        },
        "passed": bool(nested_gate["passed"] and deployment_gate["passed"]),
    }


def select_candidate(candidates: dict[str, dict[str, Any]]) -> str | None:
    if candidates["openl3"]["passed"]:
        return "openl3"
    if candidates["clap"]["passed"]:
        return "clap"
    return None


def run(args: argparse.Namespace) -> dict[str, Any]:
    actual_source_sha = broad.sha256_file(Path(plan066.__file__))
    if actual_source_sha != PLAN066_EVALUATOR_SHA256:
        raise ValueError("Plan 066 evaluator source changed")
    (
        source_result,
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        truths,
        folds,
    ) = development_arrays(args)

    representation_paths = {
        "openl3": (args.openl3_features, args.openl3_summary),
        "clap": (args.clap_features, args.clap_summary),
    }
    candidates = {}
    for name in ["openl3", "clap"]:
        embeddings, _ = plan066.load_representation(
            name, *representation_paths[name], len(truths)
        )
        candidates[name] = evaluate_candidate(
            embeddings,
            style_scores,
            baseline_features,
            arrangement,
            effnet_embeddings,
            kick_features,
            truths,
            folds,
            DEPLOYMENT_THRESHOLDS[name],
        )

    selected = select_candidate(candidates)
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "rows": len(truths),
        "allowlist": list(ALLOWLIST),
        "allowlist_semantic_sha256": hashlib.sha256(
            ("scoped-broad-roots-v1\n" + "\n".join(ALLOWLIST)).encode("utf-8")
        ).hexdigest(),
        "plan066_result_sha256": PLAN066_RESULT_SHA256,
        "plan066_evaluator_sha256": actual_source_sha,
        "plan066_inputs": source_result["inputs"],
        "candidate_priority": ["openl3", "clap"],
        "candidates": candidates,
        "selected_candidate": selected,
        "outcome": (
            "scoped_candidate_passed_development_gate"
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
                "candidates": {
                    name: {
                        "nested": {
                            key: value["nested_scoped"][key]
                            for key in ["offers", "coverage", "offered_precision"]
                        },
                        "nested_gate": value["nested_gate"],
                        "deployment": {
                            "offers": value["deployment"]["metrics"]["offers"],
                            "coverage": value["deployment"]["metrics"]["coverage"],
                            "offered_precision": value["deployment"]["metrics"][
                                "offered_precision"
                            ],
                            "gate": value["deployment"]["gate"],
                        },
                        "passed": value["passed"],
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
