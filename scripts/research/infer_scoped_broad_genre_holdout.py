#!/usr/bin/env python3
"""Fit the selected Plan 067 model and seal scoped holdout predictions."""

from __future__ import annotations

import argparse
import json
import os
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_broad_eval as broad
import discogs_effnet_supervised_broad_eval as supervised
import evaluate_permissive_broad_genre_representations as plan066
import evaluate_scoped_broad_genre_mvp as scoped


EXPERIMENT_ID = "scoped-broad-genre-mvp-holdout-inference-v1"
DEVELOPMENT_RESULT_SHA256 = (
    "baf0045315fd48ad19be92f209402e75d0af84815aa6a90bb8bf7b637ceaeea9"
)
HOLDOUT_MANIFEST_SHA256 = (
    "a2dc2ee0138792a167f81acad251fa59335e9b5d66ed7124302b3fd2cafa9da9"
)
HOLDOUT_BASE_FEATURE_SHA256 = (
    "c456c044883286c9c7f72ebca02c3b8bc5c07e620cdd135c77a4220cd8b92120"
)
HOLDOUT_BASE_SUMMARY_SHA256 = (
    "b735d57333e0ae1118c7000fdf6532880328f39798284313d8d778f9204bbff1"
)
HOLDOUT_OPENL3_FEATURE_SHA256 = (
    "45dcd030c73bd236dd8aab772ab034af0613f8bc537f1720f4f18b1408c7efe9"
)
HOLDOUT_OPENL3_SUMMARY_SHA256 = (
    "ebf7914ee12121f523a1400505f5bf5e0eb1128e7c07074f8a0f8e37a3523471"
)
HOLDOUT_ORDERED_SOURCE_SHA256 = (
    "177b6eee2e42c466f1f2142564754f30f3730b3dc4b01629a84bf671252e6b67"
)
DEPLOYMENT_THRESHOLD = 0.25702530873209417


def validate_hash(path: Path, expected: str, label: str) -> str:
    observed = broad.sha256_file(path)
    if observed != expected:
        raise ValueError(f"{label} SHA-256 changed")
    return observed


def load_holdout_features(
    manifest_path: Path,
    base_feature_path: Path,
    base_summary_path: Path,
    openl3_feature_path: Path,
    openl3_summary_path: Path,
) -> tuple[list[str], dict[str, np.ndarray], dict[str, str]]:
    hashes = {
        "holdout_manifest_sha256": validate_hash(
            manifest_path, HOLDOUT_MANIFEST_SHA256, "holdout manifest"
        ),
        "holdout_base_feature_sha256": validate_hash(
            base_feature_path,
            HOLDOUT_BASE_FEATURE_SHA256,
            "holdout base features",
        ),
        "holdout_base_summary_sha256": validate_hash(
            base_summary_path,
            HOLDOUT_BASE_SUMMARY_SHA256,
            "holdout base summary",
        ),
        "holdout_openl3_feature_sha256": validate_hash(
            openl3_feature_path,
            HOLDOUT_OPENL3_FEATURE_SHA256,
            "holdout OpenL3 features",
        ),
        "holdout_openl3_summary_sha256": validate_hash(
            openl3_summary_path,
            HOLDOUT_OPENL3_SUMMARY_SHA256,
            "holdout OpenL3 summary",
        ),
    }
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    rows = manifest["rows"]
    codes = [str(row["code"]) for row in rows]
    if len(rows) != 48 or len(set(codes)) != 48:
        raise ValueError("holdout manifest row or code count changed")
    summary = json.loads(openl3_summary_path.read_text(encoding="utf-8"))
    if (
        summary["ordered_source_sha256"] != HOLDOUT_ORDERED_SOURCE_SHA256
        or int(summary["rows"]) != 48
        or summary["model_sha256"]
        != plan066.REPRESENTATION_INPUTS["openl3"]["model_sha256"]
        or summary["extractor_source_sha256"] != plan066.EXTRACTOR_SOURCE_SHA256
    ):
        raise ValueError("holdout OpenL3 summary semantics changed")

    base = np.load(base_feature_path, allow_pickle=False)
    expected = {
        "style_scores": (48, 53),
        "embeddings": (48, 1280),
        "arrangement": (48, 4),
        "baseline_features": (48, len(broad.BROAD_TARGETS)),
        "kick_features": (48, 74),
    }
    if set(base.files) != set(expected):
        raise ValueError("holdout base feature arrays changed")
    features = {
        name: np.asarray(base[name], dtype=np.float64) for name in expected
    }
    for name, shape in expected.items():
        if features[name].shape != shape or not np.all(np.isfinite(features[name])):
            raise ValueError(f"holdout feature {name!r} is malformed")
    openl3 = np.load(openl3_feature_path, allow_pickle=False)
    if list(openl3.files) != ["embeddings"]:
        raise ValueError("holdout OpenL3 arrays changed")
    representation = np.asarray(openl3["embeddings"], dtype=np.float64)
    if representation.shape != (48, 512) or not np.all(
        np.isfinite(representation)
    ):
        raise ValueError("holdout OpenL3 embeddings are malformed")
    features["openl3"] = representation
    return codes, features, hashes


