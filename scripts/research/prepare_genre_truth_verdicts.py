#!/usr/bin/env python3
"""Convert a completed blind-review TSV into a validated private verdict file."""

from __future__ import annotations

import argparse
import csv
import io
import json
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable

import build_genre_intelligence_corpus as corpus
import ingest_genre_truth_batch as ingest


EXPECTED_HEADERS = [
    "position",
    "code",
    "artist",
    "title",
    "verdict",
    "confidence",
    "alternatives",
    "notes",
]
AMBIGUOUS_WORDS = {"ambiguous", "unsure", "uncertain"}
SKIP_WORDS = {"skip", "exclude", "excluded"}
BUILTIN_ALIASES = {
    "breaks": "Breakbeat",
    "d b": "Drum & Bass",
    "dnb": "Drum & Bass",
    "drum and bass": "Drum & Bass",
    "hiphop": "Hip Hop",
    "minimal techno": "Minimal",
    "r b": "R&B",
    "rnb": "R&B",
    "rhythm and blues": "R&B",
    "techhouse": "Tech House",
}
CONFIDENCE_ALIASES = {
    "high": "high",
    "medium": "medium",
    "low": "low",
    "medium high": "medium",
    "medium to high": "medium",
    "high medium": "medium",
    "high to medium": "medium",
}


def genre_lookup(extra_aliases: dict[str, str] | None = None) -> dict[str, str]:
    lookup: dict[str, str] = {}
    for fine_genre, parent in corpus.FINE_TO_PARENT.items():
        if parent is not None:
            lookup[corpus.normalized(fine_genre)] = parent
    for parent in corpus.PARENT_GENRES:
        lookup[corpus.normalized(parent)] = parent
    lookup.update(BUILTIN_ALIASES)
    for raw, parent in (extra_aliases or {}).items():
        if parent not in corpus.PARENT_GENRES:
            raise ValueError(f"alias {raw!r} has unsupported parent {parent!r}")
        key = corpus.normalized(raw)
        if not key:
            raise ValueError("genre alias must not be blank")
        lookup[key] = parent
    return lookup


def resolve_genre(raw: str, lookup: dict[str, str]) -> str:
    key = corpus.normalized(raw)
    parent = lookup.get(key)
    if parent is None:
        raise ValueError(f"genre wording requires an explicit alias: {raw!r}")
    return parent


def split_alternatives(raw: str) -> list[str]:
    values = [raw]
    for delimiter in ("/", ",", ";", "|"):
        values = [part for value in values for part in value.split(delimiter)]
    return [value.strip() for value in values if value.strip()]


def normalize_confidence(raw: str, *, required: bool) -> str | None:
    if not raw.strip():
        if required:
            raise ValueError("label verdict requires confidence")
        return None
    key = corpus.normalized(raw)
    confidence = CONFIDENCE_ALIASES.get(key)
    if confidence is None:
        raise ValueError(f"confidence wording is unsupported: {raw!r}")
    return confidence


