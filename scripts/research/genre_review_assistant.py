#!/usr/bin/env python3
"""Build a private, read-only human genre-review packet from frozen artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import urllib.parse
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_genre_eval as base


EXPERIMENT_ID = "genre-review-assistant-v1"
SEED = "genre-review-assistant-v1-frozen"
BATCH_SIZE = 6
MIN_REFERENCE_SUPPORT = 3
MAX_GENRES_PER_FAMILY = 2
NEIGHBOURS_PER_AFFINITY = 3
REFERENCES_PER_HINT = 2
ALTERNATIVE_HINTS = 2

ARRANGEMENT_FEATURES = (
    "loudness_range",
    "dynamic_complexity",
    "spectral_flux_mean",
    "onset_rate",
)

REFERENCE_GENRE_ALIASES = {
    "Dub Reggae": "Dub",
    "Reggae Dub": "Dub",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_hash(*parts: object) -> str:
    value = "|".join(str(part) for part in (SEED, *parts))
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def normalized(value: str) -> str:
    return " ".join(re.findall(r"[a-z0-9]+", value.casefold()))


def artist_keys(value: str) -> set[str]:
    parts = re.split(
        r"\s*(?:,|;|&|\+|/|\bfeat\.?\b|\bfeaturing\b|\bvs\.?\b|\bwith\b)\s*",
        value,
        flags=re.IGNORECASE,
    )
    keys = {normalized(part) for part in parts if normalized(part)}
    return keys or {normalized(value)}


def release_group(row: dict[str, Any]) -> str:
    artist = normalized(str(row.get("artist", "")))
    release = normalized(str(row.get("album", ""))) or normalized(
        str(row.get("title", ""))
    )
    return f"{artist}\0{release}"


def parse_location(value: str) -> str:
    parsed = urllib.parse.urlparse(value)
    if parsed.scheme != "file" or parsed.netloc not in {"", "localhost"}:
        raise ValueError(f"unsupported reference XML location: {value}")
    return urllib.parse.unquote(parsed.path)


def parse_reference_xml(path: Path) -> dict[str, dict[str, str]]:
    root = ET.parse(path).getroot()
    references: dict[str, dict[str, str]] = {}
    for element in root.findall("./COLLECTION/TRACK"):
        file_path = parse_location(element.attrib.get("Location", ""))
        if not file_path:
            raise ValueError("reference XML track has no file location")
        if file_path in references:
            raise ValueError(f"duplicate reference XML path: {file_path}")
        references[file_path] = {
            "artist": element.attrib.get("Artist", ""),
            "title": element.attrib.get("Name", ""),
            "album": element.attrib.get("Album", ""),
            "genre": REFERENCE_GENRE_ALIASES.get(
                element.attrib.get("Genre", ""), element.attrib.get("Genre", "")
            ),
            "file_path": file_path,
        }
    if not references:
        raise ValueError("reference XML contains no collection tracks")
    return references


def normalized_embeddings(values: np.ndarray, label: str) -> np.ndarray:
    array = np.asarray(values, dtype=np.float64)
    if array.ndim != 2 or not np.isfinite(array).all():
        raise ValueError(f"{label} embeddings must be a finite two-dimensional array")
    norms = np.linalg.norm(array, axis=1, keepdims=True)
    if np.any(norms <= 1e-12):
        raise ValueError(f"{label} embeddings contain a zero-length row")
    return array / norms


def validate_row_indices(rows: list[dict[str, Any]], label: str) -> None:
    for position, row in enumerate(rows):
        if row.get("row_index") != position:
            raise ValueError(f"{label} row_index alignment differs at row {position}")


def load_inputs(args: argparse.Namespace) -> dict[str, Any]:
    candidate_manifest = json.loads(args.candidate_manifest.read_text(encoding="utf-8"))
    development_manifest = json.loads(
        args.development_manifest.read_text(encoding="utf-8")
    )
    development_result = json.loads(
        args.development_result.read_text(encoding="utf-8")
    )
    provenance = json.loads(args.provenance_result.read_text(encoding="utf-8"))

    if candidate_manifest.get("experiment_id") != "genre-audit-consensus-v1":
        raise ValueError("unexpected candidate manifest experiment ID")
    if (
        development_manifest.get("experiment_id")
        != "genre-profile-grouped-cv-v2-expanded-corpus"
    ):
        raise ValueError("unexpected development manifest experiment ID")
    if (
        development_result.get("experiment_id")
        != "discogs-effnet-genre-evaluation-v2-expanded-corpus"
    ):
        raise ValueError("unexpected development result experiment ID")
    if provenance.get("experiment_id") != "genre-audit-consensus-v1":
        raise ValueError("unexpected provenance result experiment ID")

    expected = provenance.get("inputs", {})
    checks = (
        (args.candidate_manifest, "audit_manifest_sha256"),
        (args.candidate_features, "candidate_feature_sha256"),
        (args.development_manifest, "development_manifest_sha256"),
        (args.development_features, "development_feature_sha256"),
    )
    for path, key in checks:
        actual = sha256_file(path)
        if actual != expected.get(key):
            raise ValueError(f"{key} differs from the frozen provenance result")

    development_feature_record = development_result.get("feature_artifact", {})
    if sha256_file(args.development_features) != development_feature_record.get(
        "sha256"
    ):
        raise ValueError("development feature artifact differs from its result")

    development_fingerprint = development_manifest.get("corpus_fingerprint")
    if not (
        candidate_manifest.get("development_corpus_fingerprint")
        == development_fingerprint
        == development_result.get("corpus_fingerprint")
    ):
        raise ValueError("development corpus fingerprints differ")

    candidate_rows = candidate_manifest.get("rows", [])
    development_rows = development_manifest.get("rows", [])
    validate_row_indices(candidate_rows, "candidate")
    validate_row_indices(development_rows, "development")

    candidate_artifact = np.load(args.candidate_features, allow_pickle=False)
    development_artifact = np.load(args.development_features, allow_pickle=False)
    candidate_embeddings = normalized_embeddings(
        candidate_artifact["embeddings"], "candidate"
    )
    development_embeddings = normalized_embeddings(
        development_artifact["embeddings"], "development"
    )
    candidate_arrangement = np.asarray(
        candidate_artifact["arrangement"], dtype=np.float64
    )
    development_arrangement = np.asarray(
        development_artifact["arrangement"], dtype=np.float64
    )
    if len(candidate_rows) != len(candidate_embeddings):
        raise ValueError("candidate manifest and feature row counts differ")
    if len(development_rows) != len(development_embeddings):
        raise ValueError("development manifest and feature row counts differ")
    if candidate_arrangement.shape != (len(candidate_rows), len(ARRANGEMENT_FEATURES)):
        raise ValueError("candidate arrangement feature shape differs")
    if development_arrangement.shape != (
        len(development_rows),
        len(ARRANGEMENT_FEATURES),
    ):
        raise ValueError("development arrangement feature shape differs")

    xml_references = parse_reference_xml(args.reference_xml)
    reference_rows = []
    for row in development_rows:
        file_path = row["file_path"]
        identity = xml_references.get(file_path)
        if identity is None:
            raise ValueError(f"development row missing from reference XML: {file_path}")
        if identity["genre"] != row["truth"]:
            raise ValueError(f"reference XML genre differs for: {file_path}")
        reference_rows.append({**row, **identity})

    excluded_ids = {
        str(row["track_id"])
        for row in provenance.get("selected", [])
        if row.get("track_id") is not None
    }
    return {
        "candidate_manifest": candidate_manifest,
        "development_manifest": development_manifest,
        "development_result": development_result,
        "provenance": provenance,
        "candidate_rows": candidate_rows,
        "candidate_embeddings": candidate_embeddings,
        "candidate_arrangement": candidate_arrangement,
        "development_rows": reference_rows,
        "development_embeddings": development_embeddings,
        "development_arrangement": development_arrangement,
        "excluded_ids": excluded_ids,
    }


def eligible_reference_indices(
    candidate: dict[str, Any], references: list[dict[str, Any]]
) -> list[int]:
    candidate_artists = artist_keys(str(candidate.get("artist", "")))
    candidate_group = release_group(candidate)
    return [
        index
        for index, reference in enumerate(references)
        if candidate_artists.isdisjoint(artist_keys(reference["artist"]))
        and release_group(reference) != candidate_group
    ]


def distinct_reference_indices(
    similarities: np.ndarray,
    indices: list[int],
    references: list[dict[str, Any]],
    limit: int,
) -> list[int]:
    ranked = sorted(
        indices,
        key=lambda index: (
            -float(similarities[index]),
            normalized(references[index]["artist"]),
            normalized(references[index]["title"]),
            references[index]["file_path"],
        ),
    )
    selected: list[int] = []
    groups = set()
    artists: set[str] = set()
    for index in ranked:
        reference = references[index]
        group = release_group(reference)
        reference_artists = artist_keys(reference["artist"])
        if group in groups or not artists.isdisjoint(reference_artists):
            continue
        selected.append(index)
        groups.add(group)
        artists.update(reference_artists)
        if len(selected) == limit:
            break
    return selected


def reference_details(
    similarities: np.ndarray,
    indices: list[int],
    references: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    return [
        {
            "artist": references[index]["artist"],
            "title": references[index]["title"],
            "album": references[index]["album"],
            "genre": references[index]["truth"],
            "file_path": references[index]["file_path"],
            "similarity": round(float(similarities[index]), 6),
        }
        for index in indices
    ]


def genre_affinities(
    candidate: dict[str, Any],
    candidate_embedding: np.ndarray,
    references: list[dict[str, Any]],
    reference_embeddings: np.ndarray,
) -> dict[str, dict[str, Any]]:
    similarities = reference_embeddings @ candidate_embedding
    eligible = eligible_reference_indices(candidate, references)
    by_genre: dict[str, list[int]] = {}
    for index in eligible:
        by_genre.setdefault(references[index]["truth"], []).append(index)

    affinities = {}
    for genre, indices in by_genre.items():
        affinity_indices = distinct_reference_indices(
            similarities,
            indices,
            references,
            NEIGHBOURS_PER_AFFINITY,
        )
        if len(affinity_indices) < NEIGHBOURS_PER_AFFINITY:
            continue
        affinity = float(
            np.mean([float(similarities[index]) for index in affinity_indices])
        )
        refs = reference_details(
            similarities,
            affinity_indices[:REFERENCES_PER_HINT],
            references,
        )
        affinities[genre] = {
            "genre": genre,
            "reference_support": len(indices),
            "affinity": round(affinity, 6),
            "references": refs,
        }
    return affinities


def build_hints(
    current_genre: str, affinities: dict[str, dict[str, Any]]
) -> list[dict[str, Any]]:
    current = affinities.get(current_genre)
    if current is None:
        raise ValueError(f"current genre lacks eligible references: {current_genre}")
    alternatives = sorted(
        (value for genre, value in affinities.items() if genre != current_genre),
        key=lambda value: (
            -float(value["affinity"]),
            base.CANONICAL_INDEX[value["genre"]],
        ),
    )[:ALTERNATIVE_HINTS]
    return [
        {**current, "role": "current_genre_context"},
        *({**value, "role": "alternative_listening_hint"} for value in alternatives),
    ]


def percentile(value: float, population: np.ndarray) -> float | None:
    finite = population[np.isfinite(population)]
    if not math.isfinite(value) or not len(finite):
        return None
    return float(np.count_nonzero(finite <= value) / len(finite))


def vocabulary_cues(
    bpm: float, values: np.ndarray, development_values: np.ndarray
) -> list[dict[str, Any]]:
    labels = {
        "loudness_range": (
            "even loudness contour",
            "moderate loudness contrast",
            "wide loudness contrast",
        ),
        "dynamic_complexity": (
            "consistent dynamics",
            "moderate dynamic variation",
            "pronounced dynamic variation",
        ),
        "spectral_flux_mean": (
            "stable spectral texture",
            "moderate spectral motion",
            "frequent spectral change",
        ),
        "onset_rate": (
            "sparse event density",
            "moderate event density",
            "busy event density",
        ),
    }
    cues = [
        {
            "feature": "tempo",
            "description": f"measured tempo near {bpm:.2f} BPM",
            "value": round(float(bpm), 6),
        }
    ]
    for index, feature in enumerate(ARRANGEMENT_FEATURES):
        value = float(values[index])
        rank = percentile(value, development_values[:, index])
        if rank is None:
            continue
        bucket = 0 if rank <= 0.25 else 2 if rank >= 0.75 else 1
        cues.append(
            {
                "feature": feature,
                "description": labels[feature][bucket],
                "percentile": round(rank, 6),
                "value": round(value, 6),
            }
        )
    return cues


def candidate_records(inputs: dict[str, Any]) -> list[dict[str, Any]]:
    references = inputs["development_rows"]
    reference_embeddings = inputs["development_embeddings"]
    verified_support = Counter(reference["truth"] for reference in references)
    records = []
    for position, candidate in enumerate(inputs["candidate_rows"]):
        if str(candidate["track_id"]) in inputs["excluded_ids"]:
            continue
        current = candidate["current_genre"]
        if current == "Experimental" or current not in base.CANONICAL_INDEX:
            continue
        if verified_support[current] < MIN_REFERENCE_SUPPORT:
            continue
        affinities = genre_affinities(
            candidate,
            inputs["candidate_embeddings"][position],
            references,
            reference_embeddings,
        )
        current_affinity = affinities.get(current)
        if current_affinity is None:
            continue
        records.append(
            {
                **candidate,
                "release_group": release_group(candidate),
                "verified_genre_support": verified_support[current],
                "current_reference_support": current_affinity["reference_support"],
                "current_affinity": current_affinity["affinity"],
                "hints": build_hints(current, affinities),
                "listening_cues": vocabulary_cues(
                    float(candidate["bpm"]),
                    inputs["candidate_arrangement"][position],
                    inputs["development_arrangement"],
                ),
                "stable_hash": stable_hash(candidate["track_id"], candidate["file_path"]),
            }
        )
    return records


def select_batch(
    records: list[dict[str, Any]], batch_size: int = BATCH_SIZE
) -> list[dict[str, Any]]:
    by_genre: dict[str, list[dict[str, Any]]] = {}
    for record in records:
        by_genre.setdefault(record["current_genre"], []).append(record)
    genres = sorted(
        by_genre,
        key=lambda genre: (
            int(by_genre[genre][0]["verified_genre_support"]),
            base.CANONICAL_INDEX[genre],
        ),
    )

    selected = []
    groups = set()
    family_counts: Counter[str] = Counter()
    for genre in genres:
        family = base.family(genre)
        if family_counts[family] >= MAX_GENRES_PER_FAMILY:
            continue
        ranked = sorted(
            by_genre[genre],
            key=lambda record: (
                -float(record["current_affinity"]),
                record["stable_hash"],
            ),
        )
        chosen = next(
            (record for record in ranked if record["release_group"] not in groups),
            None,
        )
        if chosen is None:
            continue
        selected.append(chosen)
        groups.add(chosen["release_group"])
        family_counts[family] += 1
        if len(selected) == batch_size:
            break
    if len(selected) != batch_size:
        raise ValueError(
            f"frozen review rule produced {len(selected)} rows; required {batch_size}"
        )
    return sorted(
        selected,
        key=lambda record: stable_hash("shuffle", record["track_id"]),
    )


def private_selected_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GR{position:02d}",
        "track_id": row["track_id"],
        "file_path": row["file_path"],
        "artist": row["artist"],
        "title": row["title"],
        "album": row["album"],
        "bpm": row["bpm"],
        "current_genre": row["current_genre"],
        "release_group": row["release_group"],
        "verified_genre_support": row["verified_genre_support"],
        "current_reference_support": row["current_reference_support"],
        "current_affinity": row["current_affinity"],
        "hints": row["hints"],
        "listening_cues": row["listening_cues"],
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    inputs = load_inputs(args)
    records = candidate_records(inputs)
    selected = [
        private_selected_row(row, position)
        for position, row in enumerate(select_batch(records), 1)
    ]
    roster_sha = hashlib.sha256(
        json.dumps(selected, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "private_read_only_human_review_assistant",
        "candidate_corpus_fingerprint": inputs["candidate_manifest"][
            "candidate_corpus_fingerprint"
        ],
        "development_corpus_fingerprint": inputs["development_manifest"][
            "corpus_fingerprint"
        ],
        "inputs": {
            "candidate_manifest_sha256": sha256_file(args.candidate_manifest),
            "candidate_feature_sha256": sha256_file(args.candidate_features),
            "development_manifest_sha256": sha256_file(args.development_manifest),
            "development_feature_sha256": sha256_file(args.development_features),
            "development_result_sha256": sha256_file(args.development_result),
            "provenance_result_sha256": sha256_file(args.provenance_result),
            "reference_xml_sha256": sha256_file(args.reference_xml),
        },
        "selection_rule": {
            "batch_size": BATCH_SIZE,
            "minimum_reference_support": MIN_REFERENCE_SUPPORT,
            "neighbours_per_affinity": NEIGHBOURS_PER_AFFINITY,
            "references_per_hint": REFERENCES_PER_HINT,
            "alternative_hints": ALTERNATIVE_HINTS,
            "maximum_genres_per_family": MAX_GENRES_PER_FAMILY,
            "excluded_genres": ["Experimental"],
            "seed_sha256": hashlib.sha256(SEED.encode("utf-8")).hexdigest(),
        },
        "universe": {
            "candidate_rows": len(inputs["candidate_rows"]),
            "excluded_exposed_rows": len(inputs["excluded_ids"]),
            "eligible_review_rows": len(records),
            "verified_reference_rows": len(inputs["development_rows"]),
        },
        "roster_sha256": roster_sha,
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate-manifest", required=True, type=Path)
    parser.add_argument("--candidate-features", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--development-features", required=True, type=Path)
    parser.add_argument("--development-result", required=True, type=Path)
    parser.add_argument("--provenance-result", required=True, type=Path)
    parser.add_argument("--reference-xml", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "experiment_id": result["experiment_id"],
                "universe": result["universe"],
                "tracks": len(result["selected"]),
                "roster_sha256": result["roster_sha256"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
