#!/usr/bin/env python3
"""Build a private metadata-directed pool for blind genre truth review."""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable

import build_genre_intelligence_corpus as corpus
import recover_genre_truth_roster as recovery
import select_broad_genre_holdout as library


def track(row: dict[str, Any]) -> dict[str, Any]:
    return row.get("track", row)


def identity(
    row: dict[str, Any], live_by_path: dict[str, dict[str, Any]]
) -> tuple[str, str, str]:
    value = track(row)
    live = live_by_path.get(str(value["file_path"]), {})
    artist = str(value.get("artist") or live.get("artist") or "")
    album = str(value.get("album") or live.get("album") or "")
    title = str(value.get("title") or live.get("title") or "")
    return artist, album, title


def collect_exclusions(
    development_rows: Iterable[dict[str, Any]],
    truth_records: Iterable[dict[str, Any]],
    holdout_rows: Iterable[dict[str, Any]],
    mapping_rows: Iterable[dict[str, Any]],
    exposed_paths: Iterable[str],
    live_by_path: dict[str, dict[str, Any]],
) -> tuple[set[str], set[str]]:
    paths: set[str] = set()
    releases: set[str] = set()
    rows = [
        *development_rows,
        *truth_records,
        *holdout_rows,
        *mapping_rows,
        *({"file_path": path} for path in exposed_paths),
    ]
    for row in rows:
        value = track(row)
        path = str(value["file_path"])
        paths.add(path)
        artist, album, title = identity(row, live_by_path)
        release = str(
            value.get("release_group")
            or corpus.release_group(artist, album, title)
        )
        if release:
            releases.add(release)
    return paths, releases


def truth_groups(
    development_rows: Iterable[dict[str, Any]],
) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    artists: dict[str, set[str]] = defaultdict(set)
    releases: dict[str, set[str]] = defaultdict(set)
    for row in development_rows:
        parent = str(row["canonical_parent_genre"])
        artists[parent].add(str(row["artist_group"]))
        releases[parent].add(str(row["release_group"]))
    return artists, releases


def build_pool(
    audit: dict[str, Any],
    live_rows: list[dict[str, Any]],
    development_rows: list[dict[str, Any]],
    excluded_paths: set[str],
    excluded_releases: set[str],
    *,
    experiment_id: str,
) -> dict[str, Any]:
    audit_rows = audit.get("rows")
    if not isinstance(audit_rows, list):
        raise ValueError("audit manifest has no row list")
    audit_by_path = {str(row["file_path"]): row for row in audit_rows}
    if len(audit_by_path) != len(audit_rows):
        raise ValueError("audit manifest contains duplicate paths")
    truth_artists, truth_releases = truth_groups(development_rows)
    candidates: list[dict[str, Any]] = []
    missing_files = 0
    identity_metadata_drift_rows = 0
    live_rows_absent_from_audit = 0
    unmapped_current_genres: Counter[str] = Counter()
    unmapped_recommendations: Counter[str] = Counter()

    for live in live_rows:
        path = str(live["file_path"])
        artist = str(live["artist"])
        album = str(live["album"])
        title = str(live["title"])
        artist_group = corpus.normalized(artist)
        release_group = corpus.release_group(artist, album, title)
        if (
            not artist_group
            or path in excluded_paths
            or release_group in excluded_releases
        ):
            continue
        if not Path(path).is_file():
            missing_files += 1
            continue

        audit_row = audit_by_path.get(path)
        if audit_row is None:
            live_rows_absent_from_audit += 1
        else:
            if str(audit_row["track_id"]) != str(live["track_id"]):
                raise ValueError(f"live track ID drift for audit row: {path}")
            if any(
                str(audit_row[field]) != str(live[field])
                for field in ("artist", "title", "album")
            ):
                identity_metadata_drift_rows += 1

        current_genre = str(live.get("current_genre") or "")
        current_parent = corpus.FINE_TO_PARENT.get(current_genre)
        if current_genre and current_parent is None:
            unmapped_current_genres[current_genre] += 1

        recommendation = (
            str(audit_row.get("baseline_recommendation") or "")
            if audit_row is not None
            else ""
        )
        recommendation_parent = corpus.FINE_TO_PARENT.get(recommendation)
        if recommendation and recommendation_parent is None:
            unmapped_recommendations[recommendation] += 1

        sources: dict[str, tuple[str, str, str | None]] = {}
        if recommendation_parent is not None:
            sources[recommendation_parent] = (
                "v0_33_recommendation",
                recommendation,
                str(audit_row.get("baseline_confidence") or ""),
            )
        if current_parent is not None:
            sources[current_parent] = (
                "current_rekordbox_genre",
                current_genre,
                None,
            )

        for parent, (source_kind, source_value, source_confidence) in sources.items():
            candidates.append(
                {
                    "track_id": str(live["track_id"]),
                    "file_path": path,
                    "artist": artist,
                    "title": title,
                    "album": album,
                    "artist_group": artist_group,
                    "release_group": release_group,
                    "sampling_stratum_private": parent,
                    "sampling_source_private": source_kind,
                    "source_value_private": source_value,
                    "source_confidence_private": source_confidence,
                    "source_row_index_private": (
                        int(audit_row["row_index"])
                        if audit_row is not None
                        else None
                    ),
                    "current_genre_private": current_genre,
                    "baseline_recommendation_private": recommendation,
                    "baseline_confidence_private": (
                        str(audit_row.get("baseline_confidence") or "")
                        if audit_row is not None
                        else ""
                    ),
                    "artist_new_to_parent_truth_private": (
                        artist_group not in truth_artists[parent]
                    ),
                    "release_new_to_parent_truth_private": (
                        release_group not in truth_releases[parent]
                    ),
                }
            )

    candidates.sort(
        key=lambda row: (
            row["sampling_stratum_private"],
            row["file_path"],
            row["track_id"],
        )
    )
    counts: dict[str, Counter[str]] = defaultdict(Counter)
    for row in candidates:
        counts[str(row["sampling_stratum_private"])][
            str(row["sampling_source_private"])
        ] += 1
    return {
        "experiment_id": experiment_id,
        "method_status": "private_candidate_pool_frozen_before_selection",
        "selection_source": {
            "kind": "current_genre_then_frozen_v0_33_sampling",
            "current_genre_is_not_truth": True,
            "model_recommendation_is_not_truth": True,
            "confidence_filter": None,
        },
        "exclusions": {
            "paths": len(excluded_paths),
            "release_groups": len(excluded_releases),
            "development_truth": True,
            "sealed_holdout": True,
            "all_prior_operator_reviews": True,
            "artist_groups_excluded_globally": False,
        },
        "audit_rows": len(audit_rows),
        "live_rows": len(live_rows),
        "live_rows_absent_from_audit": live_rows_absent_from_audit,
        "missing_file_rows": missing_files,
        "identity_metadata_drift_rows": identity_metadata_drift_rows,
        "unmapped_current_genres": dict(sorted(unmapped_current_genres.items())),
        "unmapped_recommendations": dict(sorted(unmapped_recommendations.items())),
        "candidate_counts": {
            parent: dict(sorted(source_counts.items()))
            for parent, source_counts in sorted(counts.items())
        },
        "candidate_rows": len(candidates),
        "candidate_tracks": len({row["file_path"] for row in candidates}),
        "pool_fingerprint": corpus.fingerprint(candidates),
        "rows": candidates,
    }