def validate_timestamp(value: str) -> str:
    candidate = value.strip()
    try:
        parsed = datetime.fromisoformat(candidate.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("reviewed_at must be an ISO-8601 timestamp") from error
    if parsed.tzinfo is None:
        raise ValueError("reviewed_at must include a timezone")
    return candidate


def parse_assignment(value: str, label: str) -> tuple[str, str]:
    raw, separator, normalized_value = value.partition("=")
    if not separator or not raw.strip() or not normalized_value.strip():
        raise ValueError(f"{label} must use RAW=VALUE")
    return raw.strip(), normalized_value.strip()


def assignments(values: Iterable[str], label: str) -> dict[str, str]:
    result = {}
    for value in values:
        raw, normalized_value = parse_assignment(value, label)
        if raw in result:
            raise ValueError(f"duplicate {label} for {raw!r}")
        result[raw] = normalized_value
    return result


def read_review_tsv(value: str) -> list[dict[str, str]]:
    reader = csv.DictReader(io.StringIO(value), delimiter="\t")
    if reader.fieldnames != EXPECTED_HEADERS:
        raise ValueError(
            f"review TSV headers differ: expected {EXPECTED_HEADERS!r}, "
            f"got {reader.fieldnames!r}"
        )
    rows = []
    for line_number, row in enumerate(reader, start=2):
        if None in row:
            raise ValueError(f"review TSV line {line_number} has extra columns")
        if any(row[field] is None for field in EXPECTED_HEADERS):
            raise ValueError(f"review TSV line {line_number} has missing columns")
        rows.append({field: str(row[field]) for field in EXPECTED_HEADERS})
    return rows


def prepare_verdicts(
    mapping: dict[str, Any],
    review_rows: list[dict[str, str]],
    *,
    reviewer: str,
    reviewed_at: str,
    extra_aliases: dict[str, str] | None = None,
    supersessions: dict[str, str] | None = None,
    alternative_notes: set[str] | None = None,
) -> dict[str, Any]:
    selected = mapping.get("selected")
    if not isinstance(selected, list) or not 1 <= len(selected) <= 20:
        raise ValueError("mapping must contain one to twenty selected rows")
    if not reviewer.strip():
        raise ValueError("reviewer must not be blank")
    reviewed_at = validate_timestamp(reviewed_at)
    selected_by_code = {str(row["code"]): row for row in selected}
    review_by_code = {row["code"]: row for row in review_rows}
    if len(selected_by_code) != len(selected) or len(review_by_code) != len(review_rows):
        raise ValueError("mapping and review codes must be unique")
    if set(selected_by_code) != set(review_by_code):
        raise ValueError("review TSV must cover every selected code exactly once")
    lookup = genre_lookup(extra_aliases)
    supersessions = supersessions or {}
    alternative_notes = alternative_notes or set()
    unknown_supersessions = set(supersessions) - set(selected_by_code)
    if unknown_supersessions:
        raise ValueError(
            f"supersession codes are absent from the mapping: {sorted(unknown_supersessions)}"
        )
    unknown_alternative_notes = alternative_notes - set(selected_by_code)
    if unknown_alternative_notes:
        raise ValueError(
            "alternative-as-note codes are absent from the mapping: "
            f"{sorted(unknown_alternative_notes)}"
        )

    verdict_rows = []
    parent_order = {parent: index for index, parent in enumerate(corpus.PARENT_GENRES)}
    for selected_row in sorted(selected, key=lambda row: int(row["position"])):
        code = str(selected_row["code"])
        row = review_by_code[code]
        if (
            row["position"] != str(selected_row["position"])
            or row["artist"] != str(selected_row["artist"])
            or row["title"] != str(selected_row["title"])
        ):
            raise ValueError(f"review identity differs from mapping for {code}")
        genre_raw = row["verdict"].strip()
        if not genre_raw:
            raise ValueError(f"review verdict is blank for {code}")
        normalized_verdict = corpus.normalized(genre_raw)
        alternatives_raw = row["alternatives"]
        notes = row["notes"]
        alternative_values = split_alternatives(alternatives_raw)
        if code in alternative_notes:
            if not alternatives_raw.strip() or notes.strip():
                raise ValueError(
                    f"alternative-as-note requires alternatives and blank notes for {code}"
                )
            notes = alternatives_raw
            alternative_values = []
        alternatives = []
        for raw_alternative in alternative_values:
            parent = resolve_genre(raw_alternative, lookup)
            if parent not in alternatives:
                alternatives.append(parent)

        if normalized_verdict in AMBIGUOUS_WORDS:
            outcome = "ambiguous"
            genre = None
            if not alternatives:
                raise ValueError(f"ambiguous verdict requires alternatives for {code}")
            confidence = normalize_confidence(row["confidence"], required=False)
        elif normalized_verdict in SKIP_WORDS:
            outcome = "skip"
            genre = None
            confidence = normalize_confidence(row["confidence"], required=False)
        else:
            outcome = "label"
            genre = resolve_genre(genre_raw, lookup)
            alternatives = [value for value in alternatives if value != genre]
            confidence = normalize_confidence(row["confidence"], required=True)
        alternatives.sort(key=lambda parent: parent_order[parent])

        verdict = {
            "code": code,
            "outcome": outcome,
            "genre": genre,
            "genre_raw": genre_raw,
            "confidence": confidence,
            "confidence_raw": row["confidence"],
            "alternatives": alternatives,
            "alternatives_raw": alternatives_raw,
            "notes": notes,
        }
        if code in supersessions:
            verdict["supersedes_record_id"] = supersessions[code]
        ingest.validate_verdict(verdict)
        verdict_rows.append(verdict)

    return {
        "batch_id": str(mapping["experiment_id"]),
        "reviewer": reviewer.strip(),
        "reviewed_at": reviewed_at,
        "normalization": {
            "taxonomy_version": corpus.TAXONOMY_VERSION,
            "taxonomy_semantic_sha256": corpus.taxonomy_semantic_sha256(),
            "extra_aliases": dict(sorted((extra_aliases or {}).items())),
            "alternative_cells_copied_to_notes": sorted(alternative_notes),
            "confidence_policy": "casefolded; mixed medium-high normalized conservatively to medium",
        },
        "rows": verdict_rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mapping", required=True, type=Path)
    parser.add_argument("--review", required=True, type=Path)
    parser.add_argument("--reviewer", required=True)
    parser.add_argument("--reviewed-at", required=True)
    parser.add_argument("--alias", action="append", default=[])
    parser.add_argument("--supersedes", action="append", default=[])
    parser.add_argument("--alternative-as-note", action="append", default=[])
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    extra_aliases = assignments(args.alias, "alias")
    supersessions = assignments(args.supersedes, "supersedes")
    mapping = json.loads(args.mapping.read_text(encoding="utf-8"))
    review_text = args.review.read_text(encoding="utf-8")
    result = prepare_verdicts(
        mapping,
        read_review_tsv(review_text),
        reviewer=args.reviewer,
        reviewed_at=args.reviewed_at,
        extra_aliases=extra_aliases,
        supersessions=supersessions,
        alternative_notes=set(args.alternative_as_note),
    )
    result["source_review_sha256"] = corpus.sha256_file(args.review)
    corpus.atomic_write(
        args.output,
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8")
        + b"\n",
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "batch_id": result["batch_id"],
                "rows": len(result["rows"]),
                "outcomes": dict(
                    sorted(Counter(row["outcome"] for row in result["rows"]).items())
                ),
                "source_review_sha256": result["source_review_sha256"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
