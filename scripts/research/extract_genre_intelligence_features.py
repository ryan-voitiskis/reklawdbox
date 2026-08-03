#!/usr/bin/env python3
"""Extract Plan 068 candidate-A features without reading truth labels."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import build_genre_intelligence_corpus as taxonomy
import extract_kick_rhythm_features as kick


EXPERIMENT_ID = "genre-intelligence-v1-candidate-a"
STRATUM_ANALYZER = "stratum-dsp"
STRATUM_VERSION = "21"
ESSENTIA_ANALYZER = "essentia"
ESSENTIA_VERSION = "3"
RELEASE_SCOPE = [
    "House",
    "Ambient",
    "Techno",
    "Breakbeat",
    "Reggae",
    "Electro",
    "Trance",
]
PROFILE_FEATURES = [
    ("stratum_bpm", "stratum", "bpm"),
    ("danceability", "essentia", "danceability"),
    ("onset_rate", "essentia", "onset_rate"),
    ("rhythm_regularity", "essentia", "rhythm_regularity"),
    ("spectral_centroid_mean", "essentia", "spectral_centroid_mean"),
    ("spectral_centroid_cv", "essentia", "spectral_centroid_cv"),
    ("dynamic_complexity", "essentia", "dynamic_complexity"),
    ("loudness_integrated", "essentia", "loudness_integrated"),
    ("decay_mid_tau", "stratum", "decay_mid_tau"),
    ("decay_high_tau", "stratum", "decay_high_tau"),
    ("spectral_flux_mean", "essentia", "spectral_flux_mean"),
    ("dissonance_mean", "essentia", "dissonance_mean"),
    ("key_clarity", "stratum", "key_clarity"),
]
TIMBRAL_FEATURES = [
    *(f"mfcc_mean_{index}" for index in range(1, 9)),
    *(f"mfcc_std_{index}" for index in range(1, 6)),
    "spectral_contrast_0",
    "spectral_contrast_2",
    "spectral_contrast_4",
]
VALUE_FEATURES = [name for name, _, _ in PROFILE_FEATURES] + TIMBRAL_FEATURES
MISSINGNESS_FEATURES = [f"{name}_available" for name in VALUE_FEATURES]
BASELINE_FEATURES = [f"v033_{target}" for target in RELEASE_SCOPE] + [
    "v033_unknown"
]
FEATURE_NAMES = [
    *VALUE_FEATURES,
    *MISSINGNESS_FEATURES,
    *(f"kick_{index:02d}" for index in range(kick.FEATURE_COUNT)),
    *BASELINE_FEATURES,
]
FEATURE_SCHEMA = "genre-intelligence-candidate-a-features-v1"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def finite(value: Any) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return float("nan")
    result = float(value)
    return result if np.isfinite(result) else float("nan")


def vector_values(stratum: dict[str, Any], essentia: dict[str, Any]) -> np.ndarray:
    sources = {"stratum": stratum, "essentia": essentia}
    values = [finite(sources[source].get(field)) for _, source, field in PROFILE_FEATURES]

    mfcc_mean = essentia.get("mfcc_mean")
    mfcc_std = essentia.get("mfcc_std")
    contrast = essentia.get("spectral_contrast_mean")
    values.extend(
        finite(mfcc_mean[index])
        if isinstance(mfcc_mean, list) and len(mfcc_mean) > index
        else float("nan")
        for index in range(1, 9)
    )
    values.extend(
        finite(mfcc_std[index])
        if isinstance(mfcc_std, list) and len(mfcc_std) > index
        else float("nan")
        for index in range(1, 6)
    )
    values.extend(
        finite(contrast[index])
        if isinstance(contrast, list) and len(contrast) > index
        else float("nan")
        for index in (0, 2, 4)
    )
    result = np.asarray(values, dtype=np.float64)
    if result.shape != (len(VALUE_FEATURES),):
        raise ValueError("scalar and timbral feature shape differs")
    return result


def baseline_parent(value: Any) -> str | None:
    if not isinstance(value, str) or not value.strip():
        return None
    return taxonomy.FINE_TO_PARENT.get(value.strip())


def baseline_by_path(manifests: list[dict[str, Any]]) -> dict[str, str | None]:
    result: dict[str, str | None] = {}
    for manifest in manifests:
        rows = manifest.get("rows")
        if not isinstance(rows, list):
            raise ValueError("baseline manifest has no rows")
        for row in rows:
            path = str(row["file_path"])
            parent = baseline_parent(row.get("baseline_recommendation"))
            if path in result and result[path] != parent:
                raise ValueError(f"baseline manifests disagree for path: {path}")
            result[path] = parent
    return result


def read_cache(
    connection: sqlite3.Connection, path: str, analyzer: str, version: str
) -> tuple[dict[str, Any], str, str]:
    row = connection.execute(
        "SELECT analysis_version, input_fingerprint, features_json "
        "FROM audio_analysis_cache WHERE file_path = ? AND analyzer = ?",
        (path, analyzer),
    ).fetchone()
    if row is None:
        raise ValueError(f"missing {analyzer} cache row")
    observed_version, input_fingerprint, features_json = row
    if observed_version != version:
        raise ValueError(
            f"{analyzer} cache version {observed_version!r} differs from {version!r}"
        )
    return json.loads(features_json), str(input_fingerprint), str(features_json)


def extract(
    database: Path,
    rows: list[dict[str, Any]],
    baselines: dict[str, str | None],
) -> tuple[np.ndarray, dict[str, Any]]:
    vectors = []
    source_snapshot = hashlib.sha256()
    baseline_counts: Counter[str] = Counter()
    connection = sqlite3.connect(f"{database.resolve().as_uri()}?mode=ro", uri=True)
    try:
        for row_index, row in enumerate(rows):
            if set(row) != {"row_id", "file_path"}:
                raise ValueError("feature manifest row contains non-identity fields")
            path = str(row["file_path"])
            stratum, stratum_input, stratum_json = read_cache(
                connection, path, STRATUM_ANALYZER, STRATUM_VERSION
            )
            essentia, essentia_input, essentia_json = read_cache(
                connection, path, ESSENTIA_ANALYZER, ESSENTIA_VERSION
            )
            values = vector_values(stratum, essentia)
            available = np.isfinite(values).astype(np.float64)
            kick_values = kick.kick_vector(stratum)
            parent = baselines.get(path)
            baseline = np.zeros(len(BASELINE_FEATURES), dtype=np.float64)
            baseline_index = (
                RELEASE_SCOPE.index(parent)
                if parent in RELEASE_SCOPE
                else len(RELEASE_SCOPE)
            )
            baseline[baseline_index] = 1.0
            baseline_counts[parent if parent in RELEASE_SCOPE else "<unknown>"] += 1
            vector = np.concatenate([values, available, kick_values, baseline])
            if vector.shape != (len(FEATURE_NAMES),):
                raise ValueError("candidate feature vector shape differs")
            vectors.append(vector)
            for value in (
                str(row_index),
                str(row["row_id"]),
                path,
                STRATUM_VERSION,
                stratum_input,
                stratum_json,
                ESSENTIA_VERSION,
                essentia_input,
                essentia_json,
                parent or "<unknown>",
            ):
                source_snapshot.update(value.encode())
                source_snapshot.update(b"\0")
            source_snapshot.update(b"\n")
    finally:
        connection.close()

    matrix = np.vstack(vectors).astype("<f8", copy=False)
    semantic = hashlib.sha256()
    semantic.update(FEATURE_SCHEMA.encode())
    semantic.update(b"\n")
    semantic.update("\n".join(FEATURE_NAMES).encode())
    semantic.update(b"\n")
    semantic.update(matrix.tobytes(order="C"))
    summary = {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "frozen_label_blind_cache_feature_extraction",
        "feature_schema": FEATURE_SCHEMA,
        "feature_names": FEATURE_NAMES,
        "feature_count": len(FEATURE_NAMES),
        "rows": len(matrix),
        "value_feature_count": len(VALUE_FEATURES),
        "fully_available_value_rows": int(
            np.sum(np.all(np.isfinite(matrix[:, : len(VALUE_FEATURES)]), axis=1))
        ),
        "kick_available_rows": int(
            np.sum(matrix[:, len(VALUE_FEATURES) * 2] == 1.0)
        ),
        "baseline_counts": dict(sorted(baseline_counts.items())),
        "source_snapshot_sha256": source_snapshot.hexdigest(),
        "feature_semantic_sha256": semantic.hexdigest(),
        "stratum_version": STRATUM_VERSION,
        "essentia_version": ESSENTIA_VERSION,
    }
    return matrix, summary


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def atomic_save(path: Path, matrix: np.ndarray) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".npy"
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        np.save(temporary, matrix, allow_pickle=False)
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--baseline-manifest", action="append", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    if manifest.get("stage") != "private_label_blind_feature_input":
        raise ValueError("input is not a label-blind feature manifest")
    baselines = baseline_by_path(
        [json.loads(path.read_text(encoding="utf-8")) for path in args.baseline_manifest]
    )
    matrix, summary = extract(args.database, manifest["rows"], baselines)
    atomic_save(args.output, matrix)
    summary.update(
        {
            "manifest_sha256": sha256_file(args.manifest),
            "baseline_manifest_sha256": [
                sha256_file(path) for path in args.baseline_manifest
            ],
            "extractor_source_sha256": sha256_file(Path(__file__)),
            "artifact_sha256": sha256_file(args.output),
        }
    )
    atomic_write_json(args.summary, summary)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
