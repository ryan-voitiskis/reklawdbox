#!/usr/bin/env python3
"""Seal the independent Plan 067 scoped broad-genre holdout."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import select_broad_genre_holdout as base


EXPERIMENT_ID = "scoped-broad-genre-mvp-holdout-v1"
SEED = "scoped-broad-genre-mvp-holdout-v1"
DEFAULT_COUNT = 48
MAX_PER_TARGET = 8


def stable_hash(*parts: object) -> str:
    value = "|".join(str(part) for part in (SEED, *parts))
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def select_holdout(
    rows: list[dict[str, Any]], count: int = DEFAULT_COUNT
) -> list[dict[str, Any]]:
    by_target: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        candidate = dict(row)
        candidate["scoped_stable_hash"] = stable_hash(
            candidate["broad_sampling_stratum"],
            candidate["track_id"],
            candidate["file_path"],
        )
        by_target[str(candidate["broad_sampling_stratum"])].append(candidate)
    for target_rows in by_target.values():
        target_rows.sort(key=lambda row: str(row["scoped_stable_hash"]))

    selected: list[dict[str, Any]] = []
    used_artists: set[str] = set()
    used_releases: set[str] = set()
    target_counts: Counter[str] = Counter()
    cursors: Counter[str] = Counter()

    for round_index in range(MAX_PER_TARGET):
        target_order = sorted(
            by_target,
            key=lambda target: stable_hash("target", round_index, target),
        )
        for target in target_order:
            if len(selected) == count:
                return selected
            candidates = by_target[target]
            while cursors[target] < len(candidates):
                candidate = candidates[cursors[target]]
                cursors[target] += 1
                if (
                    candidate["artist_group"] in used_artists
                    or candidate["release_group"] in used_releases
                ):
                    continue
                selected.append(candidate)
                used_artists.add(str(candidate["artist_group"]))
                used_releases.add(str(candidate["release_group"]))
                target_counts[target] += 1
                break

    if len(selected) == count:
        return selected
    raise ValueError(
        f"scoped selector produced {len(selected)} rows; required {count} "
        f"with at most {MAX_PER_TARGET} per target"
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = [
        json.loads(path.read_text(encoding="utf-8"))
        for path in args.development_manifest
    ]
    exposed = [
        json.loads(path.read_text(encoding="utf-8"))
        for path in [*args.exposed_result, *args.prior_holdout]
    ]
    tracks, playlists = base.load_library_snapshot(args.sqlcipher, args.database)
    tracks_by_path = {str(row["file_path"]): row for row in tracks}

    exclusion_playlists = set(audit["exclusion_playlists"])
    exclusion_playlists.add("genre_verified")
    excluded_paths, excluded_artists, excluded_releases = base.development_identities(
        tracks_by_path,
        playlists,
        development,
        exclusion_playlists,
        exposed,
    )
    eligible, exclusions = base.eligible_rows(
        audit["rows"],
        tracks_by_path,
        excluded_paths,
        excluded_artists,
        excluded_releases,
    )
    selected_rows = select_holdout(eligible, args.count)
    selected = [
        base.selected_private_row(row, index + 1)
        for index, row in enumerate(selected_rows)
    ]
    snapshot = {
        "tracks": tracks,
        "playlists": {
            key: sorted(value) for key, value in sorted(playlists.items())
        },
    }
    result = {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "sealed_before_scoped_development_evaluation",
        "inputs": {
            "audit_manifest_sha256": base.sha256_file(args.audit_manifest),
            "development_manifest_sha256": [
                base.sha256_file(path) for path in args.development_manifest
            ],
            "exposed_result_sha256": [
                base.sha256_file(path) for path in args.exposed_result
            ],
            "prior_holdout_sha256": [
                base.sha256_file(path) for path in args.prior_holdout
            ],
            "library_snapshot_sha256": base.private_fingerprint(snapshot),
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(SEED.encode("utf-8")).hexdigest(),
            "count": args.count,
            "maximum_per_broad_sampling_stratum": MAX_PER_TARGET,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "current_genre_is_sampling_only": True,
            "exclusion_playlists": sorted(exclusion_playlists),
            "prior_holdouts_are_artist_and_release_exclusions": True,
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
        "target_counts": dict(
            sorted(Counter(row["broad_sampling_stratum"] for row in selected).items())
        ),
        "roster_sha256": base.private_fingerprint(selected),
        "selected": selected,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument(
        "--development-manifest", required=True, action="append", type=Path
    )
    parser.add_argument("--exposed-result", required=True, action="append", type=Path)
    parser.add_argument("--prior-holdout", required=True, action="append", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    result = run(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "experiment_id": result["experiment_id"],
                "method_status": result["method_status"],
                "roster_sha256": result["roster_sha256"],
                "universe": result["universe"],
                "target_counts": result["target_counts"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
