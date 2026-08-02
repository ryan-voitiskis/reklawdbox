#!/usr/bin/env python3
"""Prepare label-blind Plan 067 holdout inputs from frozen local artifacts."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_supervised_broad_eval as supervised
import extract_kick_rhythm_features as kick


EXPERIMENT_ID = "scoped-broad-genre-mvp-holdout-inputs-v1"
HOLDOUT_SHA256 = (
    "7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f"
)
AUDIT_MANIFEST_SHA256 = (
    "cea1520a6bd930250f032629732f8b53edf4143bfd1b4aabe9d315eb588105be"
)
AUDIT_FEATURE_SHA256 = (
    "49c5e57aea256cb9a721d9a4215410511e725aaa2f3b8abcfbcb2b10308ca9a1"
)
ROSTER_SHA256 = "9cf4cdbd67bc701063d886e991e7f4f57a0b675844423584b3027f0bce5418a9"


def audit_indices(
    selected: list[dict[str, Any]], audit_rows: list[dict[str, Any]]
) -> np.ndarray:
    by_path: dict[str, tuple[int, dict[str, Any]]] = {}
    for index, row in enumerate(audit_rows):
        path = str(row["file_path"])
        if path in by_path:
            raise ValueError("audit manifest contains duplicate paths")
        by_path[path] = (index, row)
    indices = []
    seen_paths: set[str] = set()
    for position, row in enumerate(selected, start=1):
        path = str(row["file_path"])
        if path in seen_paths:
            raise ValueError("holdout contains duplicate paths")
        seen_paths.add(path)
        if int(row["position"]) != position:
            raise ValueError("holdout positions are not contiguous")
        if path not in by_path:
            raise ValueError(f"holdout row {position} is absent from audit manifest")
        index, audit = by_path[path]
        if str(row["track_id"]) != str(audit["track_id"]):
            raise ValueError(f"track identity differs at holdout row {position}")
        indices.append(index)
    return np.asarray(indices, dtype=np.int64)


def representation_manifest(
    selected: list[dict[str, Any]], holdout_sha256: str
) -> dict[str, Any]:
    return {
        "experiment_id": EXPERIMENT_ID,
        "stage": "label_blind_holdout_representation_input",
        "corpus_fingerprint": f"sha256:{holdout_sha256}",
        "rows": [
            {
                "row_index": index,
                "code": str(row["code"]),
                "file_path": str(row["file_path"]),
            }
            for index, row in enumerate(selected)
        ],
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    observed_hashes = {
        "holdout_sha256": kick.sha256_file(args.holdout),
        "audit_manifest_sha256": kick.sha256_file(args.audit_manifest),
        "audit_feature_sha256": kick.sha256_file(args.audit_features),
    }
    expected_hashes = {
        "holdout_sha256": HOLDOUT_SHA256,
        "audit_manifest_sha256": AUDIT_MANIFEST_SHA256,
        "audit_feature_sha256": AUDIT_FEATURE_SHA256,
    }
    if observed_hashes != expected_hashes:
        raise ValueError(f"frozen holdout inputs changed: {observed_hashes}")

    holdout = json.loads(args.holdout.read_text(encoding="utf-8"))
    if holdout["roster_sha256"] != ROSTER_SHA256:
        raise ValueError("holdout roster SHA-256 changed")
    selected = holdout["selected"]
    if len(selected) != 48:
        raise ValueError("holdout must contain exactly 48 rows")
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    indices = audit_indices(selected, audit["rows"])
    source = np.load(args.audit_features, allow_pickle=False)
    required = {
        "style_scores": (len(audit["rows"]), 53),
        "embeddings": (len(audit["rows"]), 1280),
        "arrangement": (len(audit["rows"]), 4),
    }
    for name, shape in required.items():
        if name not in source or source[name].shape != shape:
            raise ValueError(f"audit feature {name!r} shape changed")

    kick_features, kick_summary = kick.extract_rows(args.analysis_database, selected)
    baseline_features = supervised.baseline_broad_one_hot(
        [audit["rows"][int(index)]["baseline_recommendation"] for index in indices]
    )
    args.output_manifest.write_text(
        json.dumps(
            representation_manifest(selected, observed_hashes["holdout_sha256"]),
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )
    np.savez_compressed(
        args.output_features,
        style_scores=np.asarray(source["style_scores"])[indices],
        embeddings=np.asarray(source["embeddings"])[indices],
        arrangement=np.asarray(source["arrangement"])[indices],
        baseline_features=baseline_features,
        kick_features=kick_features,
    )
    summary = {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "frozen_label_blind_holdout_input_preparation",
        "rows": len(selected),
        "inputs": observed_hashes,
        "roster_sha256": ROSTER_SHA256,
        "representation_manifest_sha256": kick.sha256_file(args.output_manifest),
        "base_feature_artifact_sha256": kick.sha256_file(args.output_features),
        "base_feature_shapes": {
            "style_scores": [len(selected), 53],
            "embeddings": [len(selected), 1280],
            "arrangement": [len(selected), 4],
            "baseline_features": [
                len(selected),
                int(baseline_features.shape[1]),
            ],
            "kick_features": [len(selected), kick.FEATURE_COUNT],
        },
        "kick": kick_summary,
    }
    args.output_summary.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    for path in [
        args.output_manifest,
        args.output_features,
        args.output_summary,
    ]:
        os.chmod(path, 0o600)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--holdout", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--audit-features", required=True, type=Path)
    parser.add_argument("--analysis-database", required=True, type=Path)
    parser.add_argument("--output-manifest", required=True, type=Path)
    parser.add_argument("--output-features", required=True, type=Path)
    parser.add_argument("--output-summary", required=True, type=Path)
    args = parser.parse_args()
    for path in [
        args.output_manifest,
        args.output_features,
        args.output_summary,
    ]:
        path.parent.mkdir(parents=True, exist_ok=True)
    print(json.dumps(run(args), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
