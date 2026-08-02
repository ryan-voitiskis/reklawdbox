#!/usr/bin/env python3
"""Seal a private, leakage-isolated broad-genre listening holdout."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


EXPERIMENT_ID = "broad-genre-next-model-holdout-v1"
SEED = "broad-genre-next-model-holdout-v1"
DEFAULT_COUNT = 60
MAX_PER_TARGET = 5
SQLCIPHER_KEY = (
    "402fd482c38817c35ffa8ffb8c7d93143"
    "b749e7d315df7a81732a1ff43608497"
)

FINE_TO_BROAD = {
    "Afro House": "House",
    "Deep House": "House",
    "Gospel House": "House",
    "House": "House",
    "Progressive House": "House",
    "Ambient Techno": "Techno",
    "Deep Techno": "Techno",
    "Dub Techno": "Techno",
    "Hard Techno": "Techno",
    "Techno": "Techno",
    "Hard Trance": "Trance",
    "Psytrance": "Trance",
    "Trance": "Trance",
    "2-Step Garage": "Garage",
    "Bassline": "Garage",
    "Future Garage": "Garage",
    "Garage": "Garage",
    "Speed Garage": "Garage",
    "UK Funky": "Garage",
    "Breakbeat": "Breakbeat",
    "Broken Beat": "Breakbeat",
    "Drum & Bass": "Drum & Bass",
    "Jungle": "Drum & Bass",
    "Dancehall": "Reggae",
    "Dub": "Reggae",
    "Reggae": "Reggae",
    "Disco": "Disco",
    "Italo Disco": "Disco",
    "Gabber": "Hardcore",
    "Happy Hardcore": "Hardcore",
    "Hardcore": "Hardcore",
    "Hardstyle": "Hardcore",
    "Downtempo": "Downtempo",
    "Trip-Hop": "Downtempo",
    "Italodance": "Pop",
    "Pop": "Pop",
    "Synth-pop": "Pop",
}

for standalone in (
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
):
    FINE_TO_BROAD[standalone] = standalone

CURRENT_GENRE_ALIASES = {
    "ambient techno": "Ambient Techno",
    "dub reggae": "Dub",
    "reggae dub": "Dub",
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


def canonical_current_genre(value: str) -> str | None:
    value = value.strip()
    if value in FINE_TO_BROAD:
        return value
    return CURRENT_GENRE_ALIASES.get(normalized(value))


def broad_target(value: str) -> str | None:
    canonical = canonical_current_genre(value)
    return FINE_TO_BROAD.get(canonical) if canonical is not None else None


def release_group(row: dict[str, Any]) -> str:
    artist = normalized(str(row["artist"]))
    release = normalized(str(row["album"])) or normalized(str(row["title"]))
    return f"{artist}\0{release}"


def parse_json_documents(value: str) -> list[Any]:
    decoder = json.JSONDecoder()
    documents = []
    offset = 0
    while offset < len(value):
        while offset < len(value) and value[offset].isspace():
            offset += 1
        if offset == len(value):
            break
        document, offset = decoder.raw_decode(value, offset)
        documents.append(document)
    return documents


def sqlcipher_query(binary: str, database: Path, query: str) -> list[dict[str, Any]]:
    command = [
        binary,
        "-readonly",
        "-bail",
        "-batch",
        "-json",
        str(database),
        f"PRAGMA key='{SQLCIPHER_KEY}'; {query}",
    ]
    completed = subprocess.run(command, capture_output=True, text=True, check=True)
    documents = parse_json_documents(completed.stdout)
    if not documents or documents[0] != [{"ok": "ok"}]:
        raise ValueError("SQLCipher did not acknowledge the Rekordbox key")
    if len(documents) != 2 or not isinstance(documents[1], list):
        raise ValueError("SQLCipher returned an unexpected JSON document sequence")
    return documents[1]


def load_library_snapshot(
    binary: str, database: Path
) -> tuple[list[dict[str, Any]], dict[str, set[str]]]:
    tracks = sqlcipher_query(
        binary,
        database,
        """
        SELECT
            c.ID AS track_id,
            COALESCE(c.FolderPath, '') AS file_path,
            COALESCE(a.Name, '') AS artist,
            COALESCE(al.Name, '') AS album,
            COALESCE(c.Title, '') AS title,
            COALESCE(g.Name, '') AS current_genre
        FROM djmdContent c
        LEFT JOIN djmdArtist a ON c.ArtistID = a.ID
        LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID
        LEFT JOIN djmdGenre g ON c.GenreID = g.ID
        WHERE c.rb_local_deleted = 0
        ORDER BY c.ID
        """,
    )
    memberships = sqlcipher_query(
        binary,
        database,
        """
        SELECT sp.ContentID AS track_id, COALESCE(p.Name, '') AS playlist
        FROM djmdSongPlaylist sp
        INNER JOIN djmdPlaylist p ON p.ID = sp.PlaylistID
        INNER JOIN djmdContent c ON c.ID = sp.ContentID
        WHERE p.rb_local_deleted = 0 AND c.rb_local_deleted = 0
        ORDER BY sp.ContentID, p.ID
        """,
    )
    playlists: dict[str, set[str]] = defaultdict(set)
    for row in memberships:
        playlists[str(row["track_id"])].add(str(row["playlist"]))
    return tracks, playlists


def private_fingerprint(value: Any) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def development_identities(
    tracks_by_path: dict[str, dict[str, Any]],
    playlists: dict[str, set[str]],
    development_manifests: Iterable[dict[str, Any]],
    exclusion_playlists: set[str],
    exposed_results: Iterable[dict[str, Any]],
) -> tuple[set[str], set[str], set[str]]:
    paths: set[str] = set()
    artists: set[str] = set()
    releases: set[str] = set()

    for manifest in development_manifests:
        for row in manifest["rows"]:
            paths.add(str(row["file_path"]))

    for track in tracks_by_path.values():
        track_playlists = playlists.get(str(track["track_id"]), set())
        if track_playlists & exclusion_playlists:
            paths.add(str(track["file_path"]))

    for result in exposed_results:
        for row in result["selected"]:
            paths.add(str(row["file_path"]))

    for path in paths:
        track = tracks_by_path.get(path)
        if track is None:
            continue
        artist = normalized(str(track["artist"]))
        if artist:
            artists.add(artist)
        releases.add(release_group(track))
    return paths, artists, releases


def eligible_rows(
    audit_rows: list[dict[str, Any]],
    tracks_by_path: dict[str, dict[str, Any]],
    excluded_paths: set[str],
    excluded_artists: set[str],
    excluded_releases: set[str],
) -> tuple[list[dict[str, Any]], Counter[str]]:
    output = []
    exclusions: Counter[str] = Counter()
    for audit_row in audit_rows:
        path = str(audit_row["file_path"])
        live = tracks_by_path.get(path)
        if live is None or str(live["track_id"]) != str(audit_row["track_id"]):
            exclusions["unresolved_live_identity"] += 1
            continue
        artist = normalized(str(live["artist"]))
        release = release_group(live)
        target = broad_target(str(live["current_genre"]))
        if path in excluded_paths:
            exclusions["exposed_path"] += 1
        elif not artist:
            exclusions["blank_artist"] += 1
        elif artist in excluded_artists:
            exclusions["development_artist"] += 1
        elif release in excluded_releases:
            exclusions["development_release"] += 1
        elif not Path(path).is_file():
            exclusions["missing_file"] += 1
        elif target is None:
            exclusions["unmapped_or_experimental_genre"] += 1
        else:
            output.append(
                {
                    **live,
                    "broad_sampling_stratum": target,
                    "artist_group": artist,
                    "release_group": release,
                    "stable_hash": stable_hash(target, live["track_id"], path),
                }
            )
    return output, exclusions


def select_holdout(
    rows: list[dict[str, Any]], count: int = DEFAULT_COUNT
) -> list[dict[str, Any]]:
    by_target: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        by_target[str(row["broad_sampling_stratum"])].append(row)
    for target_rows in by_target.values():
        target_rows.sort(key=lambda row: str(row["stable_hash"]))

    selected = []
    used_artists: set[str] = set()
    used_releases: set[str] = set()
    target_counts: Counter[str] = Counter()
    cursors: Counter[str] = Counter()

    for round_index in range(MAX_PER_TARGET):
        target_order = sorted(
            by_target,
            key=lambda target: stable_hash("target", round_index, target),
        )
        for target in target_order:
            if len(selected) == count:
                return selected
            if target_counts[target] >= MAX_PER_TARGET:
                continue
            candidates = by_target[target]
            while cursors[target] < len(candidates):
                candidate = candidates[cursors[target]]
                cursors[target] += 1
                if (
                    candidate["artist_group"] in used_artists
                    or candidate["release_group"] in used_releases
                ):
                    continue
                selected.append(candidate)
                used_artists.add(str(candidate["artist_group"]))
                used_releases.add(str(candidate["release_group"]))
                target_counts[target] += 1
                break

    raise ValueError(
        f"frozen selector produced {len(selected)} rows; required {count} "
        f"with at most {MAX_PER_TARGET} per target"
    )


def selected_private_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"BGH{position:03d}",
        "track_id": str(row["track_id"]),
        "file_path": str(row["file_path"]),
        "artist": str(row["artist"]),
        "title": str(row["title"]),
        "album": str(row["album"]),
        "current_genre_sampling_only": str(row["current_genre"]),
        "broad_sampling_stratum": str(row["broad_sampling_stratum"]),
        "artist_group": str(row["artist_group"]),
        "release_group": str(row["release_group"]),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = [
        json.loads(path.read_text(encoding="utf-8"))
        for path in args.development_manifest
    ]
    exposed = [
        json.loads(path.read_text(encoding="utf-8")) for path in args.exposed_result
    ]
    tracks, playlists = load_library_snapshot(args.sqlcipher, args.database)
    tracks_by_path = {str(row["file_path"]): row for row in tracks}

    exclusion_playlists = set(audit["exclusion_playlists"])
    exclusion_playlists.add("genre_verified")
    excluded_paths, excluded_artists, excluded_releases = development_identities(
        tracks_by_path,
        playlists,
        development,
        exclusion_playlists,
        exposed,
    )
    eligible, exclusions = eligible_rows(
        audit["rows"],
        tracks_by_path,
        excluded_paths,
        excluded_artists,
        excluded_releases,
    )
    selected_rows = select_holdout(eligible, args.count)
    selected = [
        selected_private_row(row, index + 1)
        for index, row in enumerate(selected_rows)
    ]
    roster_sha = private_fingerprint(selected)
    target_counts = Counter(row["broad_sampling_stratum"] for row in selected)
    snapshot = {
        "tracks": tracks,
        "playlists": {
            key: sorted(value) for key, value in sorted(playlists.items())
        },
    }
    result = {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "sealed_before_new_representation_inference",
        "inputs": {
            "audit_manifest_sha256": sha256_file(args.audit_manifest),
            "development_manifest_sha256": [
                sha256_file(path) for path in args.development_manifest
            ],
            "exposed_result_sha256": [
                sha256_file(path) for path in args.exposed_result
            ],
            "library_snapshot_sha256": private_fingerprint(snapshot),
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(SEED.encode("utf-8")).hexdigest(),
            "count": args.count,
            "maximum_per_broad_sampling_stratum": MAX_PER_TARGET,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "current_genre_is_sampling_only": True,
            "exclusion_playlists": sorted(exclusion_playlists),
        },
        "universe": {
            "library_rows": len(tracks),
            "audit_rows": len(audit["rows"]),
            "eligible_rows": len(eligible),
            "eligible_artists": len(
                {row["artist_group"] for row in eligible}
            ),
            "eligible_release_groups": len(
                {row["release_group"] for row in eligible}
            ),
            "exclusions": dict(sorted(exclusions.items())),
        },
        "target_counts": dict(sorted(target_counts.items())),
        "roster_sha256": roster_sha,
        "selected": selected,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument(
        "--development-manifest", required=True, action="append", type=Path
    )
    parser.add_argument("--exposed-result", required=True, action="append", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    result = run(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "experiment_id": result["experiment_id"],
                "method_status": result["method_status"],
                "roster_sha256": result["roster_sha256"],
                "universe": result["universe"],
                "target_counts": result["target_counts"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
