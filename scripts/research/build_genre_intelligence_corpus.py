#!/usr/bin/env python3
"""Build a diversity-balanced Genre Intelligence development corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

import select_broad_genre_holdout as library


SCHEMA_VERSION = 1
CORPUS_VERSION = "genre-intelligence-development-v1"
TAXONOMY_VERSION = "broad-parent-consensus-v1"
BALANCE_SEED = "genre-intelligence-development-v1"
EXPECTED_BASE_ROWS = 668
EXPECTED_BASE_CORPUS_FINGERPRINT = (
    "sha256:b88911c7b24bbeecd1d59607ceb5e873ca29ff6f15052e77913635e5832471f1"
)
EXPECTED_TAXONOMY_SEMANTIC_SHA256 = (
    "efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255"
)
MIN_ROWS = 20
MIN_ARTISTS = 15
MIN_RELEASE_GROUPS = 12
MIN_RELEASE_SCOPE_PARENTS = 7
MIN_ACCEPTED_SCOPE_COVERAGE = 0.75
MAX_ARTIST_SHARE = 0.20

CANONICAL = [
    "2-Step Garage",
    "Acid",
    "Afro House",
    "Ambient",
    "Ambient Techno",
    "Bassline",
    "Breakbeat",
    "Broken Beat",
    "Dancehall",
    "Deep House",
    "Deep Techno",
    "Disco",
    "Downtempo",
    "Drum & Bass",
    "Dub",
    "Dub Techno",
    "Dubstep",
    "EBM",
    "Electro",
    "Experimental",
    "Footwork",
    "Future Garage",
    "Gabber",
    "Garage",
    "Gospel House",
    "Grime",
    "Happy Hardcore",
    "Hard Techno",
    "Hard Trance",
    "Hardcore",
    "Hardstyle",
    "Highlife",
    "Hip Hop",
    "House",
    "IDM",
    "Italo Disco",
    "Italodance",
    "Jazz",
    "Jungle",
    "Minimal",
    "Pop",
    "Progressive House",
    "Psytrance",
    "R&B",
    "Reggae",
    "Rock",
    "Speed Garage",
    "Synth-pop",
    "Tech House",
    "Techno",
    "Trance",
    "Trip-Hop",
    "UK Funky",
]

GROUPS = {
    "House": {"Afro House", "Deep House", "Gospel House", "House", "Progressive House"},
    "Techno": {"Ambient Techno", "Deep Techno", "Dub Techno", "Hard Techno", "Techno"},
    "Trance": {"Hard Trance", "Psytrance", "Trance"},
    "Garage": {
        "2-Step Garage",
        "Bassline",
        "Future Garage",
        "Garage",
        "Speed Garage",
        "UK Funky",
    },
    "Breakbeat": {"Breakbeat", "Broken Beat"},
    "Drum & Bass": {"Drum & Bass", "Jungle"},
    "Reggae": {"Dancehall", "Dub", "Reggae"},
    "Disco": {"Disco", "Italo Disco"},
    "Hardcore": {"Gabber", "Happy Hardcore", "Hardcore", "Hardstyle"},
    "Downtempo": {"Downtempo", "Trip-Hop"},
    "Pop": {"Italodance", "Pop", "Synth-pop"},
}
SELF_PARENT = {
    "Acid",
    "Ambient",
    "Dubstep",
    "EBM",
    "Electro",
    "Footwork",
    "Grime",
    "Highlife",
    "Hip Hop",
    "IDM",
    "Jazz",
    "Minimal",
    "R&B",
    "Rock",
    "Tech House",
}


def fine_to_parent() -> dict[str, str | None]:
    result: dict[str, str | None] = {genre: None for genre in CANONICAL}
    for parent, genres in GROUPS.items():
        for genre in genres:
            if result[genre] is not None:
                raise ValueError(f"duplicate parent mapping for {genre}")
            result[genre] = parent
    for genre in SELF_PARENT:
        if result[genre] is not None:
            raise ValueError(f"duplicate parent mapping for {genre}")
        result[genre] = genre
    if result["Experimental"] is not None:
        raise ValueError("Experimental must remain unmodeled")
    missing = [
        genre
        for genre, parent in result.items()
        if parent is None and genre != "Experimental"
    ]
    if missing:
        raise ValueError(f"missing parent mappings: {missing}")
    return result


FINE_TO_PARENT = fine_to_parent()
PARENT_GENRES = tuple(
    dict.fromkeys(
        parent for genre in CANONICAL if (parent := FINE_TO_PARENT[genre]) is not None
    )
)


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


def taxonomy_semantic_sha256() -> str:
    mapping = "\n".join(
        f"{genre}=>{FINE_TO_PARENT[genre] or '<unmodeled>'}" for genre in CANONICAL
    )
    return hashlib.sha256(f"{TAXONOMY_VERSION}\n{mapping}".encode()).hexdigest()


def normalized(value: str) -> str:
    return " ".join(re.findall(r"[a-z0-9]+", value.casefold()))


def release_group(artist: str, album: str, title: str) -> str:
    release = normalized(album) or normalized(title)
    return f"{normalized(artist)}\0{release}"


def stable_row_hash(parent: str, row: dict[str, Any]) -> str:
    return hashlib.sha256(
        f"{BALANCE_SEED}|{parent}|{row['row_id']}".encode("utf-8")
    ).hexdigest()


def target_metrics(rows: list[dict[str, Any]]) -> dict[str, Any]:
    artists = Counter(str(row["artist_group"]) for row in rows)
    releases = {str(row["release_group"]) for row in rows}
    max_artist_rows = max(artists.values(), default=0)
    max_artist_share = max_artist_rows / len(rows) if rows else 0.0
    return {
        "rows": len(rows),
        "artists": len(artists),
        "release_groups": len(releases),
        "max_artist_rows": max_artist_rows,
        "max_artist_share": max_artist_share,
    }


def diversity_balance(parent: str, rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    retained = list(rows)
    if (
        len(retained) < MIN_ROWS
        or len({row["artist_group"] for row in retained}) < MIN_ARTISTS
        or len({row["release_group"] for row in retained}) < MIN_RELEASE_GROUPS
    ):
        return retained

    while len(retained) >= MIN_ROWS:
        artists = Counter(str(row["artist_group"]) for row in retained)
        max_count = max(artists.values())
        if max_count / len(retained) <= MAX_ARTIST_SHARE:
            break
        most_common_artist = min(
            artist for artist, count in artists.items() if count == max_count
        )
        removable = [
            row for row in retained if row["artist_group"] == most_common_artist
        ]
        remove = max(removable, key=lambda row: stable_row_hash(parent, row))
        retained.remove(remove)
    return sorted(retained, key=lambda row: stable_row_hash(parent, row))


def gate_passes(metrics: dict[str, Any]) -> bool:
    return bool(
        metrics["rows"] >= MIN_ROWS
        and metrics["artists"] >= MIN_ARTISTS
        and metrics["release_groups"] >= MIN_RELEASE_GROUPS
        and metrics["max_artist_share"] <= MAX_ARTIST_SHARE
    )


def build_corpus(
    base_manifest: dict[str, Any],
    truth_snapshot: dict[str, Any],
    library_rows: list[dict[str, Any]],
    *,
    base_manifest_sha256: str,
    truth_snapshot_sha256: str,
    enforce_frozen_base: bool = True,
) -> dict[str, Any]:
    if taxonomy_semantic_sha256() != EXPECTED_TAXONOMY_SEMANTIC_SHA256:
        raise ValueError("parent taxonomy semantics changed")
    base_rows = base_manifest.get("rows")
    if not isinstance(base_rows, list):
        raise ValueError("base manifest has no row list")
    if enforce_frozen_base and (
        len(base_rows) != EXPECTED_BASE_ROWS
        or base_manifest.get("corpus_fingerprint")
        != EXPECTED_BASE_CORPUS_FINGERPRINT
    ):
        raise ValueError("base development corpus differs from the frozen input")
    truth_rows = truth_snapshot.get("rows")
    if not isinstance(truth_rows, list):
        raise ValueError("truth snapshot has no row list")

    live_by_path: dict[str, dict[str, Any]] = {}
    for row in library_rows:
        path = str(row["file_path"])
        if path in live_by_path:
            raise ValueError(f"duplicate live library path: {path}")
        live_by_path[path] = row

    accepted: list[dict[str, Any]] = []
    seen_paths: set[str] = set()
    base_identity_projection = []
    for row in base_rows:
        path = str(row["file_path"])
        if path in seen_paths:
            raise ValueError(f"duplicate accepted path: {path}")
        seen_paths.add(path)
        live = live_by_path.get(path)
        if live is None:
            raise ValueError(f"base row is absent from the live library: {path}")
        fine_genre = str(row["truth"])
        if fine_genre not in FINE_TO_PARENT or FINE_TO_PARENT[fine_genre] is None:
            raise ValueError(f"base row has unsupported truth: {fine_genre!r}")
        artist = str(live["artist"])
        title = str(live["title"])
        album = str(live["album"])
        artist_group = normalized(artist)
        if not artist_group:
            raise ValueError(f"base row has a blank normalized artist: {path}")
        identity = {
            "track_id": str(live["track_id"]),
            "file_path": path,
            "artist": artist,
            "title": title,
            "album": album,
            "artist_group": artist_group,
            "release_group": release_group(artist, album, title),
        }
        base_identity_projection.append(identity)
        accepted.append(
            {
                "row_id": fingerprint(
                    {"source": "plan066_development", "file_path": path}
                ),
                **identity,
                "canonical_parent_genre": FINE_TO_PARENT[fine_genre],
                "source_truth": fine_genre,
                "confidence": "legacy_explicit_verified",
                "provenance": {
                    "kind": "plan066_development_truth",
                    "source_corpus_fingerprint": base_manifest.get(
                        "corpus_fingerprint"
                    ),
                },
            }
        )

    for row in truth_rows:
        path = str(row["file_path"])
        if path in seen_paths:
            raise ValueError(f"duplicate accepted path: {path}")
        seen_paths.add(path)
        parent = str(row["canonical_parent_genre"])
        if parent not in PARENT_GENRES:
            raise ValueError(f"blind row has unsupported parent truth: {parent!r}")
        confidence = str(row["confidence"])
        if confidence not in {"high", "medium"}:
            raise ValueError("truth snapshot contains a non-eligible confidence")
        accepted.append(
            {
                "row_id": str(row["record_id"]),
                "track_id": str(row["track_id"]),
                "file_path": path,
                "artist": str(row["artist"]),
                "title": str(row["title"]),
                "album": str(row.get("album") or ""),
                "artist_group": str(row["artist_group"]),
                "release_group": str(row["release_group"]),
                "decoded_pcm_sha256": str(row["decoded_pcm_sha256"]),
                "canonical_parent_genre": parent,
                "source_truth": parent,
                "confidence": confidence,
                "provenance": row["provenance"],
            }
        )

    by_parent: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in accepted:
        by_parent[str(row["canonical_parent_genre"])].append(row)

    support = {}
    model_ready_rows = []
    for parent in PARENT_GENRES:
        rows = by_parent.get(parent, [])
        balanced = diversity_balance(parent, rows)
        accepted_metrics = target_metrics(rows)
        balanced_metrics = target_metrics(balanced)
        supported = gate_passes(balanced_metrics)
        support[parent] = {
            "accepted": accepted_metrics,
            "balanced": balanced_metrics,
            "deficits": {
                "rows": max(0, MIN_ROWS - accepted_metrics["rows"]),
                "artists": max(0, MIN_ARTISTS - accepted_metrics["artists"]),
                "release_groups": max(
                    0, MIN_RELEASE_GROUPS - accepted_metrics["release_groups"]
                ),
            },
            "supported": supported,
        }
        if supported:
            model_ready_rows.extend(balanced)

    accepted.sort(
        key=lambda row: (
            str(row["canonical_parent_genre"]),
            str(row["artist_group"]),
            str(row["release_group"]),
            str(row["row_id"]),
        )
    )
    model_ready_rows.sort(
        key=lambda row: (
            str(row["canonical_parent_genre"]),
            str(row["artist_group"]),
            str(row["release_group"]),
            str(row["row_id"]),
        )
    )
    release_scope = [parent for parent in PARENT_GENRES if support[parent]["supported"]]
    accepted_scope_coverage = (
        len(model_ready_rows) / len(accepted) if accepted else 0.0
    )
    base_identity_projection.sort(key=lambda row: row["file_path"])
    return {
        "schema_version": SCHEMA_VERSION,
        "corpus_version": CORPUS_VERSION,
        "taxonomy_version": TAXONOMY_VERSION,
        "taxonomy_semantic_sha256": taxonomy_semantic_sha256(),
        "inputs": {
            "base_manifest_sha256": base_manifest_sha256,
            "base_corpus_fingerprint": base_manifest.get("corpus_fingerprint"),
            "base_rows": len(base_rows),
            "base_identity_fingerprint": fingerprint(base_identity_projection),
            "truth_snapshot_sha256": truth_snapshot_sha256,
            "truth_corpus_fingerprint": truth_snapshot.get("corpus_fingerprint"),
            "truth_rows": len(truth_rows),
        },
        "support_gate": {
            "minimum_rows": MIN_ROWS,
            "minimum_artists": MIN_ARTISTS,
            "minimum_release_groups": MIN_RELEASE_GROUPS,
            "maximum_artist_share": MAX_ARTIST_SHARE,
            "minimum_release_scope_parents": MIN_RELEASE_SCOPE_PARENTS,
            "minimum_accepted_scope_coverage": MIN_ACCEPTED_SCOPE_COVERAGE,
            "supported_parents": len(release_scope),
            "accepted_scope_coverage": accepted_scope_coverage,
            "passed": (
                len(release_scope) >= MIN_RELEASE_SCOPE_PARENTS
                and accepted_scope_coverage >= MIN_ACCEPTED_SCOPE_COVERAGE
            ),
        },
        "release_scope": release_scope,
        "support": support,
        "accepted_rows": len(accepted),
        "accepted_corpus_fingerprint": fingerprint(accepted),
        "model_ready_rows": len(model_ready_rows),
        "model_ready_corpus_fingerprint": fingerprint(model_ready_rows),
        "rows": accepted,
        "model_rows": model_ready_rows,
    }


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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-manifest", required=True, type=Path)
    parser.add_argument("--truth-snapshot", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    base_manifest = json.loads(args.base_manifest.read_text(encoding="utf-8"))
    truth_snapshot = json.loads(args.truth_snapshot.read_text(encoding="utf-8"))
    library_rows, _ = library.load_library_snapshot(args.sqlcipher, args.database)
    result = build_corpus(
        base_manifest,
        truth_snapshot,
        library_rows,
        base_manifest_sha256=sha256_file(args.base_manifest),
        truth_snapshot_sha256=sha256_file(args.truth_snapshot),
    )
    atomic_write(
        args.output,
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False).encode("utf-8")
        + b"\n",
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "accepted_rows": result["accepted_rows"],
                "model_ready_rows": result["model_ready_rows"],
                "release_scope": result["release_scope"],
                "support_gate": result["support_gate"],
                "accepted_corpus_fingerprint": result[
                    "accepted_corpus_fingerprint"
                ],
                "model_ready_corpus_fingerprint": result[
                    "model_ready_corpus_fingerprint"
                ],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
