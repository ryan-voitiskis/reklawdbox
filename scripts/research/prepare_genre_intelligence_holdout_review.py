#!/usr/bin/env python3
"""Prepare prediction-blind review batches for the Plan 070 holdout offers."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus


EXPERIMENT_PREFIX = "genre-intelligence-holdout-v1-h"
EXPECTED_HOLDOUT_SHA256 = (
    "1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37"
)
EXPECTED_PREDICTIONS_SHA256 = (
    "4e95f5cb364d2eb4966b5a3d0d1cbcdc2a30843f2b91475a86d4f350a40847b5"
)
EXPECTED_ROWS = 60
EXPECTED_OFFERS = 31
BATCH_SIZE = 6


def stable_order(row_id: str) -> str:
    return hashlib.sha256(
        f"genre-intelligence-holdout-review-v1|{row_id}".encode("utf-8")
    ).hexdigest()


def offered_positions(predictions: dict[str, Any]) -> list[int]:
    rows = predictions.get("rows")
    if not isinstance(rows, list) or len(rows) != EXPECTED_ROWS:
        raise ValueError("sealed prediction row count differs")
    result = []
    for position, row in enumerate(rows, start=1):
        expected_id = f"GIH-{position:03d}"
        if row.get("row_id") != expected_id:
            raise ValueError("sealed prediction identity order differs")
        if bool(row.get("offered")):
            if row.get("suggested_parent") is None:
                raise ValueError("offered prediction has no private suggestion")
            result.append(position)
    if len(result) != EXPECTED_OFFERS or predictions.get("offers") != EXPECTED_OFFERS:
        raise ValueError("sealed prediction offer count differs")
    return result


def review_rows(
    holdout: dict[str, Any], predictions: dict[str, Any]
) -> list[dict[str, Any]]:
    selected = holdout.get("selected")
    if not isinstance(selected, list) or len(selected) != EXPECTED_ROWS:
        raise ValueError("holdout identity row count differs")
    positions = offered_positions(predictions)
    ordered = sorted(positions, key=lambda value: stable_order(f"GIH-{value:03d}"))
    result = []
    for position in ordered:
        source = selected[position - 1]
        if int(source["position"]) != position:
            raise ValueError("holdout identity order differs")
        result.append(
            {
                "track_id": source["track_id"],
                "file_path": source["file_path"],
                "artist": source["artist"],
                "title": source["title"],
                "album": source.get("album"),
                "artist_group": source["artist_group"],
                "release_group": source["release_group"],
                "source_row_id_private": f"GIH-{position:03d}",
            }
        )
    if len({str(row["artist_group"]) for row in result}) != len(result):
        raise ValueError("offered holdout rows contain duplicate artists")
    if len({str(row["release_group"]) for row in result}) != len(result):
        raise ValueError("offered holdout rows contain duplicate releases")
    return result


def batches(rows: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    return [rows[index : index + BATCH_SIZE] for index in range(0, len(rows), BATCH_SIZE)]


def mapping_for_batch(
    rows: list[dict[str, Any]], batch_number: int
) -> dict[str, Any]:
    batch_label = f"H{batch_number:02d}"
    selected = []
    for position, row in enumerate(rows, start=1):
        selected.append(
            {
                "position": position,
                "code": f"GI{batch_label}-{position:02d}",
                **row,
            }
        )
    experiment_id = f"{EXPERIMENT_PREFIX}{batch_number:02d}"
    return {
        "schema_version": 1,
        "experiment_id": experiment_id,
        "method_status": "blind_release_holdout_review_pending",
        "export_playlist_name": experiment_id.replace("-", "_"),
        "source": {
            "holdout_sha256": EXPECTED_HOLDOUT_SHA256,
            "predictions_sha256": EXPECTED_PREDICTIONS_SHA256,
            "offered_rows_only": True,
            "predictions_frozen_before_review": True,
        },
        "selection_rule": {
            "review_order": "sha256 of fixed seed and opaque row ID",
            "batch_size_cap": BATCH_SIZE,
            "maximum_tracks_per_artist": 1,
            "one_per_release_group": True,
            "model_and_sampling_fields_absent": True,
        },
        "roster_sha256": corpus.fingerprint(selected),
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--holdout", required=True, type=Path)
    parser.add_argument("--predictions", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--output-manifest", required=True, type=Path)
    args = parser.parse_args()
    if corpus.sha256_file(args.holdout) != EXPECTED_HOLDOUT_SHA256:
        raise ValueError("holdout artifact SHA-256 differs")
    if corpus.sha256_file(args.predictions) != EXPECTED_PREDICTIONS_SHA256:
        raise ValueError("sealed prediction SHA-256 differs")
    holdout = json.loads(args.holdout.read_text(encoding="utf-8"))
    predictions = json.loads(args.predictions.read_text(encoding="utf-8"))
    rows = review_rows(holdout, predictions)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    records = []
    for batch_number, batch_rows in enumerate(batches(rows), start=1):
        mapping = mapping_for_batch(batch_rows, batch_number)
        path = args.output_dir / f"holdout-review-h{batch_number:02d}-mapping.json"
        corpus.atomic_write(
            path,
            json.dumps(mapping, indent=2, sort_keys=True, ensure_ascii=False).encode(
                "utf-8"
            )
            + b"\n",
        )
        records.append(
            {
                "batch": f"H{batch_number:02d}",
                "rows": len(batch_rows),
                "mapping_sha256": corpus.sha256_file(path),
                "mapping_path": str(path),
            }
        )
    manifest = {
        "schema_version": 1,
        "experiment_id": "genre-intelligence-holdout-review-v1",
        "method_status": "all_offers_partitioned_before_listening",
        "holdout_sha256": EXPECTED_HOLDOUT_SHA256,
        "predictions_sha256": EXPECTED_PREDICTIONS_SHA256,
        "offers": len(rows),
        "batch_size_cap": BATCH_SIZE,
        "batches": records,
        "model_and_sampling_fields_absent_from_mappings": True,
    }
    corpus.atomic_write(
        args.output_manifest,
        json.dumps(manifest, indent=2, sort_keys=True).encode("utf-8") + b"\n",
    )
    print(
        json.dumps(
            {
                "offers": len(rows),
                "batch_size_cap": BATCH_SIZE,
                "batch_rows": [record["rows"] for record in records],
                "output_manifest_sha256": corpus.sha256_file(args.output_manifest),
                "identity_values_exposed": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
