#!/usr/bin/env python3
"""Extract frozen Plan 065 kick-rhythm features from the read-only cache."""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np


FEATURE_SCHEMA = "kick-rhythm-v1:available,patterns5,confidence,kicks_per_bar,bases2,l1_histogram64"
ANALYZER = "stratum-dsp"
ANALYSIS_VERSION = "21"
PATTERNS = ["four_on_floor", "broken_beat", "halftime", "sparse", "irregular"]
RATE_BASES = ["main_groove", "track"]
FEATURE_COUNT = 74


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def finite_non_negative(value: Any, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be numeric")
    result = float(value)
    if not np.isfinite(result) or result < 0.0:
        raise ValueError(f"{field} must be finite and non-negative")
    return result


def kick_vector(features: dict[str, Any]) -> np.ndarray:
    pattern = features.get("kick_pattern")
    if pattern is None:
        return np.zeros(FEATURE_COUNT, dtype=np.float64)
    if pattern not in PATTERNS:
        raise ValueError(f"unknown kick_pattern: {pattern!r}")
    rate_basis = features.get("kick_rate_basis")
    if rate_basis not in RATE_BASES:
        raise ValueError(f"unknown kick_rate_basis: {rate_basis!r}")
    confidence = finite_non_negative(
        features.get("kick_pattern_confidence"), "kick_pattern_confidence"
    )
    kicks_per_bar = finite_non_negative(
        features.get("kick_kicks_per_bar"), "kick_kicks_per_bar"
    )
    histogram_raw = features.get("kick_histogram")
    if not isinstance(histogram_raw, list) or len(histogram_raw) != 64:
        raise ValueError("kick_histogram must contain exactly 64 values")
    histogram = np.asarray(
        [finite_non_negative(value, "kick_histogram") for value in histogram_raw],
        dtype=np.float64,
    )
    total = float(np.sum(histogram))
    if total > 0.0:
        histogram /= total

    result = np.zeros(FEATURE_COUNT, dtype=np.float64)
    result[0] = 1.0
    result[1 + PATTERNS.index(pattern)] = 1.0
    result[6] = confidence
    result[7] = kicks_per_bar
    result[8 + RATE_BASES.index(rate_basis)] = 1.0
    result[10:] = histogram
    return result


def read_only_connection(database: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)


def extract_rows(
    database: Path, rows: list[dict[str, Any]]
) -> tuple[np.ndarray, dict[str, Any]]:
    vectors: list[np.ndarray] = []
    snapshot = hashlib.sha256()
    pattern_counts: Counter[str] = Counter()
    rate_basis_counts: Counter[str] = Counter()
    connection = read_only_connection(database)
    try:
        for row_index, row in enumerate(rows):
            file_path = row["file_path"]
            cached = connection.execute(
                "SELECT analysis_version, input_fingerprint, features_json "
                "FROM audio_analysis_cache "
                "WHERE file_path = ? AND analyzer = ?",
                (file_path, ANALYZER),
            ).fetchone()
            if cached is None:
                raise ValueError(f"missing {ANALYZER} cache row at manifest index {row_index}")
            analysis_version, input_fingerprint, features_json = cached
            if analysis_version != ANALYSIS_VERSION:
                raise ValueError(
                    f"Stratum version differs at manifest index {row_index}: "
                    f"{analysis_version!r}"
                )
            features = json.loads(features_json)
            vector = kick_vector(features)
            vectors.append(vector)
            pattern_counts[str(features.get("kick_pattern") or "<missing>")] += 1
            rate_basis_counts[str(features.get("kick_rate_basis") or "<missing>")] += 1
            snapshot.update(str(row_index).encode())
            snapshot.update(b"\0")
            snapshot.update(file_path.encode())
            snapshot.update(b"\0")
            snapshot.update(str(analysis_version).encode())
            snapshot.update(b"\0")
            snapshot.update(str(input_fingerprint).encode())
            snapshot.update(b"\0")
            snapshot.update(features_json.encode())
            snapshot.update(b"\n")
    finally:
        connection.close()

    matrix = np.vstack(vectors).astype("<f8", copy=False)
    semantic = hashlib.sha256()
    semantic.update(FEATURE_SCHEMA.encode())
    semantic.update(b"\n")
    semantic.update(str(len(matrix)).encode())
    semantic.update(b"\n")
    semantic.update(matrix.tobytes(order="C"))
    summary = {
        "feature_schema": FEATURE_SCHEMA,
        "feature_count": FEATURE_COUNT,
        "rows": len(matrix),
        "available_rows": int(np.sum(matrix[:, 0] == 1.0)),
        "missing_rows": int(np.sum(matrix[:, 0] == 0.0)),
        "pattern_counts": dict(sorted(pattern_counts.items())),
        "rate_basis_counts": dict(sorted(rate_basis_counts.items())),
        "source_snapshot_sha256": snapshot.hexdigest(),
        "feature_semantic_sha256": semantic.hexdigest(),
    }
    return matrix, summary


def run(args: argparse.Namespace) -> dict[str, Any]:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    rows = manifest["rows"]
    matrix, summary = extract_rows(args.database, rows)
    np.savez_compressed(
        args.output,
        kick_features=matrix,
        feature_schema=np.asarray(FEATURE_SCHEMA),
    )
    summary.update(
        {
            "manifest_sha256": sha256_file(args.manifest),
            "artifact_sha256": sha256_file(args.output),
            "analyzer": ANALYZER,
            "analysis_version": ANALYSIS_VERSION,
        }
    )
    args.summary.write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
