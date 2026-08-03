#!/usr/bin/env python3
"""Prepare private label-blind inputs for the Plan 070 release holdout."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from pathlib import Path
from typing import Any


EXPERIMENT_ID = "genre-intelligence-v1-supported-parent-preview"
EXPECTED_HOLDOUT_SHA256 = (
    "1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37"
)
EXPECTED_HOLDOUT_ROSTER_SHA256 = (
    "e90b400645d89b287aab4300465fd0893314830bc6ec8b6ab22b5f9de4fbfdf9"
)
EXPECTED_AUDIT_SHA256 = (
    "c45eea0041388c80bb88fdcbc4abdf648c3fcbbd59347d8958a473a78695ca01"
)
EXPECTED_DEVELOPMENT_SHA256 = (
    "caf76dbe8156943a139a8ab73e8d8b492a1d74bfe1b1e9c80898104ff21f5580"
)
EXPECTED_DEVELOPMENT_FEATURES_SHA256 = (
    "d50519a80812a8f5705a8db834ca2764618f0fde18d3ce99ad8e981724c60e24"
)
EXPECTED_CORPUS_SHA256 = (
    "0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1"
)
EXPECTED_ROWS = 60


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def unique_by(rows: list[dict[str, Any]], field: str, label: str) -> dict[str, dict]:
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
) -> tuple[list[dict[str, str]], dict[str, int]]:
    if len(selected) != EXPECTED_ROWS:
        raise ValueError(f"holdout must contain exactly {EXPECTED_ROWS} rows")
    if len(development_rows) != len(development_feature_rows):
        raise ValueError("development truth and feature row counts differ")
    audit_by_path = unique_by(audit_rows, "file_path", "audit manifest")
    development_by_id = unique_by(development_rows, "row_id", "development")
    feature_by_id = unique_by(
        development_feature_rows, "row_id", "development feature manifest"
    )
    if set(development_by_id) != set(feature_by_id):
        raise ValueError("development truth and feature row identities differ")

    development_paths = {
        str(feature_by_id[row_id]["file_path"]) for row_id in development_by_id
    }
    development_artists = {
        str(row["artist_group"]) for row in development_rows
    }
    development_releases = {
        str(row["release_group"]) for row in development_rows
    }
    reviewed_paths = {str(row["file_path"]) for row in corpus_rows}

    feature_rows = []
    holdout_paths: set[str] = set()
    holdout_artists: set[str] = set()
    holdout_releases: set[str] = set()
    for expected_position, row in enumerate(selected, start=1):
        if int(row["position"]) != expected_position:
            raise ValueError("holdout positions are not contiguous")
        path = str(row["file_path"])
        artist = str(row["artist_group"])
        release = str(row["release_group"])
        if not path or path in holdout_paths:
            raise ValueError("holdout contains a blank or duplicate path")
        if not artist or artist in holdout_artists:
            raise ValueError("holdout contains a blank or duplicate artist group")
        if not release or release in holdout_releases:
            raise ValueError("holdout contains a blank or duplicate release group")
        holdout_paths.add(path)
        holdout_artists.add(artist)
        holdout_releases.add(release)
        audit = audit_by_path.get(path)
        if audit is None or str(audit["track_id"]) != str(row["track_id"]):
            raise ValueError("holdout identity does not match the frozen audit")
        feature_rows.append(
            {"row_id": f"GIH-{expected_position:03d}", "file_path": path}
        )

    leakage = {
        "development_path_overlap": len(holdout_paths & development_paths),
        "development_artist_overlap": len(holdout_artists & development_artists),
        "development_release_overlap": len(holdout_releases & development_releases),
        "accepted_truth_path_overlap": len(holdout_paths & reviewed_paths),
    }
    if any(leakage.values()):
        raise ValueError(f"holdout leakage detected: {leakage}")
    return feature_rows, leakage


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


def run(args: argparse.Namespace) -> dict[str, Any]:
    observed = {
        "holdout_sha256": sha256_file(args.holdout),
        "audit_sha256": sha256_file(args.audit_manifest),
        "development_sha256": sha256_file(args.development_manifest),
        "development_features_sha256": sha256_file(args.development_features),
        "corpus_sha256": sha256_file(args.corpus),
    }
    expected = {
        "holdout_sha256": EXPECTED_HOLDOUT_SHA256,
        "audit_sha256": EXPECTED_AUDIT_SHA256,
        "development_sha256": EXPECTED_DEVELOPMENT_SHA256,
        "development_features_sha256": EXPECTED_DEVELOPMENT_FEATURES_SHA256,
        "corpus_sha256": EXPECTED_CORPUS_SHA256,
    }
    if observed != expected:
        raise ValueError(f"holdout preparation inputs changed: {observed}")

    holdout = json.loads(args.holdout.read_text(encoding="utf-8"))
    if holdout.get("roster_sha256") != EXPECTED_HOLDOUT_ROSTER_SHA256:
        raise ValueError("holdout roster SHA-256 changed")
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    development_features = json.loads(
        args.development_features.read_text(encoding="utf-8")
    )
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    rows, leakage = prepare_rows(
        holdout["selected"],
        audit["rows"],
        development["rows"],
        development_features["rows"],
        corpus["rows"],
    )
    missing_files = sum(not Path(row["file_path"]).is_file() for row in rows)
    if missing_files:
        raise ValueError(f"holdout contains {missing_files} missing audio files")

    feature_manifest = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "stage": "private_label_blind_feature_input",
        "roster_sha256": EXPECTED_HOLDOUT_ROSTER_SHA256,
        "source_holdout_sha256": observed["holdout_sha256"],
        "rows": rows,
    }
    atomic_write(args.output_feature_manifest, feature_manifest)
    feature_manifest_sha = sha256_file(args.output_feature_manifest)
    representation_manifest = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "stage": "frozen_label_blind_representation_input",
        "corpus_fingerprint": f"sha256:{EXPECTED_HOLDOUT_ROSTER_SHA256}",
        "source_manifest_sha256": feature_manifest_sha,
        "rows": rows,
    }
    atomic_write(args.output_representation_manifest, representation_manifest)
    summary = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": "frozen_label_blind_holdout_input_preparation",
        "rows": len(rows),
        "inputs": observed,
        "roster_sha256": EXPECTED_HOLDOUT_ROSTER_SHA256,
        "feature_manifest_sha256": feature_manifest_sha,
        "representation_manifest_sha256": sha256_file(
            args.output_representation_manifest
        ),
        "leakage": leakage,
        "missing_files": missing_files,
        "identity_values_exposed": False,
    }
    atomic_write(args.output_summary, summary)
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--holdout", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--development-features", required=True, type=Path)
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--output-feature-manifest", required=True, type=Path)
    parser.add_argument("--output-representation-manifest", required=True, type=Path)
    parser.add_argument("--output-summary", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
