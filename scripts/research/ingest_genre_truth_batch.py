#!/usr/bin/env python3
"""Append a reviewed blind batch to the private Genre Intelligence truth ledger."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any, Callable


SCHEMA_VERSION = 1
PARENT_GENRES = frozenset(
    {
        "Acid",
        "Ambient",
        "Breakbeat",
        "Disco",
        "Downtempo",
        "Drum & Bass",
        "Dubstep",
        "EBM",
        "Electro",
        "Footwork",
        "Garage",
        "Grime",
        "Hardcore",
        "Highlife",
        "Hip Hop",
        "House",
        "IDM",
        "Jazz",
        "Minimal",
        "Pop",
        "R&B",
        "Reggae",
        "Rock",
        "Tech House",
        "Techno",
        "Trance",
    }
)
CONFIDENCE_LEVELS = frozenset({"high", "medium", "low"})
OUTCOMES = frozenset({"label", "ambiguous", "skip"})


def canonical_json_bytes(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def fingerprint(value: Any) -> str:
    return hashlib.sha256(canonical_json_bytes(value)).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def decoded_audio_identity(path: Path, ffmpeg: str) -> dict[str, Any]:
    if not path.is_file():
        raise ValueError(f"audio file is missing: {path}")
    process = subprocess.Popen(
        [
            ffmpeg,
            "-v",
            "error",
            "-nostdin",
            "-i",
            str(path),
            "-map",
            "a:0",
            "-ac",
            "1",
            "-ar",
            "48000",
            "-f",
            "f32le",
            "-",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.stdout is None or process.stderr is None:
        process.kill()
        raise RuntimeError("failed to capture ffmpeg output")
    decoded_digest = hashlib.sha256()
    decoded_bytes = 0
    for chunk in iter(lambda: process.stdout.read(1024 * 1024), b""):
        decoded_digest.update(chunk)
        decoded_bytes += len(chunk)
    stderr = process.stderr.read().decode("utf-8", errors="replace").strip()
    return_code = process.wait()
    if return_code != 0:
        raise ValueError(f"ffmpeg decode failed for {path}: {stderr[:500]}")
    if decoded_bytes == 0:
        raise ValueError(f"ffmpeg decoded no audio for {path}")
    stat = path.stat()
    return {
        "file_sha256": sha256_file(path),
        "file_size": stat.st_size,
        "file_mtime_ns": stat.st_mtime_ns,
        "decoded_pcm_sha256": decoded_digest.hexdigest(),
        "decoded_pcm_bytes": decoded_bytes,
        "decoded_pcm_format": "f32le_mono_48000hz",
    }


def validate_verdict(value: dict[str, Any]) -> dict[str, Any]:
    outcome = str(value.get("outcome", ""))
    if outcome not in OUTCOMES:
        raise ValueError(f"unsupported verdict outcome {outcome!r}")
    confidence = value.get("confidence")
    if confidence is not None and confidence not in CONFIDENCE_LEVELS:
        raise ValueError(f"unsupported confidence {confidence!r}")
    genre = value.get("genre")
    if outcome == "label":
        if genre not in PARENT_GENRES:
            raise ValueError(f"label verdict requires a canonical parent genre: {genre!r}")
        if confidence not in CONFIDENCE_LEVELS:
            raise ValueError("label verdict requires normalized confidence")
    elif genre is not None:
        raise ValueError(f"{outcome} verdict must not contain a genre label")
    alternatives = value.get("alternatives", [])
    if not isinstance(alternatives, list) or any(
        alternative not in PARENT_GENRES for alternative in alternatives
    ):
        raise ValueError("alternatives must be canonical parent genres")
    if len(set(alternatives)) != len(alternatives):
        raise ValueError("alternatives must be unique")
    if genre in alternatives:
        raise ValueError("primary genre must not be repeated as an alternative")
    return {
        "outcome": outcome,
        "canonical_parent_genre": genre,
        "confidence": confidence,
        "confidence_raw": value.get("confidence_raw"),
        "alternatives": alternatives,
        "notes": str(value.get("notes", "")),
    }


def load_ledger(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    records = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        record = json.loads(line)
        if record.get("schema_version") != SCHEMA_VERSION:
            raise ValueError(f"ledger line {line_number} has unsupported schema")
        records.append(record)
    return records


def atomic_write(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def active_records(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    superseded = {
        str(record["supersedes_record_id"])
        for record in records
        if record.get("supersedes_record_id")
    }
    return [record for record in records if record["record_id"] not in superseded]


def build_snapshot(records: list[dict[str, Any]], ledger_sha256: str) -> dict[str, Any]:
    active = active_records(records)
    eligible = [
        record for record in active if record["model_eligibility"]["eligible"]
    ]
    rows = [
        {
            "record_id": record["record_id"],
            "track_id": record["track"]["track_id"],
            "file_path": record["track"]["file_path"],
            "artist": record["track"]["artist"],
            "title": record["track"]["title"],
            "album": record["track"]["album"],
            "artist_group": record["track"]["artist_group"],
            "release_group": record["track"]["release_group"],
            "decoded_pcm_sha256": record["audio_identity"]["decoded_pcm_sha256"],
            "canonical_parent_genre": record["review"]["canonical_parent_genre"],
            "confidence": record["review"]["confidence"],
            "provenance": record["provenance"],
        }
        for record in eligible
    ]
    rows.sort(
        key=lambda row: (
            str(row["canonical_parent_genre"]),
            str(row["artist_group"]),
            str(row["release_group"]),
            str(row["decoded_pcm_sha256"]),
        )
    )
    outcome_counts = Counter(record["review"]["outcome"] for record in active)
    genre_counts = Counter(str(row["canonical_parent_genre"]) for row in rows)
    return {
        "schema_version": SCHEMA_VERSION,
        "corpus_version": "genre-intelligence-truth-v1",
        "source_ledger_sha256": ledger_sha256,
        "active_review_records": len(active),
        "model_eligible_rows": len(rows),
        "outcome_counts": dict(sorted(outcome_counts.items())),
        "genre_counts": dict(sorted(genre_counts.items())),
        "corpus_fingerprint": fingerprint(rows),
        "rows": rows,
    }


def ingest(
    mapping: dict[str, Any],
    verdicts: dict[str, Any],
    ledger_path: Path,
    snapshot_path: Path,
    ffmpeg: str,
    identity_loader: Callable[[Path, str], dict[str, Any]] = decoded_audio_identity,
) -> dict[str, Any]:
    if mapping.get("experiment_id") != verdicts.get("batch_id"):
        raise ValueError("mapping experiment ID and verdict batch ID differ")
    selected = mapping.get("selected")
    verdict_rows = verdicts.get("rows")
    if not isinstance(selected, list) or not 1 <= len(selected) <= 20:
        raise ValueError("mapping must contain one to twenty selected rows")
    if not isinstance(verdict_rows, list):
        raise ValueError("verdict file has no row list")
    selected_by_code = {str(row["code"]): row for row in selected}
    verdict_by_code = {str(row["code"]): row for row in verdict_rows}
    if len(selected_by_code) != len(selected) or len(verdict_by_code) != len(verdict_rows):
        raise ValueError("mapping and verdict codes must be unique")
    if set(selected_by_code) != set(verdict_by_code):
        raise ValueError("verdicts must cover every selected code exactly once")

    proposed = []
    for row in sorted(selected, key=lambda item: int(item["position"])):
        verdict = verdict_by_code[str(row["code"])]
        review = validate_verdict(verdict)
        audio_identity = identity_loader(Path(str(row["file_path"])), ffmpeg)
        eligible = (
            review["outcome"] == "label"
            and review["confidence"] in {"high", "medium"}
        )
        record = {
            "schema_version": SCHEMA_VERSION,
            "batch_id": mapping["experiment_id"],
            "batch_roster_sha256": mapping["roster_sha256"],
            "code": row["code"],
            "track": {
                "track_id": row["track_id"],
                "file_path": row["file_path"],
                "artist": row["artist"],
                "title": row["title"],
                "album": row.get("album"),
                "artist_group": row["artist_group"],
                "release_group": row["release_group"],
            },
            "audio_identity": audio_identity,
            "review": review,
            "provenance": {
                "kind": "operator_blind_review",
                "reviewer": verdicts["reviewer"],
                "reviewed_at": verdicts["reviewed_at"],
                "hidden_sampling_and_model_fields": True,
            },
            "model_eligibility": {
                "eligible": eligible,
                "reason": (
                    "explicit_high_or_medium_parent_genre_verdict"
                    if eligible
                    else f"{review['outcome']}_or_low_confidence"
                ),
            },
            "supersedes_record_id": verdict.get("supersedes_record_id"),
        }
        record["record_id"] = fingerprint(
            {
                "schema_version": SCHEMA_VERSION,
                "batch_id": record["batch_id"],
                "code": record["code"],
                "track_id": record["track"]["track_id"],
                "decoded_pcm_sha256": audio_identity["decoded_pcm_sha256"],
                "review": review,
                "provenance": record["provenance"],
                "supersedes_record_id": record["supersedes_record_id"],
            }
        )
        proposed.append(record)

    existing = load_ledger(ledger_path)
    by_batch_code = {
        (str(record["batch_id"]), str(record["code"])): record
        for record in active_records(existing)
    }
    existing_ids = {str(record["record_id"]) for record in existing}
    active_by_audio = {
        str(record["audio_identity"]["decoded_pcm_sha256"]): record
        for record in active_records(existing)
    }
    additions = []
    for record in proposed:
        if record["record_id"] in existing_ids:
            continue
        supersedes = record.get("supersedes_record_id")
        if supersedes is not None and supersedes not in existing_ids:
            raise ValueError(
                f"superseded record does not exist for {record['code']}: {supersedes}"
            )
        key = (str(record["batch_id"]), str(record["code"]))
        previous = by_batch_code.get(key)
        if previous is not None and supersedes != previous["record_id"]:
            raise ValueError(
                f"changed verdict for {record['code']} must explicitly supersede "
                f"record {previous['record_id']}"
            )
        decoded_sha = str(record["audio_identity"]["decoded_pcm_sha256"])
        prior_audio = active_by_audio.get(decoded_sha)
        if prior_audio is not None and supersedes != prior_audio["record_id"]:
            raise ValueError(
                f"decoded audio for {record['code']} already has active record "
                f"{prior_audio['record_id']}; explicit supersession is required"
            )
        additions.append(record)
        active_by_audio[decoded_sha] = record

    all_records = [*existing, *additions]
    ledger_payload = b"".join(
        canonical_json_bytes(record) + b"\n" for record in all_records
    )
    atomic_write(ledger_path, ledger_payload)
    ledger_sha256 = hashlib.sha256(ledger_payload).hexdigest()
    snapshot = build_snapshot(all_records, ledger_sha256)
    atomic_write(snapshot_path, json.dumps(snapshot, indent=2, sort_keys=True).encode() + b"\n")
    return {
        "ledger": str(ledger_path),
        "snapshot": str(snapshot_path),
        "records_added": len(additions),
        "active_review_records": snapshot["active_review_records"],
        "model_eligible_rows": snapshot["model_eligible_rows"],
        "outcome_counts": snapshot["outcome_counts"],
        "genre_counts": snapshot["genre_counts"],
        "ledger_sha256": ledger_sha256,
        "corpus_fingerprint": snapshot["corpus_fingerprint"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mapping", required=True, type=Path)
    parser.add_argument("--verdicts", required=True, type=Path)
    parser.add_argument("--ledger", required=True, type=Path)
    parser.add_argument("--snapshot", required=True, type=Path)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    args = parser.parse_args()
    result = ingest(
        json.loads(args.mapping.read_text(encoding="utf-8")),
        json.loads(args.verdicts.read_text(encoding="utf-8")),
        args.ledger,
        args.snapshot,
        args.ffmpeg,
    )
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
