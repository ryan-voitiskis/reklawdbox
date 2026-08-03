#!/usr/bin/env python3
"""Build the frozen private candidate pool for blind truth batch B04."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

import build_genre_intelligence_corpus as corpus
import recover_genre_truth_roster as recovery
import select_broad_genre_holdout as library


EXPERIMENT_ID = "genre-intelligence-candidate-pool-v1-b04"
EXPECTED_AUDIT_SHA256 = (
    "d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e"
)
EXPECTED_DEVELOPMENT_CORPUS_SHA256 = (
    "4c71348627af625b122fea206a340b8011c19c5a6cff2b81e06b1d11d70515f4"
)


def add_exclusion(
    paths: set[str],
    artists: set[str],
    releases: set[str],
    row: dict[str, Any],
    live_by_path: dict[str, dict[str, Any]],
) -> None:
    track = row.get("track", row)
    path = str(track["file_path"])
    paths.add(path)
    live = live_by_path.get(path)
    artist = str(track.get("artist") or (live or {}).get("artist") or "")
    album = str(track.get("album") or (live or {}).get("album") or "")
    title = str(track.get("title") or (live or {}).get("title") or "")
    artist_group = str(track.get("artist_group") or corpus.normalized(artist))
    release_group = str(
        track.get("release_group")
        or corpus.release_group(artist, album, title)
    )
    if artist_group:
        artists.add(artist_group)
    if release_group:
        releases.add(release_group)


def collect_exclusions(
    development_rows: Iterable[dict[str, Any]],
    truth_records: Iterable[dict[str, Any]],
    holdout_rows: Iterable[dict[str, Any]],
    mapping_rows: Iterable[dict[str, Any]],
    exposed_paths: Iterable[str],
    live_by_path: dict[str, dict[str, Any]],
) -> tuple[set[str], set[str], set[str]]:
    paths: set[str] = set()
    artists: set[str] = set()
    releases: set[str] = set()
    for rows in (development_rows, truth_records, holdout_rows, mapping_rows):
        for row in rows:
            add_exclusion(paths, artists, releases, row, live_by_path)
    for path in exposed_paths:
        add_exclusion(paths, artists, releases, {"file_path": path}, live_by_path)
    return paths, artists, releases


def build_pool(
    audit: dict[str, Any],
    live_rows: list[dict[str, Any]],
    excluded_paths: set[str],
    excluded_artists: set[str],
    excluded_releases: set[str],
) -> dict[str, Any]:
    audit_rows = audit.get("rows")
    if not isinstance(audit_rows, list):
        raise ValueError("audit manifest has no row list")
    live_by_path = {str(row["file_path"]): row for row in live_rows}
    if len(live_by_path) != len(live_rows):
        raise ValueError("live library contains duplicate paths")
    candidates = []
    missing_files = 0
    identity_metadata_drift_rows = 0
    unmapped_recommendations = 0
    for row in audit_rows:
        path = str(row["file_path"])
        live = live_by_path.get(path)
        if live is None:
            continue
        if str(live["track_id"]) != str(row["track_id"]):
            raise ValueError(f"live track ID drift for audit row: {path}")
        if any(
            str(live[field]) != str(row[field])
            for field in ("artist", "title", "album")
        ):
            identity_metadata_drift_rows += 1
        if not Path(path).is_file():
            missing_files += 1
            continue
        artist_group = corpus.normalized(str(live["artist"]))
        release_group = corpus.release_group(
            str(live["artist"]), str(live["album"]), str(live["title"])
        )
        if (
            not artist_group
            or path in excluded_paths
            or artist_group in excluded_artists
            or release_group in excluded_releases
        ):
            continue
        recommendation = str(row["baseline_recommendation"])
        sampling_stratum = corpus.FINE_TO_PARENT.get(recommendation)
        if sampling_stratum is None:
            unmapped_recommendations += 1
            continue
        candidates.append(
            {
                "track_id": str(row["track_id"]),
                "file_path": path,
                "artist": str(live["artist"]),
                "title": str(live["title"]),
                "album": str(live["album"]),
                "artist_group": artist_group,
                "release_group": release_group,
                "sampling_stratum_private": sampling_stratum,
                "source_recommendation_private": recommendation,
                "source_confidence_private": str(row["baseline_confidence"]),
                "source_row_index": int(row["row_index"]),
            }
        )
    candidates.sort(key=lambda row: (row["file_path"], row["track_id"]))
    counts = Counter(row["sampling_stratum_private"] for row in candidates)
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "private_candidate_pool_frozen_before_identity_review",
        "selection_source": {
            "kind": "frozen_v0_33_recommendation_sampling_only",
            "recommendation_is_not_truth": True,
            "confidence_filter": None,
        },
        "exclusions": {
            "paths": len(excluded_paths),
            "artists": len(excluded_artists),
            "release_groups": len(excluded_releases),
            "development_truth": True,
            "sealed_holdout": True,
            "all_prior_operator_reviews": True,
        },
        "audit_rows": len(audit_rows),
        "missing_file_rows": missing_files,
        "identity_metadata_drift_rows": identity_metadata_drift_rows,
        "unmapped_recommendation_rows": unmapped_recommendations,
        "candidate_counts": dict(sorted(counts.items())),
        "candidate_rows": len(candidates),
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--development-corpus", required=True, type=Path)
    parser.add_argument("--truth-ledger", required=True, type=Path)
    parser.add_argument("--sealed-holdout", required=True, type=Path)
    parser.add_argument("--exclude-mapping", action="append", type=Path, default=[])
    parser.add_argument("--exclude-xml", action="append", type=Path, default=[])
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    audit_sha256 = corpus.sha256_file(args.audit_manifest)
    if audit_sha256 != EXPECTED_AUDIT_SHA256:
        raise ValueError("frozen audit candidate source changed")
    development_sha256 = corpus.sha256_file(args.development_corpus)
    if development_sha256 != EXPECTED_DEVELOPMENT_CORPUS_SHA256:
        raise ValueError("frozen development corpus changed")
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
    exclusions = collect_exclusions(
        development_rows,
        truth_records,
        holdout_rows,
        mapping_rows,
        exposed_paths,
        live_by_path,
    )
    result = build_pool(audit, live_rows, *exclusions)
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