def json_rows(path: Path, key: str) -> list[dict[str, Any]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    rows = value.get(key)
    if not isinstance(rows, list):
        raise ValueError(f"{path} has no {key!r} row list")
    return rows


def ledger_rows(path: Path) -> list[dict[str, Any]]:
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]


def require_sha256(path: Path, expected: str, label: str) -> str:
    actual = corpus.sha256_file(path)
    if actual != expected:
        raise ValueError(f"{label} checksum differs: expected {expected}, got {actual}")
    return actual


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--experiment-id", required=True)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--expected-audit-sha256", required=True)
    parser.add_argument("--development-corpus", required=True, type=Path)
    parser.add_argument("--expected-development-sha256", required=True)
    parser.add_argument("--truth-ledger", required=True, type=Path)
    parser.add_argument("--sealed-holdout", required=True, type=Path)
    parser.add_argument("--exclude-mapping", action="append", type=Path, default=[])
    parser.add_argument("--exclude-xml", action="append", type=Path, default=[])
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()

    audit_sha256 = require_sha256(
        args.audit_manifest, args.expected_audit_sha256, "audit manifest"
    )
    development_sha256 = require_sha256(
        args.development_corpus,
        args.expected_development_sha256,
        "development corpus",
    )
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    live_rows, _ = library.load_library_snapshot(args.sqlcipher, args.database)
    live_by_path = {str(row["file_path"]): row for row in live_rows}
    development_rows = json_rows(args.development_corpus, "rows")
    truth_records = ledger_rows(args.truth_ledger)
    holdout_rows = json_rows(args.sealed_holdout, "selected")
    mapping_rows = [
        row
        for path in args.exclude_mapping
        for row in json_rows(path, "selected")
    ]
    exposed_paths = [
        track_path
        for path in args.exclude_xml
        for track_path in recovery.rekordbox_xml_paths(path)
    ]
    excluded_paths, excluded_releases = collect_exclusions(
        development_rows,
        truth_records,
        holdout_rows,
        mapping_rows,
        exposed_paths,
        live_by_path,
    )
    result = build_pool(
        audit,
        live_rows,
        development_rows,
        excluded_paths,
        excluded_releases,
        experiment_id=args.experiment_id,
    )
    result["inputs"] = {
        "audit_manifest_sha256": audit_sha256,
        "development_corpus_sha256": development_sha256,
        "truth_ledger_sha256": corpus.sha256_file(args.truth_ledger),
        "sealed_holdout_sha256": corpus.sha256_file(args.sealed_holdout),
        "exclude_mapping_sha256": {
            str(path): corpus.sha256_file(path) for path in args.exclude_mapping
        },
        "exclude_xml_sha256": {
            str(path): corpus.sha256_file(path) for path in args.exclude_xml
        },
    }
    corpus.atomic_write(
        args.output,
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8")
        + b"\n",
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "candidate_rows": result["candidate_rows"],
                "candidate_tracks": result["candidate_tracks"],
                "candidate_counts": result["candidate_counts"],
                "pool_fingerprint": result["pool_fingerprint"],
                "exclusions": result["exclusions"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
