#!/usr/bin/env python3
"""Verify decoded-audio isolation for the private Plan 070 holdout."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

import evaluate_genre_intelligence_candidate as candidate


EXPERIMENT_ID = "genre-intelligence-v1-supported-parent-preview"
EXPECTED_DEVELOPMENT_MANIFEST_SHA256 = (
    "d50519a80812a8f5705a8db834ca2764618f0fde18d3ce99ad8e981724c60e24"
)
EXPECTED_HOLDOUT_MANIFEST_SHA256 = (
    "d970b54a4da4c370e9a5a21524393c4845c6b70ed5a09276540f09a4dbb0c152"
)
EXPECTED_DEVELOPMENT_ROWS = 575
EXPECTED_HOLDOUT_ROWS = 60


def decoded_hash(path: str, ffmpeg: str) -> str:
    try:
        completed = subprocess.run(
            [
                ffmpeg,
                "-v",
                "error",
                "-nostdin",
                "-i",
                path,
                "-map",
                "a:0",
                "-ac",
                "1",
                "-ar",
                "48000",
                "-f",
                "hash",
                "-hash",
                "sha256",
                "-",
            ],
            capture_output=True,
            check=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        raise ValueError("decoded-audio hash command failed") from None
    value = completed.stdout.strip()
    prefix = "SHA256="
    if not value.startswith(prefix) or len(value) != len(prefix) + 64:
        raise ValueError("decoded-audio hash output is malformed")
    result = value[len(prefix) :].lower()
    if any(character not in "0123456789abcdef" for character in result):
        raise ValueError("decoded-audio hash is not hexadecimal")
    return result


def hash_paths(paths: list[str], ffmpeg: str, workers: int) -> list[str]:
    if workers < 1 or workers > 8:
        raise ValueError("workers must be between one and eight")
    with ThreadPoolExecutor(max_workers=workers) as executor:
        return list(executor.map(lambda path: decoded_hash(path, ffmpeg), paths))


def ordered_digest(values: list[str]) -> str:
    digest = hashlib.sha256()
    for value in values:
        digest.update(value.encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()


def audit_hashes(development: list[str], holdout: list[str]) -> dict[str, Any]:
    development_set = set(development)
    holdout_set = set(holdout)
    overlap = development_set & holdout_set
    return {
        "development_rows": len(development),
        "development_unique_decoded_audio": len(development_set),
        "development_ordered_hash_sha256": ordered_digest(development),
        "holdout_rows": len(holdout),
        "holdout_unique_decoded_audio": len(holdout_set),
        "holdout_ordered_hash_sha256": ordered_digest(holdout),
        "cross_partition_decoded_audio_overlap": len(overlap),
        "passed": not overlap,
    }


def manifest_paths(path: Path, expected_sha256: str, expected_rows: int) -> list[str]:
    if candidate.sha256_file(path) != expected_sha256:
        raise ValueError("decoded-audio input manifest SHA-256 changed")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    rows = manifest.get("rows")
    if not isinstance(rows, list) or len(rows) != expected_rows:
        raise ValueError("decoded-audio input manifest row count changed")
    paths = [str(row["file_path"]) for row in rows]
    if len(set(paths)) != len(paths) or any(not Path(value).is_file() for value in paths):
        raise ValueError("decoded-audio input paths are duplicate or missing")
    return paths


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--holdout-manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()
    development_paths = manifest_paths(
        args.development_manifest,
        EXPECTED_DEVELOPMENT_MANIFEST_SHA256,
        EXPECTED_DEVELOPMENT_ROWS,
    )
    holdout_paths = manifest_paths(
        args.holdout_manifest,
        EXPECTED_HOLDOUT_MANIFEST_SHA256,
        EXPECTED_HOLDOUT_ROWS,
    )
    if set(development_paths) & set(holdout_paths):
        raise ValueError("decoded-audio manifests contain an exact path overlap")
    development_hashes = hash_paths(development_paths, args.ffmpeg, args.workers)
    holdout_hashes = hash_paths(holdout_paths, args.ffmpeg, args.workers)
    audit = audit_hashes(development_hashes, holdout_hashes)
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": "private_decoded_audio_partition_audit",
        "development_manifest_sha256": EXPECTED_DEVELOPMENT_MANIFEST_SHA256,
        "holdout_manifest_sha256": EXPECTED_HOLDOUT_MANIFEST_SHA256,
        "decoder": {
            "ffmpeg": args.ffmpeg,
            "sample_rate": 48000,
            "channels": 1,
            "hash": "sha256",
        },
        **audit,
        "identity_values_exposed": False,
    }
    candidate.atomic_write(args.output, result)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
