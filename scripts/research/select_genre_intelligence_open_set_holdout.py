#!/usr/bin/env python3
"""Seal the Plan 071 open-set holdout before candidate training or inference."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus
import select_broad_genre_holdout as previous


EXPERIMENT_ID = "genre-intelligence-open-set-holdout-v2"
METHOD_STATUS = "sealed_before_candidate_training_or_inference"
SEED = "genre-intelligence-open-set-holdout-v2"
HOLDOUT_ROWS = 60
MAX_PER_STRATUM = 15
EXPECTED_HASHES = {
    "audit_manifest_sha256": (
        "d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e"
    ),
    "development_corpus_sha256": (
        "0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1"
    ),
    "consumed_holdout_sha256": (
        "1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37"
    ),
}
DESIRED_QUOTAS = {
    "Ambient": 5,
    "Breakbeat": 10,
    "Downtempo": 1,
    "Drum & Bass": 1,
    "Electro": 1,
    "Hip Hop": 1,
    "House": 12,
    "IDM": 1,
    "Minimal": 2,
    "Pop": 2,
    "R&B": 8,
    "Reggae": 3,
    "Techno": 12,
    "Trance": 1,
}


def stable_hash(*parts: object) -> str:
    value = "|".join(str(part) for part in (SEED, *parts))
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def select_holdout(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_stratum: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        stratum = str(row["broad_sampling_stratum"])
        by_stratum[stratum].append(row)
    for stratum, values in by_stratum.items():
        values.sort(
            key=lambda row: stable_hash(
                "row", stratum, row["track_id"], row["file_path"]
            )
        )

    selected: list[dict[str, Any]] = []
    selected_paths: set[str] = set()
    used_artists: set[str] = set()
    used_releases: set[str] = set()
    counts: Counter[str] = Counter()

    def add(row: dict[str, Any]) -> bool:
        stratum = str(row["broad_sampling_stratum"])
        path = str(row["file_path"])
        artist = str(row["artist_group"])
        release = str(row["release_group"])
        if (
            path in selected_paths
            or artist in used_artists
            or release in used_releases
            or counts[stratum] >= MAX_PER_STRATUM
        ):
            return False
        selected.append(row)
        selected_paths.add(path)
        used_artists.add(artist)
        used_releases.add(release)
        counts[stratum] += 1
        return True

    scarcity_order = sorted(
        DESIRED_QUOTAS,
        key=lambda stratum: (
            len(by_stratum.get(stratum, [])),
            stable_hash("scarcity", stratum),
        ),
    )
    for stratum in scarcity_order:
        for row in by_stratum.get(stratum, []):
            if counts[stratum] >= DESIRED_QUOTAS[stratum]:
                break
            add(row)

    remaining = sorted(
        (row for values in by_stratum.values() for row in values),
        key=lambda row: stable_hash(
            "fill",
            row["broad_sampling_stratum"],
            row["track_id"],
            row["file_path"],
        ),
    )
    for row in remaining:
        if len(selected) == HOLDOUT_ROWS:
            break
        add(row)

    if len(selected) != HOLDOUT_ROWS:
        raise ValueError(
            f"frozen selector produced {len(selected)} rows; "
            f"required {HOLDOUT_ROWS}"
        )
    if len(used_artists) != HOLDOUT_ROWS or len(used_releases) != HOLDOUT_ROWS:
        raise ValueError("holdout artist or release isolation differs")
    return selected


def selected_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GIO{position:03d}",
        "track_id": str(row["track_id"]),
        "file_path": str(row["file_path"]),
        "artist": str(row["artist"]),
        "title": str(row["title"]),
        "album": str(row["album"]),
        "current_genre_sampling_only": str(row["current_genre"]),
        "broad_sampling_stratum": str(row["broad_sampling_stratum"]),
        "artist_group": str(row["artist_group"]),
        "release_group": str(row["release_group"]),
    }


def validate_hash(path: Path, field: str) -> str:
    value = corpus.sha256_file(path)
    if value != EXPECTED_HASHES[field]:
        raise ValueError(f"{field} differs from the frozen Plan 071 input")
    return value


def run(args: argparse.Namespace) -> dict[str, Any]:
    observed = {
        "audit_manifest_sha256": validate_hash(
            args.audit_manifest, "audit_manifest_sha256"
        ),
        "development_corpus_sha256": validate_hash(
            args.development_corpus, "development_corpus_sha256"
        ),
        "consumed_holdout_sha256": validate_hash(
            args.consumed_holdout, "consumed_holdout_sha256"
        ),
        "selector_source_sha256": corpus.sha256_file(Path(__file__)),
    }
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = json.loads(args.development_corpus.read_text(encoding="utf-8"))
    consumed = json.loads(args.consumed_holdout.read_text(encoding="utf-8"))
    tracks, playlists = previous.load_library_snapshot(args.sqlcipher, args.database)
    tracks_by_path = {str(row["file_path"]): row for row in tracks}
    if len(tracks_by_path) != len(tracks):
        raise ValueError("live library contains duplicate paths")

    exclusion_playlists = set(audit["exclusion_playlists"])
    exclusion_playlists.add("genre_verified")
    excluded_paths, excluded_artists, excluded_releases = (
        previous.development_identities(
            tracks_by_path,
            playlists,
            [development],
            exclusion_playlists,
            [consumed],
        )
    )
    eligible, exclusions = previous.eligible_rows(
        audit["rows"],
        tracks_by_path,
        excluded_paths,
        excluded_artists,
        excluded_releases,
    )
    selected = [
        selected_row(row, position)
        for position, row in enumerate(select_holdout(eligible), start=1)
    ]
    snapshot = {
        "tracks": tracks,
        "playlists": {key: sorted(value) for key, value in sorted(playlists.items())},
    }
    counts = Counter(row["broad_sampling_stratum"] for row in selected)
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "inputs": {
            **observed,
            "library_snapshot_sha256": corpus.fingerprint(snapshot),
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(SEED.encode("utf-8")).hexdigest(),
            "rows": HOLDOUT_ROWS,
            "desired_hidden_stratum_quotas": DESIRED_QUOTAS,
            "scarce_strata_selected_first": True,
            "fixed_seed_fill_after_quota_shortfall": True,
            "maximum_per_hidden_stratum": MAX_PER_STRATUM,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "current_genre_is_sampling_only": True,
            "development_and_consumed_holdout_groups_excluded": True,
            "exclusion_playlists": sorted(exclusion_playlists),
        },
        "universe": {
            "library_rows": len(tracks),
            "audit_rows": len(audit["rows"]),
            "eligible_rows": len(eligible),
            "eligible_artists": len({row["artist_group"] for row in eligible}),
            "eligible_release_groups": len(
                {row["release_group"] for row in eligible}
            ),
            "exclusions": dict(sorted(exclusions.items())),
        },
        "hidden_stratum_counts": dict(sorted(counts.items())),
        "roster_sha256": corpus.fingerprint(selected),
        "selected": selected,
    }


def atomic_write(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True, ensure_ascii=False)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--development-corpus", required=True, type=Path)
    parser.add_argument("--consumed-holdout", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    result = run(args)
    atomic_write(args.output, result)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "output_sha256": corpus.sha256_file(args.output),
                "method_status": result["method_status"],
                "roster_sha256": result["roster_sha256"],
                "universe": result["universe"],
                "hidden_stratum_counts": result["hidden_stratum_counts"],
                "identity_values_exposed": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