def offered_predictions(
    predictions: np.ndarray, margins: np.ndarray
) -> np.ndarray:
    allowed = np.asarray(
        [broad.BROAD_INDEX[target] for target in scoped.ALLOWLIST], dtype=np.int64
    )
    return (margins >= DEPLOYMENT_THRESHOLD) & np.isin(predictions, allowed)


def run(args: argparse.Namespace) -> dict[str, Any]:
    validate_hash(
        args.development_result,
        DEVELOPMENT_RESULT_SHA256,
        "scoped development result",
    )
    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    if development_result["selected_candidate"] != "openl3":
        raise ValueError("scoped development result no longer selects OpenL3")
    (
        _,
        development_style,
        development_baseline,
        development_arrangement,
        development_effnet,
        development_kick,
        development_truths,
        _,
    ) = scoped.development_arrays(args)
    development_openl3, _ = plan066.load_representation(
        "openl3",
        args.openl3_features,
        args.openl3_summary,
        len(development_truths),
    )
    codes, holdout, holdout_hashes = load_holdout_features(
        args.holdout_manifest,
        args.holdout_base_features,
        args.holdout_base_summary,
        args.holdout_openl3_features,
        args.holdout_openl3_summary,
    )

    development_rows = len(development_truths)
    holdout_rows = len(codes)
    train_mask = np.asarray(
        [True] * development_rows + [False] * holdout_rows, dtype=bool
    )
    test_mask = ~train_mask
    style_scores = np.vstack(
        [development_style, broad.broad_scores(holdout["style_scores"])]
    )
    baseline_features = np.vstack(
        [development_baseline, holdout["baseline_features"]]
    )
    arrangement = np.vstack(
        [development_arrangement, holdout["arrangement"]]
    )
    effnet_embeddings = np.vstack(
        [development_effnet, holdout["embeddings"]]
    )
    kick_features = np.vstack([development_kick, holdout["kick_features"]])
    openl3_embeddings = np.vstack([development_openl3, holdout["openl3"]])
    truths = np.concatenate(
        [development_truths, np.full(holdout_rows, -1, dtype=np.int64)]
    )
    features = plan066.augmented_fold_features(
        style_scores,
        baseline_features,
        arrangement,
        effnet_embeddings,
        kick_features,
        openl3_embeddings,
        train_mask,
    )
    indices, scores, classes = supervised.ridge_score_split(
        features, truths, train_mask, test_mask
    )
    if not np.array_equal(
        indices, np.arange(development_rows, development_rows + holdout_rows)
    ):
        raise ValueError("holdout score order changed")
    predictions, margins = supervised.predictions_and_margins(scores, classes)
    offered = offered_predictions(predictions, margins)
    rows = [
        {
            "code": code,
            "prediction": broad.BROAD_TARGETS[int(prediction)],
            "margin": float(margin),
            "offered": bool(is_offered),
        }
        for code, prediction, margin, is_offered in zip(
            codes, predictions, margins, offered, strict=True
        )
    ]
    offered_counts = Counter(
        row["prediction"] for row in rows if bool(row["offered"])
    )
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "sealed_before_blind_listening",
        "selected_candidate": "openl3",
        "development_rows": development_rows,
        "holdout_rows": holdout_rows,
        "deployment_threshold": DEPLOYMENT_THRESHOLD,
        "allowlist": list(scoped.ALLOWLIST),
        "model_source_sha256": broad.sha256_file(Path(__file__)),
        "development_result_sha256": DEVELOPMENT_RESULT_SHA256,
        "holdout_inputs": holdout_hashes,
        "offers": int(np.sum(offered)),
        "abstentions": int(np.sum(~offered)),
        "offer_coverage": broad.safe_fraction(int(np.sum(offered)), holdout_rows),
        "offered_prediction_counts": dict(sorted(offered_counts.items())),
        "rows": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--development-result", required=True, type=Path)
    parser.add_argument("--current-manifest", required=True, type=Path)
    parser.add_argument("--source-manifest", required=True, type=Path)
    parser.add_argument("--source-features", required=True, type=Path)
    parser.add_argument("--kick-features", required=True, type=Path)
    parser.add_argument("--openl3-features", required=True, type=Path)
    parser.add_argument("--openl3-summary", required=True, type=Path)
    parser.add_argument("--clap-features", required=True, type=Path)
    parser.add_argument("--clap-summary", required=True, type=Path)
    parser.add_argument("--holdout-manifest", required=True, type=Path)
    parser.add_argument("--holdout-base-features", required=True, type=Path)
    parser.add_argument("--holdout-base-summary", required=True, type=Path)
    parser.add_argument("--holdout-openl3-features", required=True, type=Path)
    parser.add_argument("--holdout-openl3-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "selected_candidate": result["selected_candidate"],
                "holdout_rows": result["holdout_rows"],
                "offers": result["offers"],
                "abstentions": result["abstentions"],
                "offer_coverage": result["offer_coverage"],
                "offered_prediction_counts": result["offered_prediction_counts"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
