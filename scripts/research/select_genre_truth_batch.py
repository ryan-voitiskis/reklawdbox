#!/usr/bin/env python3
"""Select the first blind Genre Intelligence truth-expansion batch."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any


SOURCE_EXPERIMENT_ID = "scoped-broad-genre-mvp-holdout-v1"
SOURCE_ARTIFACT_SHA256 = (
    "7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f"
)
EXPERIMENT_ID = "genre-intelligence-truth-v1-b01"
SEED = EXPERIMENT_ID
QUOTAS = {"Breakbeat": 3, "Pop": 1, "Trance": 2}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_hash(row: dict[str, Any]) -> str:
    value = "|".join(
        (
            SEED,
            str(row["broad_sampling_stratum"]),
            str(row["track_id"]),
            str(row["file_path"]),
        )
    )
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def private_fingerprint(value: Any) -> str:
    payload = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def select_batch(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    used_paths: set[str] = set()
    used_artists: set[str] = set()
    used_releases: set[str] = set()

    for stratum, count in sorted(QUOTAS.items()):
        candidates = sorted(
            (
                row
                for row in rows
                if row.get("broad_sampling_stratum") == stratum
            ),
            key=stable_hash,
        )
        accepted = 0
        for row in candidates:
            path = str(row["file_path"])
            artist_group = str(row["artist_group"])
            release_group = str(row["release_group"])
            if (
                path in used_paths
                or artist_group in used_artists
                or release_group in used_releases
            ):
                continue
            selected.append(dict(row))
            used_paths.add(path)
            used_artists.add(artist_group)
            used_releases.add(release_group)
            accepted += 1
            if accepted == count:
                break
        if accepted != count:
            raise ValueError(
                f"sampling stratum {stratum!r} produced {accepted} distinct rows; "
                f"required {count}"
            )

    selected.sort(
        key=lambda row: hashlib.sha256(
            f"{SEED}|review-order|{row['track_id']}|{row['file_path']}".encode()
        ).hexdigest()
    )
    return selected


def private_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GI01-{position:02d}",
        "track_id": row["track_id"],
        "file_path": row["file_path"],
        "artist": row["artist"],
        "title": row["title"],
        "album": row.get("album"),
        "artist_group": row["artist_group"],
        "release_group": row["release_group"],
        "sampling_stratum_private": row["broad_sampling_stratum"],
        "source_code": row["code"],
    }


def build_result(source: dict[str, Any], source_sha256: str) -> dict[str, Any]:
    if source.get("experiment_id") != SOURCE_EXPERIMENT_ID:
        raise ValueError("unexpected retired-roster experiment ID")
    if source_sha256 != SOURCE_ARTIFACT_SHA256:
        raise ValueError(
            "retired-roster artifact identity changed: "
            f"expected {SOURCE_ARTIFACT_SHA256}, got {source_sha256}"
        )
    rows = source.get("selected")
    if not isinstance(rows, list):
        raise ValueError("retired roster has no selected row list")
    selected = [
        private_row(row, position)
        for position, row in enumerate(select_batch(rows), start=1)
    ]
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "blind_development_truth_review_pending",
        "source": {
            "artifact_sha256": source_sha256,
            "historical_experiment_id": SOURCE_EXPERIMENT_ID,
            "role": "retired_experiment_roster_not_holdout_evidence",
            "model_predictions_used": False,
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(SEED.encode()).hexdigest(),
            "fixed_sampling_quotas": QUOTAS,
            "one_per_path": True,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "sampling_strata_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "roster_sha256": private_fingerprint(selected),
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--retired-roster", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    source_sha256 = sha256_file(args.retired_roster)
    source = json.loads(args.retired_roster.read_text(encoding="utf-8"))
    result = build_result(source, source_sha256)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    print(
        json.dumps(
            {
                "experiment_id": result["experiment_id"],
                "method_status": result["method_status"],
                "output": str(args.output),
                "roster_sha256": result["roster_sha256"],
                "rows": len(result["selected"]),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
