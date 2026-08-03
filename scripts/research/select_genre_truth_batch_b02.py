#!/usr/bin/env python3
"""Select Genre Intelligence blind truth batch B02 from a pre-holdout XML."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any
from urllib.parse import unquote, urlparse
import xml.etree.ElementTree as ET

import select_broad_genre_holdout as base
import select_genre_truth_batch as truth


EXPERIMENT_ID = "genre-intelligence-truth-v1-b02"
SEED = EXPERIMENT_ID
SOURCE_XML_SHA256 = (
    "67d6f19adfe7acb642cfd9d2fead240ede6c392dce5465a64c6329656d3b9a18"
)
REQUIRED_PRIOR_PLAYLIST = "genre_reference_candidates"
FORBIDDEN_PLAYLISTS = frozenset(
    {
        "genre_verified",
        "genre_discovery_blind_v1",
        "genre_discovery_v2_tech_house_batch_01",
        "genre_discovery_v3_tech_house_batch_01",
        "minimal_candidates",
        "minimal_research_candidates_v2",
        "tech_house_research_candidates_v2",
        "genre_intelligence_blind_v1_b01",
    }
)
QUOTAS = {"IDM": 1, "Minimal": 5}


def stable_hash(row: dict[str, Any]) -> str:
    value = "|".join(
        (
            SEED,
            str(row["sampling_stratum_private"]),
            str(row["track_id"]),
            str(row["file_path"]),
        )
    )
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


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
                if row["sampling_stratum_private"] == stratum
            ),
            key=stable_hash,
        )
        accepted = 0
        for row in candidates:
            path = str(row["file_path"])
            artist = str(row["artist_group"])
            release = str(row["release_group"])
            if path in used_paths or artist in used_artists or release in used_releases:
                continue
            selected.append(dict(row))
            used_paths.add(path)
            used_artists.add(artist)
            used_releases.add(release)
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
    for position, row in enumerate(selected, 1):
        row["position"] = position
        row["code"] = f"GI02-{position:02d}"
    return selected


def source_rows(xml_path: Path) -> list[dict[str, str]]:
    if truth.sha256_file(xml_path) != SOURCE_XML_SHA256:
        raise ValueError("pre-holdout candidate XML identity changed")
    root = ET.parse(xml_path).getroot()
    rows = []
    for node in root.findall("./COLLECTION/TRACK"):
        location = urlparse(node.get("Location", ""))
        rows.append(
            {
                "file_path": unquote(location.path),
                "artist": node.get("Artist", ""),
                "title": node.get("Name", ""),
                "album": node.get("Album", ""),
                "sampling_stratum_private": node.get("Genre", ""),
            }
        )
    return rows


def eligible_rows(
    source: list[dict[str, str]],
    tracks: list[dict[str, Any]],
    memberships: dict[str, set[str]],
    reviewed_paths: set[str],
) -> list[dict[str, Any]]:
    live_by_path = {str(row["file_path"]): row for row in tracks}
    eligible = []
    for candidate in source:
        if candidate["sampling_stratum_private"] not in QUOTAS:
            continue
        path = candidate["file_path"]
        live = live_by_path.get(path)
        if live is None or path in reviewed_paths:
            continue
        playlists = memberships.get(str(live["track_id"]), set())
        if REQUIRED_PRIOR_PLAYLIST not in playlists or playlists & FORBIDDEN_PLAYLISTS:
            continue
        if (
            str(live["artist"]) != candidate["artist"]
            or str(live["title"]) != candidate["title"]
            or str(live["album"]) != candidate["album"]
            or str(live["current_genre"])
            != candidate["sampling_stratum_private"]
        ):
            raise ValueError(f"live identity or sampling drift for {path}")
        eligible.append(
            {
                "track_id": str(live["track_id"]),
                "file_path": path,
                "artist": str(live["artist"]),
                "title": str(live["title"]),
                "album": str(live["album"]),
                "artist_group": base.normalized(str(live["artist"])),
                "release_group": base.release_group(live),
                "sampling_stratum_private": candidate[
                    "sampling_stratum_private"
                ],
                "source_code": "pre_holdout_genre_reference_candidate",
            }
        )
    return eligible


def reviewed_paths(ledger_path: Path) -> set[str]:
    return {
        str(json.loads(line)["track"]["file_path"])
        for line in ledger_path.read_text(encoding="utf-8").splitlines()
        if line.strip()
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--source-xml", required=True, type=Path)
    parser.add_argument("--truth-ledger", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()

    tracks, memberships = base.load_library_snapshot(args.sqlcipher, args.database)
    eligible = eligible_rows(
        source_rows(args.source_xml),
        tracks,
        memberships,
        reviewed_paths(args.truth_ledger),
    )
    selected = select_batch(eligible)
    result = {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "blind_development_truth_review_pending",
        "export_playlist_name": "genre_intelligence_blind_v1_b02",
        "source": {
            "xml_sha256": SOURCE_XML_SHA256,
            "role": "pre_holdout_candidate_export_in_frozen_exclusion_playlist",
            "required_prior_playlist": REQUIRED_PRIOR_PLAYLIST,
            "sealed_holdout_rows_eligible": False,
            "model_predictions_used": False,
        },
        "selection_rule": {
            "seed_sha256": hashlib.sha256(SEED.encode()).hexdigest(),
            "fixed_sampling_quotas": QUOTAS,
            "forbidden_playlists": sorted(FORBIDDEN_PLAYLISTS),
            "one_per_path": True,
            "one_per_normalized_artist": True,
            "one_per_artist_release_group": True,
            "sampling_strata_are_private_and_not_truth": True,
            "review_batch_size": len(selected),
        },
        "universe": {
            "source_rows": len(source_rows(args.source_xml)),
            "eligible_rows": len(eligible),
            "eligible_artists": len({row["artist_group"] for row in eligible}),
            "eligible_release_groups": len(
                {row["release_group"] for row in eligible}
            ),
        },
        "roster_sha256": truth.private_fingerprint(selected),
        "selected": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    print(
        json.dumps(
            {
                "experiment_id": EXPERIMENT_ID,
                "output": str(args.output),
                "eligible_rows": len(eligible),
                "selected_rows": len(selected),
                "roster_sha256": result["roster_sha256"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
