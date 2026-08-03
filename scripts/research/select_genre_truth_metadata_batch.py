#!/usr/bin/env python3
"""Select a deterministic metadata-directed blind genre-truth batch."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus


EXPERIMENT_ID = "genre-intelligence-truth-v1-b05"
SOURCE_EXPERIMENT_ID = "genre-intelligence-candidate-pool-v1-b05"
QUOTAS = {"Garage": 7, "Minimal": 8, "Tech House": 5}
MINIMUM_NEW_PARENT_ARTISTS = {"Garage": 4, "Minimal": 2, "Tech House": 0}
MAX_TRACKS_PER_ARTIST = 2
SOURCE_PRIORITY = {
    "current_rekordbox_genre": 0,
    "v0_33_recommendation": 1,
}


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
    expected_sha256: str,
    expected_fingerprint: str,
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


def source_key(row: dict[str, Any], purpose: str) -> tuple[int, str]:
    source = str(row["sampling_source_private"])
    if source not in SOURCE_PRIORITY:
        raise ValueError(f"unsupported sampling source {source!r}")
    return SOURCE_PRIORITY[source], stable_hash(row, purpose)


def select_batch(
    rows: list[dict[str, Any]],
    quotas: dict[str, int] = QUOTAS,
    minimum_new_artists: dict[str, int] = MINIMUM_NEW_PARENT_ARTISTS,
    max_tracks_per_artist: int = MAX_TRACKS_PER_ARTIST,
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    used_paths: set[str] = set()
    used_releases: set[str] = set()
    artist_counts: Counter[str] = Counter()

    def available(row: dict[str, Any]) -> bool:
        return (
            str(row["file_path"]) not in used_paths
            and str(row["release_group"]) not in used_releases
            and artist_counts[str(row["artist_group"])] < max_tracks_per_artist
        )

    def add(row: dict[str, Any]) -> None:
        selected.append(row)
        used_paths.add(str(row["file_path"]))
        used_releases.add(str(row["release_group"]))
        artist_counts[str(row["artist_group"])] += 1

    for stratum, quota in sorted(quotas.items()):
        candidates = [
            row
            for row in rows
            if row.get("sampling_stratum_private") == stratum
        ]
        required_new = minimum_new_artists.get(stratum, 0)
        new_artists_selected: set[str] = set()
        if required_new:
            for row in sorted(
                candidates, key=lambda value: source_key(value, "new-artist")
            ):
                artist = str(row["artist_group"])
                if (
                    not row.get("artist_new_to_parent_truth_private")
                    or artist in new_artists_selected
                    or not available(row)
                ):
                    continue
                add(row)
                new_artists_selected.add(artist)
                if len(new_artists_selected) == required_new:
                    break
        if len(new_artists_selected) != required_new:
            raise ValueError(
                f"sampling stratum {stratum!r} produced "
                f"{len(new_artists_selected)} new parent artists; "
                f"required {required_new}"
            )

        selected_in_stratum = sum(
            row["sampling_stratum_private"] == stratum for row in selected
        )
        for row in sorted(candidates, key=lambda value: source_key(value, "quota")):
            if selected_in_stratum == quota:
                break
            if row in selected or not available(row):
                continue
            add(row)
            selected_in_stratum += 1
        if selected_in_stratum != quota:
            raise ValueError(
                f"sampling stratum {stratum!r} produced {selected_in_stratum} "
                f"eligible rows; required {quota}"
            )

    return sorted(selected, key=lambda row: stable_hash(row, "review-order"))


def private_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GI05-{position:02d}",
        "track_id": row["track_id"],
        "file_path": row["file_path"],
        "artist": row["artist"],
        "title": row["title"],
        "album": row.get("album"),
        "artist_group": row["artist_group"],
        "release_group": row["release_group"],
        "sampling_stratum_private": row["sampling_stratum_private"],
        "sampling_source_private": row["sampling_source_private"],
        "source_value_private": row["source_value_private"],
        "source_confidence_private": row["source_confidence_private"],
        "source_row_index_private": row["source_row_index_private"],
        "current_genre_private": row["current_genre_private"],
        "baseline_recommendation_private": row[
            "baseline_recommendation_private"
        ],
        "baseline_confidence_private": row["baseline_confidence_private"],
        "artist_new_to_parent_truth_private": row[
            "artist_new_to_parent_truth_private"
        ],
        "release_new_to_parent_truth_private": row[
            "release_new_to_parent_truth_private"
        ],
    }


def build_result(
    pool: dict[str, Any],
    source_sha256: str,
    *,
    expected_sha256: str,
    expected_fingerprint: str,
) -> dict[str, Any]:
    rows = validate_pool(
        pool,
        source_sha256,
        expected_sha256=expected_sha256,
        expected_fingerprint=expected_fingerprint,
    )
    selected = [
        private_row(row, position)
        for position, row in enumerate(select_batch(rows), start=1)
    ]
    source_counts = Counter(row["sampling_source_private"] for row in selected)
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "blind_development_truth_review_pending",
        "export_playlist_name": EXPERIMENT_ID.replace("-", "_"),
        "source": {
            "artifact_sha256": source_sha256,
            "experiment_id": SOURCE_EXPERIMENT_ID,
            "pool_fingerprint": expected_fingerprint,
            "role": "metadata_directed_sampling_pool_not_truth",
            "current_genre_used_for_sampling": True,
            "model_recommendation_used_for_sampling": True,
            "sampling_metadata_used_as_truth": False,
            "confidence_filter": None,
            "sealed_holdout_excluded": True,
            "prior_operator_reviews_excluded": True,
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(EXPERIMENT_ID.encode()).hexdigest(),
            "fixed_sampling_quotas": QUOTAS,
            "minimum_new_parent_artists": MINIMUM_NEW_PARENT_ARTISTS,
            "source_priority": SOURCE_PRIORITY,
            "maximum_tracks_per_artist": MAX_TRACKS_PER_ARTIST,
            "one_per_path": True,
            "one_per_release_group": True,
            "sampling_and_model_fields_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "selected_source_counts_private": dict(sorted(source_counts.items())),
        "roster_sha256": corpus.fingerprint(selected),
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate-pool", required=True, type=Path)
    parser.add_argument("--expected-pool-sha256", required=True)
    parser.add_argument("--expected-pool-fingerprint", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source_sha256 = corpus.sha256_file(args.candidate_pool)
    pool = json.loads(args.candidate_pool.read_text(encoding="utf-8"))
    result = build_result(
        pool,
        source_sha256,
        expected_sha256=args.expected_pool_sha256,
        expected_fingerprint=args.expected_pool_fingerprint,
    )
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
                "selected_source_counts_private": result[
                    "selected_source_counts_private"
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
