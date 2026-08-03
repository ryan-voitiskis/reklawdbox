#!/usr/bin/env python3
"""Select a deterministic metadata-directed blind genre-truth batch."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus


EXPERIMENT_ID = "genre-intelligence-truth-v1-b05"
SOURCE_EXPERIMENT_ID = "genre-intelligence-candidate-pool-v1-b05"
QUOTAS = {"Garage": 7, "Minimal": 7, "Tech House": 6}
MINIMUM_NEW_PARENT_ARTISTS = {"Garage": 4, "Minimal": 2, "Tech House": 0}
MAX_TRACKS_PER_ARTIST = 3
STRATUM_ORDER = ("Minimal", "Garage", "Tech House")
SOURCE_PRIORITY = {
    "current_rekordbox_genre": 0,
    "v0_33_recommendation": 1,
}
DEFAULT_CONFIG = {
    "schema_version": 1,
    "experiment_id": EXPERIMENT_ID,
    "source_experiment_id": SOURCE_EXPERIMENT_ID,
    "quotas": QUOTAS,
    "minimum_new_parent_artists": MINIMUM_NEW_PARENT_ARTISTS,
    "maximum_tracks_per_artist": MAX_TRACKS_PER_ARTIST,
    "stratum_order": STRATUM_ORDER,
}


def stable_hash(
    row: dict[str, Any], purpose: str, experiment_id: str = EXPERIMENT_ID
) -> str:
    value = "|".join(
        (
            experiment_id,
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
    source_experiment_id: str = SOURCE_EXPERIMENT_ID,
) -> list[dict[str, Any]]:
    if source_sha256 != expected_sha256:
        raise ValueError("candidate-pool artifact checksum differs")
    if value.get("experiment_id") != source_experiment_id:
        raise ValueError("unexpected candidate-pool experiment ID")
    rows = value.get("rows")
    if not isinstance(rows, list):
        raise ValueError("candidate pool has no row list")
    if value.get("pool_fingerprint") != expected_fingerprint:
        raise ValueError("candidate-pool fingerprint differs")
    if corpus.fingerprint(rows) != expected_fingerprint:
        raise ValueError("candidate-pool rows do not match their fingerprint")
    return rows


def source_key(
    row: dict[str, Any], purpose: str, experiment_id: str = EXPERIMENT_ID
) -> tuple[int, str]:
    source = str(row["sampling_source_private"])
    if source not in SOURCE_PRIORITY:
        raise ValueError(f"unsupported sampling source {source!r}")
    return SOURCE_PRIORITY[source], stable_hash(row, purpose, experiment_id)


def select_batch(
    rows: list[dict[str, Any]],
    quotas: dict[str, int] = QUOTAS,
    minimum_new_artists: dict[str, int] = MINIMUM_NEW_PARENT_ARTISTS,
    max_tracks_per_artist: int = MAX_TRACKS_PER_ARTIST,
    stratum_order: tuple[str, ...] = STRATUM_ORDER,
    experiment_id: str = EXPERIMENT_ID,
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

    if set(quotas) != set(stratum_order):
        raise ValueError("sampling quotas differ from the frozen stratum order")
    for stratum in stratum_order:
        quota = quotas[stratum]
        candidates = [
            row
            for row in rows
            if row.get("sampling_stratum_private") == stratum
        ]
        required_new = minimum_new_artists.get(stratum, 0)
        new_artists_selected: set[str] = set()
        if required_new:
            for row in sorted(
                candidates,
                key=lambda value: source_key(value, "new-artist", experiment_id),
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
        for row in sorted(
            candidates,
            key=lambda value: source_key(value, "quota", experiment_id),
        ):
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

    return sorted(
        selected,
        key=lambda row: stable_hash(row, "review-order", experiment_id),
    )


def private_row(
    row: dict[str, Any], position: int, experiment_id: str = EXPERIMENT_ID
) -> dict[str, Any]:
    batch_number = experiment_id.rsplit("b", 1)[-1]
    return {
        "position": position,
        "code": f"GI{batch_number}-{position:02d}",
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
    config: dict[str, Any] = DEFAULT_CONFIG,
    private_config_sha256: str | None = None,
) -> dict[str, Any]:
    normalized_config = validate_config(config)
    experiment_id = normalized_config["experiment_id"]
    source_experiment_id = normalized_config["source_experiment_id"]
    rows = validate_pool(
        pool,
        source_sha256,
        expected_sha256=expected_sha256,
        expected_fingerprint=expected_fingerprint,
        source_experiment_id=source_experiment_id,
    )
    selected = [
        private_row(row, position, experiment_id)
        for position, row in enumerate(
            select_batch(
                rows,
                quotas=normalized_config["quotas"],
                minimum_new_artists=normalized_config[
                    "minimum_new_parent_artists"
                ],
                max_tracks_per_artist=normalized_config[
                    "maximum_tracks_per_artist"
                ],
                stratum_order=normalized_config["stratum_order"],
                experiment_id=experiment_id,
            ),
            start=1,
        )
    ]
    source_counts = Counter(row["sampling_source_private"] for row in selected)
    return {
        "experiment_id": experiment_id,
        "method_status": "blind_development_truth_review_pending",
        "export_playlist_name": experiment_id.replace("-", "_"),
        "source": {
            "artifact_sha256": source_sha256,
            "experiment_id": source_experiment_id,
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
            "seed_sha256": hashlib.sha256(experiment_id.encode()).hexdigest(),
            "private_config_sha256": private_config_sha256,
            "fixed_sampling_quotas": normalized_config["quotas"],
            "minimum_new_parent_artists": normalized_config[
                "minimum_new_parent_artists"
            ],
            "stratum_order": normalized_config["stratum_order"],
            "source_priority": SOURCE_PRIORITY,
            "maximum_tracks_per_artist": normalized_config[
                "maximum_tracks_per_artist"
            ],
            "one_per_path": True,
            "one_per_release_group": True,
            "sampling_and_model_fields_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "selected_source_counts_private": dict(sorted(source_counts.items())),
        "roster_sha256": corpus.fingerprint(selected),
        "selected": selected,
    }


def validate_config(value: dict[str, Any]) -> dict[str, Any]:
    if value.get("schema_version") != 1:
        raise ValueError("unsupported private selection config schema")
    experiment_id = str(value.get("experiment_id") or "")
    source_experiment_id = str(value.get("source_experiment_id") or "")
    match = re.fullmatch(r"genre-intelligence-truth-v1-b(\d{2})", experiment_id)
    if match is None:
        raise ValueError("invalid truth-batch experiment ID")
    expected_source = f"genre-intelligence-candidate-pool-v1-b{match.group(1)}"
    if source_experiment_id != expected_source:
        raise ValueError("candidate-pool experiment ID does not match truth batch")

    quotas = value.get("quotas")
    if (
        not isinstance(quotas, dict)
        or not quotas
        or any(parent not in corpus.PARENT_GENRES for parent in quotas)
        or any(not isinstance(count, int) or count < 1 for count in quotas.values())
        or sum(quotas.values()) > 20
    ):
        raise ValueError("private selection quotas are invalid")
    minima = value.get("minimum_new_parent_artists")
    if (
        not isinstance(minima, dict)
        or set(minima) != set(quotas)
        or any(
            not isinstance(count, int) or count < 0 or count > quotas[parent]
            for parent, count in minima.items()
        )
    ):
        raise ValueError("private new-parent-artist minima are invalid")
    stratum_order = value.get("stratum_order")
    if (
        not isinstance(stratum_order, (list, tuple))
        or len(stratum_order) != len(quotas)
        or set(stratum_order) != set(quotas)
    ):
        raise ValueError("private stratum order differs from quotas")
    maximum = value.get("maximum_tracks_per_artist")
    if not isinstance(maximum, int) or maximum < 1 or maximum > 20:
        raise ValueError("private artist cap is invalid")
    return {
        "schema_version": 1,
        "experiment_id": experiment_id,
        "source_experiment_id": source_experiment_id,
        "quotas": dict(quotas),
        "minimum_new_parent_artists": dict(minima),
        "maximum_tracks_per_artist": maximum,
        "stratum_order": tuple(stratum_order),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate-pool", required=True, type=Path)
    parser.add_argument("--expected-pool-sha256", required=True)
    parser.add_argument("--expected-pool-fingerprint", required=True)
    parser.add_argument("--private-config", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source_sha256 = corpus.sha256_file(args.candidate_pool)
    pool = json.loads(args.candidate_pool.read_text(encoding="utf-8"))
    config = DEFAULT_CONFIG
    private_config_sha256 = None
    if args.private_config is not None:
        config = json.loads(args.private_config.read_text(encoding="utf-8"))
        private_config_sha256 = corpus.sha256_file(args.private_config)
    result = build_result(
        pool,
        source_sha256,
        expected_sha256=args.expected_pool_sha256,
        expected_fingerprint=args.expected_pool_fingerprint,
        config=config,
        private_config_sha256=private_config_sha256,
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
