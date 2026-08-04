#!/usr/bin/env python3
"""Prepare and audit the sealed Plan 071/072 open-set holdout inputs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus
import evaluate_genre_intelligence_open_set as evaluation
import select_broad_genre_holdout as library


EXPERIMENT_ID = "genre-intelligence-v1-precision-buffer-holdout"
EXPECTED_ROWS = 60
EXPECTED_ROSTER_SHA256 = (
    "81ea5361b52ac1edc5c885abb72dddbe88f352aa1b6ff599957bd444f45b1519"
)
EXPECTED_INPUT_SHA256 = {
    "holdout": (
        "35968f0e3947502ede3322295b4cba6d692e6aefedc7b10940fd82ac9c43f662"
    ),
    "audit_manifest": (
        "d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e"
    ),
    "development_manifest": (
        "dfd11addd96a2e7b5727700594b337aaacfc19bdd97db408e1ba0955f80853bd"
    ),
    "development_features": (
        "6bf80b80f060649877a90a5d6dfa8188c9549eaa0986f1667d611e115689b682"
    ),
    "development_corpus": (
        "0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1"
    ),
    "consumed_holdout": (
        "1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37"
    ),
}


def unique_by(
    rows: list[dict[str, Any]], field: str, label: str
) -> dict[str, dict[str, Any]]:
    result = {}
    for row in rows:
        value = str(row[field])
        if not value or value in result:
            raise ValueError(f"{label} contains a blank or duplicate {field}")
        result[value] = row
    return result


def prepare_rows(
    selected: list[dict[str, Any]],
    audit_rows: list[dict[str, Any]],
    development_rows: list[dict[str, Any]],
    development_feature_rows: list[dict[str, Any]],
    corpus_rows: list[dict[str, Any]],
    consumed_rows: list[dict[str, Any]],
    library_rows: list[dict[str, Any]],
    playlists: dict[str, set[str]],
    exclusion_playlists: set[str],
) -> tuple[list[dict[str, str]], dict[str, int]]:
    if len(selected) != EXPECTED_ROWS:
        raise ValueError(f"holdout must contain exactly {EXPECTED_ROWS} rows")
    if len(development_rows) != len(development_feature_rows):
        raise ValueError("development truth and feature row counts differ")
    audit_by_path = unique_by(audit_rows, "file_path", "audit manifest")
    live_by_path = unique_by(library_rows, "file_path", "live library")
    development_by_id = unique_by(development_rows, "row_id", "development")
    features_by_id = unique_by(
        development_feature_rows, "row_id", "development features"
    )
    if set(development_by_id) != set(features_by_id):
        raise ValueError("development truth and feature identities differ")

    development_paths = {
        str(features_by_id[row_id]["file_path"]) for row_id in development_by_id
    }
    development_artists = {
        str(row["artist_group"]) for row in development_rows
    }
    development_releases = {
        str(row["release_group"]) for row in development_rows
    }
    reviewed_paths = {str(row["file_path"]) for row in corpus_rows}
    consumed_paths = {str(row["file_path"]) for row in consumed_rows}
    consumed_artists = {str(row["artist_group"]) for row in consumed_rows}
    consumed_releases = {str(row["release_group"]) for row in consumed_rows}
    playlist_track_ids = {
        str(track_id)
        for name in exclusion_playlists
        for track_id in playlists.get(name, set())
    }

    feature_rows = []
    holdout_paths: set[str] = set()
    holdout_artists: set[str] = set()
    holdout_releases: set[str] = set()
    holdout_track_ids: set[str] = set()
    missing_files = 0
    for expected_position, row in enumerate(selected, start=1):
        if int(row["position"]) != expected_position:
            raise ValueError("holdout positions are not contiguous")
        path = str(row["file_path"])
        track_id = str(row["track_id"])
        artist = str(row["artist_group"])
        release = str(row["release_group"])
        if not path or path in holdout_paths:
            raise ValueError("holdout contains a blank or duplicate path")
        if not track_id or track_id in holdout_track_ids:
            raise ValueError("holdout contains a blank or duplicate track ID")
        if not artist or artist in holdout_artists:
            raise ValueError("holdout contains a blank or duplicate artist group")
        if not release or release in holdout_releases:
            raise ValueError("holdout contains a blank or duplicate release group")
        audit = audit_by_path.get(path)
        live = live_by_path.get(path)
        if audit is None or str(audit["track_id"]) != track_id:
            raise ValueError("holdout identity does not match the frozen audit")
        if live is None or str(live["track_id"]) != track_id:
            raise ValueError("holdout identity does not match the live snapshot")
        if str(live["artist_group"]) != artist:
            raise ValueError("holdout artist group differs from the live snapshot")
        if str(live["release_group"]) != release:
            raise ValueError("holdout release group differs from the live snapshot")
        missing_files += int(not Path(path).is_file())
        holdout_paths.add(path)
        holdout_track_ids.add(track_id)
        holdout_artists.add(artist)
        holdout_releases.add(release)
        feature_rows.append(
            {"row_id": f"GIO-{expected_position:03d}", "file_path": path}
        )

    leakage = {
        "development_path_overlap": len(holdout_paths & development_paths),
        "development_artist_overlap": len(holdout_artists & development_artists),
        "development_release_overlap": len(
            holdout_releases & development_releases
        ),
        "accepted_truth_path_overlap": len(holdout_paths & reviewed_paths),
        "consumed_holdout_path_overlap": len(holdout_paths & consumed_paths),
        "consumed_holdout_artist_overlap": len(
            holdout_artists & consumed_artists
        ),
        "consumed_holdout_release_overlap": len(
            holdout_releases & consumed_releases
        ),
        "research_playlist_track_overlap": len(
            holdout_track_ids & playlist_track_ids
        ),
        "missing_files": missing_files,
    }
    if any(leakage.values()):
        raise ValueError(
            f"holdout identity or availability leakage detected: {leakage}"
        )
    return feature_rows, leakage


def run(args: argparse.Namespace) -> dict[str, Any]:
    paths = {
        "holdout": args.holdout,
        "audit_manifest": args.audit_manifest,
        "development_manifest": args.development_manifest,
        "development_features": args.development_features,
        "development_corpus": args.development_corpus,
        "consumed_holdout": args.consumed_holdout,
    }
    observed = {name: evaluation.sha256_file(path) for name, path in paths.items()}
    if observed != EXPECTED_INPUT_SHA256:
        raise ValueError("open-set holdout preparation inputs differ")
    holdout = json.loads(args.holdout.read_text(encoding="utf-8"))
    if holdout.get("roster_sha256") != EXPECTED_ROSTER_SHA256:
        raise ValueError("open-set holdout roster fingerprint differs")
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = json.loads(
        args.development_manifest.read_text(encoding="utf-8")
    )
    development_features = json.loads(
        args.development_features.read_text(encoding="utf-8")
    )
    development_corpus = json.loads(
        args.development_corpus.read_text(encoding="utf-8")
    )
    consumed = json.loads(args.consumed_holdout.read_text(encoding="utf-8"))
    tracks, playlists = library.load_library_snapshot(args.sqlcipher, args.database)
    snapshot = {
        "tracks": tracks,
        "playlists": {key: sorted(value) for key, value in sorted(playlists.items())},
    }
    snapshot_sha = corpus.fingerprint(snapshot)
    expected_snapshot_sha = str(holdout["inputs"]["library_snapshot_sha256"])
    if snapshot_sha != expected_snapshot_sha:
        raise ValueError("live library differs from the sealed holdout snapshot")
    exclusion_playlists = set(audit["exclusion_playlists"])
    exclusion_playlists.add("genre_verified")
    rows, leakage = prepare_rows(
        holdout["selected"],
        audit["rows"],
        development["rows"],
        development_features["rows"],
        development_corpus["rows"],
        consumed["selected"],
        tracks,
        playlists,
        exclusion_playlists,
    )
    feature_manifest = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "stage": "private_label_blind_feature_input",
        "roster_sha256": EXPECTED_ROSTER_SHA256,
        "source_holdout_sha256": observed["holdout"],
        "rows": rows,
    }
    evaluation.atomic_write(args.output_feature_manifest, feature_manifest)
    feature_sha = evaluation.sha256_file(args.output_feature_manifest)
    representation_manifest = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "stage": "frozen_label_blind_representation_input",
        "corpus_fingerprint": f"sha256:{EXPECTED_ROSTER_SHA256}",
        "source_manifest_sha256": feature_sha,
        "rows": rows,
    }
    evaluation.atomic_write(
        args.output_representation_manifest, representation_manifest
    )
    summary = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": "audited_label_blind_holdout_input_preparation",
        "inputs": observed,
        "library_snapshot_sha256": snapshot_sha,
        "roster_sha256": EXPECTED_ROSTER_SHA256,
        "rows": len(rows),
        "feature_manifest_sha256": feature_sha,
        "representation_manifest_sha256": evaluation.sha256_file(
            args.output_representation_manifest
        ),
        "leakage": leakage,
        "exclusion_playlists_checked": sorted(exclusion_playlists),
        "identity_values_exposed": False,
    }
    evaluation.atomic_write(args.output_summary, summary)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--holdout", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--development-features", required=True, type=Path)
    parser.add_argument("--development-corpus", required=True, type=Path)
    parser.add_argument("--consumed-holdout", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    parser.add_argument("--output-feature-manifest", required=True, type=Path)
    parser.add_argument("--output-representation-manifest", required=True, type=Path)
    parser.add_argument("--output-summary", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    print(
        json.dumps(
            {
                "rows": result["rows"],
                "feature_manifest_sha256": result["feature_manifest_sha256"],
                "representation_manifest_sha256": result[
                    "representation_manifest_sha256"
                ],
                "library_snapshot_sha256": result["library_snapshot_sha256"],
                "leakage": result["leakage"],
                "identity_values_exposed": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
