#!/usr/bin/env python3
"""Freeze private open-set development inputs for Plan 071."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


EXPERIMENT_ID = "genre-intelligence-v1-open-set"
FOLD_SEED = "genre-intelligence-v1-open-set-folds-v1"
FOLD_COUNT = 5
EXPECTED_CORPUS_SHA256 = (
    "0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1"
)
EXPECTED_ACCEPTED_FINGERPRINT = (
    "07a754c42ae676eb7f6fcbc02ee1b5748e3153e155311d4559d3d749fbdd6cf1"
)
EXPECTED_ACCEPTED_ROWS = 716
EXPECTED_NON_TARGET_ROWS = 141
OUTPUT_PARENTS = [
    "House",
    "Ambient",
    "Techno",
    "Breakbeat",
    "Reggae",
    "Electro",
    "Trance",
]
OTHER_CLASS = "Other"
FOLD_BUCKETS = [*OUTPUT_PARENTS, OTHER_CLASS]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_hash(value: str) -> str:
    return hashlib.sha256(f"{FOLD_SEED}|{value}".encode()).hexdigest()


def fold_bucket(parent: str) -> str:
    return parent if parent in OUTPUT_PARENTS else OTHER_CLASS


def fold_score(
    counts: list[Counter[str]], sizes: list[int], totals: Counter[str]
) -> float:
    score = 0.0
    for bucket, total in totals.items():
        ideal = total / FOLD_COUNT
        score += sum((fold[bucket] - ideal) ** 2 for fold in counts) / max(
            ideal, 1.0
        )
    ideal_size = sum(totals.values()) / FOLD_COUNT
    score += 0.1 * sum((size - ideal_size) ** 2 for size in sizes) / max(
        ideal_size, 1.0
    )
    return score


def assign_artist_folds(rows: list[dict[str, Any]]) -> dict[str, int]:
    by_artist: dict[str, list[dict[str, Any]]] = defaultdict(list)
    totals: Counter[str] = Counter()
    for row in rows:
        artist = str(row["artist_group"])
        parent = str(row["canonical_parent_genre"])
        if not artist:
            raise ValueError("accepted row has a blank artist group")
        bucket = fold_bucket(parent)
        by_artist[artist].append(row)
        totals[bucket] += 1

    if set(totals) != set(FOLD_BUCKETS):
        raise ValueError("development corpus does not contain every fold bucket")
    group_counts = {
        artist: Counter(
            fold_bucket(str(row["canonical_parent_genre"])) for row in artist_rows
        )
        for artist, artist_rows in by_artist.items()
    }
    ordered = sorted(
        by_artist,
        key=lambda artist: (
            -max(group_counts[artist].values()),
            -len(by_artist[artist]),
            stable_hash(artist),
        ),
    )
    counts = [Counter() for _ in range(FOLD_COUNT)]
    sizes = [0 for _ in range(FOLD_COUNT)]
    assignments: dict[str, int] = {}
    for artist in ordered:
        group = group_counts[artist]
        candidates = []
        for fold in range(FOLD_COUNT):
            candidate_counts = [value.copy() for value in counts]
            candidate_sizes = list(sizes)
            candidate_counts[fold].update(group)
            candidate_sizes[fold] += len(by_artist[artist])
            candidates.append(
                (
                    fold_score(candidate_counts, candidate_sizes, totals),
                    candidate_sizes[fold],
                    stable_hash(f"{artist}|{fold}"),
                    fold,
                )
            )
        selected = min(candidates)[-1]
        assignments[artist] = selected
        counts[selected].update(group)
        sizes[selected] += len(by_artist[artist])

    for fold in range(FOLD_COUNT):
        missing = [bucket for bucket in FOLD_BUCKETS if counts[fold][bucket] == 0]
        if missing:
            raise ValueError(f"fold {fold} is missing buckets: {missing}")
        if counts[fold][OTHER_CLASS] < 20:
            raise ValueError(f"fold {fold} has fewer than twenty non-target rows")
    return assignments


def prepare(source: dict[str, Any], source_sha256: str) -> tuple[dict, dict]:
    if source_sha256 != EXPECTED_CORPUS_SHA256:
        raise ValueError("development corpus SHA-256 differs from the frozen input")
    if source.get("accepted_corpus_fingerprint") != EXPECTED_ACCEPTED_FINGERPRINT:
        raise ValueError("accepted corpus fingerprint differs")
    if source.get("accepted_rows") != EXPECTED_ACCEPTED_ROWS:
        raise ValueError("accepted corpus row count differs")
    if source.get("release_scope") != OUTPUT_PARENTS:
        raise ValueError("output parent order differs from the frozen protocol")
    rows = source.get("rows")
    if not isinstance(rows, list) or len(rows) != EXPECTED_ACCEPTED_ROWS:
        raise ValueError("accepted row payload differs")
    non_target_rows = sum(
        str(row["canonical_parent_genre"]) not in OUTPUT_PARENTS for row in rows
    )
    if non_target_rows != EXPECTED_NON_TARGET_ROWS:
        raise ValueError("non-target row count differs from the frozen protocol")

    assignments = assign_artist_folds(rows)
    development_rows = []
    feature_rows = []
    seen_ids: set[str] = set()
    seen_paths: set[str] = set()
    for row in rows:
        row_id = str(row["row_id"])
        path = str(row["file_path"])
        artist = str(row["artist_group"])
        if row_id in seen_ids or path in seen_paths:
            raise ValueError("open-set manifest contains duplicate identity")
        seen_ids.add(row_id)
        seen_paths.add(path)
        development_rows.append(
            {
                "row_id": row_id,
                "canonical_parent_genre": str(row["canonical_parent_genre"]),
                "artist_group": artist,
                "release_group": str(row["release_group"]),
                "fold": assignments[artist],
            }
        )
        feature_rows.append({"row_id": row_id, "file_path": path})

    common = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "source_corpus_sha256": source_sha256,
        "accepted_corpus_fingerprint": EXPECTED_ACCEPTED_FINGERPRINT,
        "corpus_fingerprint": EXPECTED_ACCEPTED_FINGERPRINT,
        "accepted_rows": EXPECTED_ACCEPTED_ROWS,
        "non_target_rows": non_target_rows,
        "output_parents": OUTPUT_PARENTS,
    }
    development = {
        **common,
        "stage": "private_open_set_development_truth_and_folds",
        "fold_seed": FOLD_SEED,
        "fold_count": FOLD_COUNT,
        "grouping": "normalized_artist_strictly_contains_release_group",
        "rows": development_rows,
    }
    features = {
        **common,
        "stage": "private_label_blind_feature_input",
        "rows": feature_rows,
    }
    return development, features


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
    parser.add_argument("--corpus", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--feature-manifest", required=True, type=Path)
    args = parser.parse_args()
    source_sha = sha256_file(args.corpus)
    development, features = prepare(
        json.loads(args.corpus.read_text(encoding="utf-8")), source_sha
    )
    atomic_write(args.development_manifest, development)
    atomic_write(args.feature_manifest, features)
    fold_counts = Counter(row["fold"] for row in development["rows"])
    fold_non_targets = Counter(
        row["fold"]
        for row in development["rows"]
        if row["canonical_parent_genre"] not in OUTPUT_PARENTS
    )
    print(
        json.dumps(
            {
                "development_manifest_sha256": sha256_file(
                    args.development_manifest
                ),
                "feature_manifest_sha256": sha256_file(args.feature_manifest),
                "rows": len(development["rows"]),
                "non_target_rows": development["non_target_rows"],
                "fold_rows": dict(sorted(fold_counts.items())),
                "fold_non_target_rows": dict(sorted(fold_non_targets.items())),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
