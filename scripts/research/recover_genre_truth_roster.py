#!/usr/bin/env python3
"""Recover the exact retired Plan 067 roster after temporary files are lost."""

from __future__ import annotations

import argparse
import json
import os
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Iterable

import select_broad_genre_holdout as broad
import select_scoped_broad_genre_holdout as scoped


EXPECTED_LIBRARY_SNAPSHOT_SHA256 = (
    "553dcfd0c5526f2ca8309f1a53097ac58749c8d54216879c80390c355264e653"
)
EXPECTED_PLAN_066_ROSTER_SHA256 = (
    "e90b400645d89b287aab4300465fd0893314830bc6ec8b6ab22b5f9de4fbfdf9"
)
EXPECTED_PLAN_067_ARTIFACT_SHA256 = (
    "7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f"
)
EXPECTED_PLAN_067_ROSTER_SHA256 = (
    "9cf4cdbd67bc701063d886e991e7f4f57a0b675844423584b3027f0bce5418a9"
)
EXPECTED_PLAN_066_ELIGIBLE_ROWS = 707
EXPECTED_PLAN_067_ELIGIBLE_ROWS = 490
EXPECTED_RECOVERED_PLAN_066_ELIGIBLE_ROWS = 706
EXPECTED_RECOVERED_PLAN_067_ELIGIBLE_ROWS = 489
EXPECTED_PLAN_067_TARGET_COUNTS = {
    "Ambient": 6,
    "Breakbeat": 7,
    "Disco": 3,
    "Downtempo": 2,
    "Drum & Bass": 3,
    "Electro": 2,
    "House": 8,
    "Pop": 1,
    "Techno": 8,
    "Trance": 8,
}


def rekordbox_xml_paths(path: Path) -> list[str]:
    root = ET.parse(path).getroot()
    locations = []
    for track in root.findall("./COLLECTION/TRACK"):
        location = track.get("Location")
        if not location:
            raise ValueError(f"XML track has no Location: {path}")
        parsed = urllib.parse.urlsplit(location)
        if parsed.scheme != "file" or parsed.netloc not in {"", "localhost"}:
            raise ValueError(f"unsupported Rekordbox Location: {location}")
        locations.append(urllib.parse.unquote(parsed.path))
    if not locations:
        raise ValueError(f"XML collection has no tracks: {path}")
    if len(set(locations)) != len(locations):
        raise ValueError(f"XML collection contains duplicate paths: {path}")
    return locations


