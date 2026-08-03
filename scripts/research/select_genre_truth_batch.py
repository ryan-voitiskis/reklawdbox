#!/usr/bin/env python3
"""Select frozen blind Genre Intelligence truth-expansion batches."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any


SOURCE_EXPERIMENT_ID = "scoped-broad-genre-mvp-holdout-v1"
SOURCE_ARTIFACT_SHA256 = (
    "7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f"
)
SOURCE_ROSTER_SHA256 = (
    "9cf4cdbd67bc701063d886e991e7f4f57a0b675844423584b3027f0bce5418a9"
)
EXPERIMENT_ID = "genre-intelligence-truth-v1-b01"
SEED = EXPERIMENT_ID
QUOTAS = {"Breakbeat": 3, "Pop": 1, "Trance": 2}
BATCH_CONFIGS = {
    EXPERIMENT_ID: {
        "quotas": QUOTAS,
        "required_prior_batches": set(),
    },
    "genre-intelligence-truth-v1-b03": {
        "quotas": {
            "Breakbeat": 4,
            "Disco": 3,
            "Downtempo": 2,
            "Drum & Bass": 3,
            "Electro": 2,
            "Trance": 6,
        },
        "required_prior_batches": {EXPERIMENT_ID},
    },
}
EXPECTED_PRIOR_ROSTERS = {
    EXPERIMENT_ID: "05e21bd2a42d233047f52104732563a64556921e0d9c750fd3a4d30c917e74c0"
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_hash(row: dict[str, Any], seed: str = SEED) -> str:
    value = "|".join(
        (
            seed,
            str(row["broad_sampling_stratum"]),
            str(row["track_id"]),
            str(row["file_path"]),
        )
    )
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def private_fingerprint(value: Any) -> str:
    payload = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def source_roster_fingerprint(value: Any) -> str:
    """Match the retired holdout selector's ASCII-escaped JSON checksum."""
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def validate_source(
    source: dict[str, Any],
    source_sha256: str,
    expected_artifact_sha256: str = SOURCE_ARTIFACT_SHA256,
    expected_roster_sha256: str = SOURCE_ROSTER_SHA256,
) -> str:
    if source.get("experiment_id") != SOURCE_EXPERIMENT_ID:
        raise ValueError("unexpected retired-roster experiment ID")
    rows = source.get("selected")
    if not isinstance(rows, list):
        raise ValueError("retired roster has no selected row list")
    if source.get("roster_sha256") != expected_roster_sha256:
        raise ValueError("retired-roster checksum differs from the sealed roster")
    if source_roster_fingerprint(rows) != expected_roster_sha256:
        raise ValueError("retired-roster rows do not match their checksum")
    if source_sha256 == expected_artifact_sha256:
        return "original_artifact"
    recovery = source.get("recovery", {})
    if (
        recovery.get("original_artifact_sha256") != expected_artifact_sha256
        or recovery.get("original_roster_sha256") != expected_roster_sha256
        or recovery.get("identity_roster_replayed_exactly") is not True
        or recovery.get("byte_identical_replay") is not True
        or recovery.get("model_features_or_predictions_used") is not False
    ):
        raise ValueError(
            "retired-roster artifact identity changed without a verified recovery"
        )
    return "verified_recovery"


def validated_prior_rows(
    prior_mappings: list[dict[str, Any]],
    expected_rosters: dict[str, str] = EXPECTED_PRIOR_ROSTERS,
) -> tuple[set[str], list[dict[str, Any]]]:
    prior_batch_ids = [str(mapping.get("experiment_id")) for mapping in prior_mappings]
    if len(set(prior_batch_ids)) != len(prior_batch_ids):
        raise ValueError("prior batch mappings must be unique")
    rows: list[dict[str, Any]] = []
    for mapping, batch_id in zip(prior_mappings, prior_batch_ids, strict=True):
        expected_roster = expected_rosters.get(batch_id)
        selected = mapping.get("selected")
        if expected_roster is None or not isinstance(selected, list):
            raise ValueError(f"unsupported prior batch mapping {batch_id!r}")
        if mapping.get("roster_sha256") != expected_roster:
            raise ValueError(f"prior batch {batch_id} roster checksum differs")
        if private_fingerprint(selected) != expected_roster:
            raise ValueError(f"prior batch {batch_id} rows do not match their checksum")
        rows.extend(selected)
    return set(prior_batch_ids), rows


