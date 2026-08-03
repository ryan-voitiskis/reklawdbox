#!/usr/bin/env python3
"""Prepare a Plan 066-compatible label-blind manifest for Plan 068."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import tempfile
from pathlib import Path
from typing import Any

import prepare_genre_intelligence_candidate as candidate


EXPERIMENT_ID = "genre-intelligence-v1-candidate-b-openl3"
EXPECTED_INPUT_SHA256 = (
    "d50519a80812a8f5705a8db834ca2764618f0fde18d3ce99ad8e981724c60e24"
)
EXPECTED_ROWS = 575


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def prepare(source: dict[str, Any], source_sha256: str) -> dict[str, Any]:
    if source_sha256 != EXPECTED_INPUT_SHA256:
        raise ValueError("label-blind feature manifest SHA-256 differs")
    if source.get("stage") != "private_label_blind_feature_input":
        raise ValueError("source is not a label-blind feature manifest")
    if source.get("model_ready_corpus_fingerprint") != candidate.EXPECTED_MODEL_FINGERPRINT:
        raise ValueError("model-ready corpus fingerprint differs")
    rows = source.get("rows")
    if not isinstance(rows, list) or len(rows) != EXPECTED_ROWS:
        raise ValueError("label-blind feature row count differs")
    for row in rows:
        if set(row) != {"row_id", "file_path"}:
            raise ValueError("representation source row contains non-identity fields")
    return {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "stage": "frozen_label_blind_representation_input",
        "corpus_fingerprint": candidate.EXPECTED_MODEL_FINGERPRINT,
        "source_manifest_sha256": source_sha256,
        "rows": rows,
    }


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
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source_sha = sha256_file(args.source)
    result = prepare(json.loads(args.source.read_text(encoding="utf-8")), source_sha)
    atomic_write(args.output, result)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "output_sha256": sha256_file(args.output),
                "rows": len(result["rows"]),
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
