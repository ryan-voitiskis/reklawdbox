#!/usr/bin/env python3
"""Verify decoded-audio isolation for the powered Plan 073 holdout."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import evaluate_genre_intelligence_open_set as evaluation
import verify_genre_intelligence_open_set_holdout_audio as audio


EXPERIMENT_ID = "genre-intelligence-precision-first-holdout-v1"
EXPECTED_REFERENCE_MANIFESTS = [
    {
        "name": "development",
        "sha256": (
            "6bf80b80f060649877a90a5d6dfa8188c9549eaa0986f1667d611e115689b682"
        ),
        "rows": 716,
    },
    {
        "name": "first_consumed_holdout",
        "sha256": (
            "d970b54a4da4c370e9a5a21524393c4845c6b70ed5a09276540f09a4dbb0c152"
        ),
        "rows": 60,
    },
    {
        "name": "second_consumed_holdout",
        "sha256": (
            "1629f8db56e0c84b9900e8fe0ea6656d38dcdfbd8e3cc6d80585abf650e3a0d2"
        ),
        "rows": 60,
    },
]
EXPECTED_HOLDOUT_MANIFEST_SHA256 = (
    "a324393657531919df7b143b400c329e5b74288aebdb0eaae4871457169f9380"
)
EXPECTED_HOLDOUT_ROWS = 150


def audit_partitions(
    reference_hashes: list[tuple[str, list[str]]], holdout_hashes: list[str]
) -> dict[str, Any]:
    reference_values = [
        value for _, values in reference_hashes for value in values
    ]
    reference_set = set(reference_values)
    holdout_set = set(holdout_hashes)
    per_reference = {
        name: {
            "rows": len(values),
            "unique_decoded_audio": len(set(values)),
            "ordered_hash_sha256": audio.ordered_digest(values),
            "holdout_overlap": len(set(values) & holdout_set),
        }
        for name, values in reference_hashes
    }
    return {
        "reference_rows": len(reference_values),
        "reference_unique_decoded_audio": len(reference_set),
        "reference_ordered_hash_sha256": audio.ordered_digest(reference_values),
        "holdout_rows": len(holdout_hashes),
        "holdout_unique_decoded_audio": len(holdout_set),
        "holdout_ordered_hash_sha256": audio.ordered_digest(holdout_hashes),
        "cross_partition_decoded_audio_overlap": len(reference_set & holdout_set),
        "per_reference": per_reference,
        "passed": (
            len(holdout_hashes) == len(holdout_set)
            and not (reference_set & holdout_set)
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--reference-manifest", required=True, action="append", type=Path
    )
    parser.add_argument("--holdout-manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--workers", type=int, default=12)
    args = parser.parse_args()
    if len(args.reference_manifest) != len(EXPECTED_REFERENCE_MANIFESTS):
        raise ValueError("decoded-audio reference manifest count differs")
    reference_paths = []
    for path, expected in zip(
        args.reference_manifest, EXPECTED_REFERENCE_MANIFESTS, strict=True
    ):
        reference_paths.append(
            (
                str(expected["name"]),
                audio.manifest_paths(
                    path, str(expected["sha256"]), int(expected["rows"])
                ),
            )
        )
    holdout_paths = audio.manifest_paths(
        args.holdout_manifest,
        EXPECTED_HOLDOUT_MANIFEST_SHA256,
        EXPECTED_HOLDOUT_ROWS,
    )
    all_reference_paths = [
        value for _, values in reference_paths for value in values
    ]
    if set(all_reference_paths) & set(holdout_paths):
        raise ValueError("decoded-audio manifests contain an exact path overlap")
    reference_hashes = []
    offset = 0
    combined_hashes = audio.hash_paths(
        all_reference_paths, args.ffmpeg, args.workers
    )
    for name, paths in reference_paths:
        count = len(paths)
        reference_hashes.append((name, combined_hashes[offset : offset + count]))
        offset += count
    holdout_hashes = audio.hash_paths(holdout_paths, args.ffmpeg, args.workers)
    audit = audit_partitions(reference_hashes, holdout_hashes)
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": "private_decoded_audio_multi_partition_audit",
        "reference_manifests": [
            {
                "name": expected["name"],
                "sha256": expected["sha256"],
                "rows": expected["rows"],
            }
            for expected in EXPECTED_REFERENCE_MANIFESTS
        ],
        "holdout_manifest_sha256": EXPECTED_HOLDOUT_MANIFEST_SHA256,
        "auditor_source_sha256": evaluation.sha256_file(Path(__file__)),
        "decoder": {
            "ffmpeg": args.ffmpeg,
            "sample_rate": 48000,
            "channels": 1,
            "hash": "sha256",
        },
        **audit,
        "identity_values_exposed": False,
    }
    evaluation.atomic_write(args.output, result)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