def recovered_audit_manifest(
    current: dict[str, Any],
    supplemental_paths: Iterable[str],
    historical_development_paths: set[str],
    tracks_by_path: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    rows_by_path = {
        str(row["file_path"]): dict(row)
        for row in current.get("rows", [])
        if str(row["file_path"]) not in historical_development_paths
    }
    for path in supplemental_paths:
        if path in historical_development_paths or path in rows_by_path:
            continue
        live = tracks_by_path.get(path)
        if live is None:
            raise ValueError(f"supplemental audit identity is no longer live: {path}")
        rows_by_path[path] = {
            "track_id": str(live["track_id"]),
            "file_path": path,
        }
    result = dict(current)
    result["rows"] = sorted(
        rows_by_path.values(), key=lambda row: (str(row["track_id"]), row["file_path"])
    )
    result["recovery"] = {
        "purpose": "identity-only replay of the retired Plan 066 and Plan 067 selectors",
        "supplemental_review_rows": len(set(supplemental_paths)),
        "historical_development_rows_removed": len(historical_development_paths),
        "model_features_or_predictions_used": False,
    }
    return result


def minimal_rows(paths: Iterable[str]) -> dict[str, Any]:
    return {"rows": [{"file_path": path} for path in sorted(set(paths))]}


def minimal_selected(paths: Iterable[str]) -> dict[str, Any]:
    return {"selected": [{"file_path": path} for path in sorted(set(paths))]}


def write_private_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(path, 0o600)


def require_equal(label: str, actual: Any, expected: Any) -> None:
    if actual != expected:
        raise ValueError(f"{label} differs: expected {expected!r}, got {actual!r}")


def run(args: argparse.Namespace) -> dict[str, Any]:
    tracks, _ = broad.load_library_snapshot(args.sqlcipher, args.database)
    tracks_by_path = {str(row["file_path"]): row for row in tracks}
    supplemental_paths = [
        path
        for xml_path in args.supplemental_review_xml
        for path in rekordbox_xml_paths(xml_path)
    ]
    historical_paths = set(args.historical_development_path)
    current_audit = json.loads(
        args.current_audit_manifest.read_text(encoding="utf-8")
    )
    recovered_audit = recovered_audit_manifest(
        current_audit,
        supplemental_paths,
        historical_paths,
        tracks_by_path,
    )

    recovered_audit_path = args.output_dir / "audit-manifest-recovered.json"
    historical_development_path = (
        args.output_dir / "historical-development-exclusions.json"
    )
    exposed_paths = []
    for index, xml_path in enumerate(args.exposed_xml, start=1):
        output = args.output_dir / f"exposed-result-{index:02d}.json"
        write_private_json(output, minimal_selected(rekordbox_xml_paths(xml_path)))
        exposed_paths.append(output)
    write_private_json(recovered_audit_path, recovered_audit)
    write_private_json(
        historical_development_path,
        minimal_rows(historical_paths),
    )

    plan_066_path = args.output_dir / "plan-066-holdout-recovered.json"
    plan_066_args = SimpleNamespace(
        database=args.database,
        audit_manifest=recovered_audit_path,
        development_manifest=[
            historical_development_path,
            args.current_development_manifest,
        ],
        exposed_result=exposed_paths,
        count=60,
        sqlcipher=args.sqlcipher,
    )
    plan_066 = broad.run(plan_066_args)
    require_equal(
        "library snapshot",
        plan_066["inputs"]["library_snapshot_sha256"],
        EXPECTED_LIBRARY_SNAPSHOT_SHA256,
    )
    require_equal(
        "recovered Plan 066 eligible rows",
        plan_066["universe"]["eligible_rows"],
        EXPECTED_RECOVERED_PLAN_066_ELIGIBLE_ROWS,
    )
    require_equal(
        "Plan 066 roster",
        plan_066["roster_sha256"],
        EXPECTED_PLAN_066_ROSTER_SHA256,
    )
    write_private_json(plan_066_path, plan_066)

    plan_067_path = args.output_dir / "plan-067-roster-recovered.json"
    plan_067_args = SimpleNamespace(
        database=args.database,
        audit_manifest=recovered_audit_path,
        development_manifest=[
            historical_development_path,
            args.current_development_manifest,
        ],
        exposed_result=exposed_paths,
        prior_holdout=[plan_066_path],
        count=48,
        sqlcipher=args.sqlcipher,
    )
    plan_067 = scoped.run(plan_067_args)
    require_equal(
        "recovered Plan 067 eligible rows",
        plan_067["universe"]["eligible_rows"],
        EXPECTED_RECOVERED_PLAN_067_ELIGIBLE_ROWS,
    )
    require_equal(
        "Plan 067 target counts",
        plan_067["target_counts"],
        EXPECTED_PLAN_067_TARGET_COUNTS,
    )
    require_equal(
        "Plan 067 roster",
        plan_067["roster_sha256"],
        EXPECTED_PLAN_067_ROSTER_SHA256,
    )
    replay = scoped.run(plan_067_args)
    require_equal("Plan 067 replay", replay, plan_067)
    plan_067["recovery"] = {
        "original_artifact_sha256": EXPECTED_PLAN_067_ARTIFACT_SHA256,
        "original_roster_sha256": EXPECTED_PLAN_067_ROSTER_SHA256,
        "plan_066_roster_sha256": EXPECTED_PLAN_066_ROSTER_SHA256,
        "library_snapshot_sha256": EXPECTED_LIBRARY_SNAPSHOT_SHA256,
        "identity_roster_replayed_exactly": True,
        "byte_identical_replay": True,
        "historical_plan_066_eligible_rows": EXPECTED_PLAN_066_ELIGIBLE_ROWS,
        "recovered_plan_066_eligible_rows": (
            EXPECTED_RECOVERED_PLAN_066_ELIGIBLE_ROWS
        ),
        "historical_plan_067_eligible_rows": EXPECTED_PLAN_067_ELIGIBLE_ROWS,
        "recovered_plan_067_eligible_rows": (
            EXPECTED_RECOVERED_PLAN_067_ELIGIBLE_ROWS
        ),
        "unselected_candidate_universe_gap_rows": 1,
        "model_features_or_predictions_used": False,
    }
    write_private_json(plan_067_path, plan_067)

    return {
        "output": str(plan_067_path),
        "rows": len(plan_067["selected"]),
        "recovered_audit_rows": len(recovered_audit["rows"]),
        "historical_eligible_rows": EXPECTED_PLAN_067_ELIGIBLE_ROWS,
        "recovered_eligible_rows": plan_067["universe"]["eligible_rows"],
        "roster_sha256": plan_067["roster_sha256"],
        "original_artifact_sha256": EXPECTED_PLAN_067_ARTIFACT_SHA256,
        "library_snapshot_sha256": EXPECTED_LIBRARY_SNAPSHOT_SHA256,
        "replay_verified": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--current-audit-manifest", required=True, type=Path)
    parser.add_argument("--current-development-manifest", required=True, type=Path)
    parser.add_argument(
        "--supplemental-review-xml", required=True, action="append", type=Path
    )
    parser.add_argument("--exposed-xml", required=True, action="append", type=Path)
    parser.add_argument(
        "--historical-development-path", required=True, action="append"
    )
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--sqlcipher", default="sqlcipher")
    args = parser.parse_args()
    print(json.dumps(run(args), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