def select_batch(
    rows: list[dict[str, Any]],
    quotas: dict[str, int] = QUOTAS,
    seed: str = SEED,
    excluded_paths: set[str] | None = None,
    excluded_artists: set[str] | None = None,
    excluded_releases: set[str] | None = None,
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    used_paths = set(excluded_paths or set())
    used_artists = set(excluded_artists or set())
    used_releases = set(excluded_releases or set())

    for stratum, count in sorted(quotas.items()):
        candidates = sorted(
            (
                row
                for row in rows
                if row.get("broad_sampling_stratum") == stratum
            ),
            key=lambda row: stable_hash(row, seed),
        )
        accepted = 0
        for row in candidates:
            path = str(row["file_path"])
            artist_group = str(row["artist_group"])
            release_group = str(row["release_group"])
            if (
                path in used_paths
                or artist_group in used_artists
                or release_group in used_releases
            ):
                continue
            selected.append(dict(row))
            used_paths.add(path)
            used_artists.add(artist_group)
            used_releases.add(release_group)
            accepted += 1
            if accepted == count:
                break
        if accepted != count:
            raise ValueError(
                f"sampling stratum {stratum!r} produced {accepted} distinct rows; "
                f"required {count}"
            )

    selected.sort(
        key=lambda row: hashlib.sha256(
            f"{seed}|review-order|{row['track_id']}|{row['file_path']}".encode()
        ).hexdigest()
    )
    return selected


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
        "sampling_stratum_private": row["broad_sampling_stratum"],
        "source_code": row["code"],
    }


def build_result(
    source: dict[str, Any],
    source_sha256: str,
    experiment_id: str = EXPERIMENT_ID,
    prior_mappings: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    if experiment_id not in BATCH_CONFIGS:
        raise ValueError(f"unsupported truth batch {experiment_id!r}")
    prior_mappings = prior_mappings or []
    prior_batch_ids, prior_rows = validated_prior_rows(prior_mappings)
    required = BATCH_CONFIGS[experiment_id]["required_prior_batches"]
    if not required.issubset(prior_batch_ids):
        raise ValueError(
            f"truth batch {experiment_id} requires prior mappings {sorted(required)}"
        )
    excluded_paths = {str(row["file_path"]) for row in prior_rows}
    excluded_artists = {str(row["artist_group"]) for row in prior_rows}
    excluded_releases = {str(row["release_group"]) for row in prior_rows}
    source_verification = validate_source(source, source_sha256)
    rows = source.get("selected")
    assert isinstance(rows, list)
    quotas = BATCH_CONFIGS[experiment_id]["quotas"]
    selected = [
        private_row(row, position, experiment_id)
        for position, row in enumerate(
            select_batch(
                rows,
                quotas=quotas,
                seed=experiment_id,
                excluded_paths=excluded_paths,
                excluded_artists=excluded_artists,
                excluded_releases=excluded_releases,
            ),
            start=1,
        )
    ]
    return {
        "experiment_id": experiment_id,
        "method_status": "blind_development_truth_review_pending",
        "export_playlist_name": experiment_id.replace("-", "_"),
        "source": {
            "artifact_sha256": source_sha256,
            "expected_original_artifact_sha256": SOURCE_ARTIFACT_SHA256,
            "historical_experiment_id": SOURCE_EXPERIMENT_ID,
            "roster_sha256": SOURCE_ROSTER_SHA256,
            "role": "retired_experiment_roster_not_holdout_evidence",
            "model_predictions_used": False,
            "verification": source_verification,
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(experiment_id.encode()).hexdigest(),
            "fixed_sampling_quotas": quotas,
            "prior_batch_ids": sorted(prior_batch_ids),
            "one_per_path": True,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "sampling_strata_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "roster_sha256": private_fingerprint(selected),
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--retired-roster", required=True, type=Path)
    parser.add_argument(
        "--batch-id", choices=sorted(BATCH_CONFIGS), default=EXPERIMENT_ID
    )
    parser.add_argument("--prior-mapping", action="append", type=Path, default=[])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    source_sha256 = sha256_file(args.retired_roster)
    source = json.loads(args.retired_roster.read_text(encoding="utf-8"))
    prior_mappings = [
        json.loads(path.read_text(encoding="utf-8")) for path in args.prior_mapping
    ]
    result = build_result(source, source_sha256, args.batch_id, prior_mappings)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
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
