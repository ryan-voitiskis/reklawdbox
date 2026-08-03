#!/usr/bin/env python3
"""Select a frozen model-directed blind Genre Intelligence review batch."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus


EXPERIMENT_ID = "genre-intelligence-truth-v1-b04"
SOURCE_EXPERIMENT_ID = "genre-intelligence-candidate-pool-v1-b04"
EXPECTED_POOL_SHA256 = (
    "78d4679e1945f0e5c687c92cd4fb85e925ed5cf2ad847d9e0a062abd5518d9a7"
)
EXPECTED_POOL_FINGERPRINT = (
    "94bd0521fb29f358c23b3d9b4b8da3039c2cb8b6e0520a5ff78cdcdea3419e4f"
)
QUOTAS = {"Downtempo": 6, "IDM": 10, "Tech House": 4}


def stable_hash(row: dict[str, Any], purpose: str) -> str:
    value = "|".join(
        (
            EXPERIMENT_ID,
            purpose,
            str(row["sampling_stratum_private"]),
            str(row["track_id"]),
            str(row["file_path"]),
        )
    )
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def validate_pool(
    value: dict[str, Any],
    source_sha256: str,
    *,
    expected_sha256: str = EXPECTED_POOL_SHA256,
    expected_fingerprint: str = EXPECTED_POOL_FINGERPRINT,
) -> list[dict[str, Any]]:
    if source_sha256 != expected_sha256:
        raise ValueError("candidate-pool artifact checksum differs")
    if value.get("experiment_id") != SOURCE_EXPERIMENT_ID:
        raise ValueError("unexpected candidate-pool experiment ID")
    rows = value.get("rows")
    if not isinstance(rows, list):
        raise ValueError("candidate pool has no row list")
    if value.get("pool_fingerprint") != expected_fingerprint:
        raise ValueError("candidate-pool fingerprint differs")
    if corpus.fingerprint(rows) != expected_fingerprint:
        raise ValueError("candidate-pool rows do not match their fingerprint")
    return rows


def select_batch(
    rows: list[dict[str, Any]], quotas: dict[str, int] = QUOTAS
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    used_paths: set[str] = set()
    used_artists: set[str] = set()
    used_releases: set[str] = set()
    for stratum, count in sorted(quotas.items()):
        candidates = sorted(
            (
                row
                for row in rows
                if row.get("sampling_stratum_private") == stratum
            ),
            key=lambda row: stable_hash(row, "quota"),
        )
        accepted = 0
        for row in candidates:
            path = str(row["file_path"])
            artist = str(row["artist_group"])
            release = str(row["release_group"])
            if (
                path in used_paths
                or artist in used_artists
                or release in used_releases
            ):
                continue
            selected.append(row)
            used_paths.add(path)
            used_artists.add(artist)
            used_releases.add(release)
            accepted += 1
            if accepted == count:
                break
        if accepted != count:
            raise ValueError(
                f"sampling stratum {stratum!r} produced {accepted} distinct rows; "
                f"required {count}"
            )
    return sorted(selected, key=lambda row: stable_hash(row, "review-order"))


def private_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GI04-{position:02d}",
        "track_id": row["track_id"],
        "file_path": row["file_path"],
        "artist": row["artist"],
        "title": row["title"],
        "album": row.get("album"),
        "artist_group": row["artist_group"],
        "release_group": row["release_group"],
        "sampling_stratum_private": row["sampling_stratum_private"],
        "source_recommendation_private": row["source_recommendation_private"],
        "source_confidence_private": row["source_confidence_private"],
        "source_row_index": row["source_row_index"],
    }


def build_result(pool: dict[str, Any], source_sha256: str) -> dict[str, Any]:
    rows = validate_pool(pool, source_sha256)
    selected = [
        private_row(row, position)
        for position, row in enumerate(select_batch(rows), start=1)
    ]
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "blind_development_truth_review_pending",
        "export_playlist_name": EXPERIMENT_ID.replace("-", "_"),
        "source": {
            "artifact_sha256": source_sha256,
            "experiment_id": SOURCE_EXPERIMENT_ID,
            "pool_fingerprint": EXPECTED_POOL_FINGERPRINT,
            "role": "model_directed_sampling_pool_not_truth",
            "model_recommendation_used_for_sampling": True,
            "model_recommendation_used_as_truth": False,
            "confidence_filter": None,
            "sealed_holdout_excluded": True,
            "prior_operator_reviews_excluded": True,
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(EXPERIMENT_ID.encode()).hexdigest(),
            "fixed_sampling_quotas": QUOTAS,
            "one_per_path": True,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "sampling_and_model_fields_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "roster_sha256": corpus.fingerprint(selected),
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate-pool", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source_sha256 = corpus.sha256_file(args.candidate_pool)
    pool = json.loads(args.candidate_pool.read_text(encoding="utf-8"))
    result = build_result(pool, source_sha256)
    corpus.atomic_write(
        args.output,
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8")
        + b"\n",
    )
    print(
        json.dumps(
            {
                "experiment_id": result["experiment_id"],
                "method_status": result["method_status"],
                "output": str(args.output),
                "roster_sha256": result["roster_sha256"],
                "rows": len(result["selected"]),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
