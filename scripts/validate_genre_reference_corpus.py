#!/usr/bin/env python3
"""Validate the public genre-reference candidate corpus without network access."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import Counter, defaultdict
from datetime import date
from pathlib import Path
from typing import Any
from urllib.parse import urlparse


CATALOG_PATH = Path("src/domain/classification/taxonomy/catalog.rs")
EXPECTED_EXCLUSION = "Experimental"
EXPECTED_CATALOG_COUNT = 53
EXPECTED_CORPUS_GENRE_COUNT = 52
MIN_TOTAL_CANDIDATES = EXPECTED_CORPUS_GENRE_COUNT * 12
PLAYLIST_NAME = "genre_reference_candidates"
TRAINING_PLAYLIST = "genre_verified"
HOLDOUT_PLAYLIST = "genre_reference_holdout"

FAMILIES = {"House", "Techno", "Hardcore", "Bass", "Downtempo", "Other"}
DISPOSITIONS = {"audio_reference", "metadata_led", "taxonomy_review"}
REFERENCE_ROLES = {"foundational", "representative", "contemporary", "boundary"}
POOL_ROLES = {"training_anchor", "holdout_candidate", "boundary_review"}
CONFIDENCE_LEVELS = {"high", "medium"}
SOURCE_TYPES = {
    "academic",
    "archival",
    "artist_history",
    "institutional",
    "label_history",
    "release_metadata",
    "respected_editorial",
    "scene_history",
}
STRONG_DEFINITION_SOURCE_TYPES = {
    "academic",
    "archival",
    "artist_history",
    "institutional",
    "label_history",
    "respected_editorial",
    "scene_history",
}
FORBIDDEN_PRIVATE_KEYS = {
    "account",
    "account_id",
    "audio_fingerprint",
    "file_path",
    "fingerprint",
    "owned",
    "ownership",
    "price",
    "price_paid",
    "rekordbox_id",
    "sha256",
    "track_id",
}
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
CATALOG_RE = re.compile(r'pub const GENRES:.*?=\s*&\[(.*?)\];', re.DOTALL)
STRING_RE = re.compile(r'"([^"\\]*(?:\\.[^"\\]*)*)"')


def normalize(value: str) -> str:
    """Normalize display text for deterministic duplicate/concentration checks."""
    return re.sub(r"[^a-z0-9]+", "", value.casefold())


def load_catalog(repo_root: Path) -> list[str]:
    catalog_file = repo_root / CATALOG_PATH
    try:
        text = catalog_file.read_text(encoding="utf-8")
    except OSError as exc:
        raise ValueError(f"cannot read taxonomy catalog {catalog_file}: {exc}") from exc
    match = CATALOG_RE.search(text)
    if not match:
        raise ValueError(f"cannot locate GENRES array in {catalog_file}")
    names = [bytes(value, "utf-8").decode("unicode_escape") for value in STRING_RE.findall(match.group(1))]
    if not names:
        raise ValueError(f"GENRES array in {catalog_file} is empty")
    return names


def is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def is_iso_date(value: Any) -> bool:
    if not isinstance(value, str) or not DATE_RE.fullmatch(value):
        return False
    try:
        date.fromisoformat(value)
    except ValueError:
        return False
    return True


def is_https_url(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    parsed = urlparse(value)
    return parsed.scheme == "https" and bool(parsed.netloc) and not parsed.fragment


def find_forbidden_fields(value: Any, location: str = "$") -> list[str]:
    errors: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            normalized_key = key.casefold().replace("-", "_")
            if normalized_key in FORBIDDEN_PRIVATE_KEYS or "fingerprint" in normalized_key:
                errors.append(f"{location}.{key}: forbidden private-data field")
            errors.extend(find_forbidden_fields(child, f"{location}.{key}"))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            errors.extend(find_forbidden_fields(child, f"{location}[{index}]"))
    return errors


def require_keys(record: dict[str, Any], keys: set[str], location: str, errors: list[str]) -> None:
    for key in sorted(keys - set(record)):
        errors.append(f"{location}: missing required field '{key}'")


def validate_source(source: Any, location: str, errors: list[str]) -> None:
    if not isinstance(source, dict):
        errors.append(f"{location}: source must be an object")
        return
    required = {"id", "title", "publisher", "url", "source_type", "claim", "accessed_on"}
    require_keys(source, required, location, errors)
    for field in ("id", "title", "publisher", "claim"):
        if field in source and not is_nonempty_string(source[field]):
            errors.append(f"{location}.{field}: must be a non-empty string")
    if "url" in source and not is_https_url(source["url"]):
        errors.append(f"{location}.url: must be a direct HTTPS URL without a fragment")
    if source.get("source_type") not in SOURCE_TYPES:
        errors.append(f"{location}.source_type: invalid source type {source.get('source_type')!r}")
    if "accessed_on" in source and not is_iso_date(source["accessed_on"]):
        errors.append(f"{location}.accessed_on: must be an ISO date")


def validate_acquisition(acquisition: Any, location: str, errors: list[str]) -> None:
    if not isinstance(acquisition, dict):
        errors.append(f"{location}: acquisition must be an object")
        return
    required = {"store", "url", "formats", "accessed_on", "australian_region_caveat"}
    require_keys(acquisition, required, location, errors)
    if "store" in acquisition and not is_nonempty_string(acquisition["store"]):
        errors.append(f"{location}.store: must be a non-empty string")
    if "url" in acquisition and not is_https_url(acquisition["url"]):
        errors.append(f"{location}.url: must be a direct HTTPS URL without a fragment")
    formats = acquisition.get("formats")
    if not isinstance(formats, list) or not formats or not all(is_nonempty_string(item) for item in formats):
        errors.append(f"{location}.formats: must list at least one advertised digital format")
    if "accessed_on" in acquisition and not is_iso_date(acquisition["accessed_on"]):
        errors.append(f"{location}.accessed_on: must be an ISO date")
    caveat = acquisition.get("australian_region_caveat")
    if caveat is not None and not is_nonempty_string(caveat):
        errors.append(f"{location}.australian_region_caveat: must be null or a non-empty string")


def validate_candidate(
    candidate: Any,
    location: str,
    sources_by_id: dict[str, dict[str, Any]],
    errors: list[str],
) -> None:
    if not isinstance(candidate, dict):
        errors.append(f"{location}: candidate must be an object")
        return
    required = {
        "id",
        "artist",
        "track_title",
        "mix_version",
        "original_release",
        "label",
        "original_year",
        "catalog_number",
        "reference_role",
        "era_bucket",
        "substyle_scene",
        "canonicality_rationale",
        "canonicality_source_ids",
        "metadata_source_ids",
        "acquisitions",
        "confidence",
        "leakage_group",
        "recommended_pool_role",
    }
    require_keys(candidate, required, location, errors)
    for field in (
        "id",
        "artist",
        "track_title",
        "mix_version",
        "original_release",
        "label",
        "era_bucket",
        "substyle_scene",
        "canonicality_rationale",
        "leakage_group",
    ):
        if field in candidate and not is_nonempty_string(candidate[field]):
            errors.append(f"{location}.{field}: must be a non-empty string")
    year = candidate.get("original_year")
    if not isinstance(year, int) or isinstance(year, bool) or year < 1900 or year > date.today().year:
        errors.append(f"{location}.original_year: must be an integer from 1900 through {date.today().year}")
    catalog_number = candidate.get("catalog_number")
    if catalog_number is not None and not is_nonempty_string(catalog_number):
        errors.append(f"{location}.catalog_number: must be null or a non-empty string")
    if candidate.get("reference_role") not in REFERENCE_ROLES:
        errors.append(f"{location}.reference_role: invalid role {candidate.get('reference_role')!r}")
    if candidate.get("recommended_pool_role") not in POOL_ROLES:
        errors.append(f"{location}.recommended_pool_role: invalid pool role {candidate.get('recommended_pool_role')!r}")
    if candidate.get("confidence") not in CONFIDENCE_LEVELS:
        errors.append(f"{location}.confidence: must be 'high' or 'medium'")

    canonicality_ids = candidate.get("canonicality_source_ids")
    if not isinstance(canonicality_ids, list) or len(set(canonicality_ids)) < 2:
        errors.append(f"{location}.canonicality_source_ids: must contain at least two distinct source IDs")
        canonicality_ids = []
    metadata_ids = candidate.get("metadata_source_ids")
    if not isinstance(metadata_ids, list) or not metadata_ids:
        errors.append(f"{location}.metadata_source_ids: must contain at least one release/version source ID")
        metadata_ids = []

    for field_name, source_ids in (
        ("canonicality_source_ids", canonicality_ids),
        ("metadata_source_ids", metadata_ids),
    ):
        if not all(is_nonempty_string(source_id) for source_id in source_ids):
            errors.append(f"{location}.{field_name}: all IDs must be non-empty strings")
            continue
        unknown = sorted(set(source_ids) - set(sources_by_id))
        if unknown:
            errors.append(f"{location}.{field_name}: unknown source IDs {unknown}")

    canonicality_sources = [sources_by_id[source_id] for source_id in set(canonicality_ids) if source_id in sources_by_id]
    canonicality_publishers = {normalize(source["publisher"]) for source in canonicality_sources}
    canonicality_urls = {source["url"] for source in canonicality_sources}
    if len(canonicality_publishers) < 2 or len(canonicality_urls) < 2:
        errors.append(f"{location}.canonicality_source_ids: sources must have two independent publishers and URLs")
    if any(source.get("source_type") == "release_metadata" for source in canonicality_sources):
        errors.append(f"{location}.canonicality_source_ids: release metadata cannot substitute for canonicality evidence")

    acquisitions = candidate.get("acquisitions")
    if not isinstance(acquisitions, list) or not acquisitions:
        errors.append(f"{location}.acquisitions: must contain at least one current digital-purchase route")
    else:
        for index, acquisition in enumerate(acquisitions):
            validate_acquisition(acquisition, f"{location}.acquisitions[{index}]", errors)


def validate_genre(
    genre: dict[str, Any],
    location: str,
    canonical_names: set[str],
    allow_incomplete: bool,
    errors: list[str],
) -> bool:
    required = {
        "genre",
        "classifier_family",
        "research_disposition",
        "working_definition",
        "explicit_exclusions",
        "boundary_genres",
        "sources",
        "definition_source_ids",
        "research_caveats",
        "candidates",
    }
    require_keys(genre, required, location, errors)
    candidates = genre.get("candidates")
    if not isinstance(candidates, list):
        errors.append(f"{location}.candidates: must be an array")
        return False
    populated = bool(candidates)
    if allow_incomplete and not populated:
        return False

    for field in ("genre", "working_definition"):
        if not is_nonempty_string(genre.get(field)):
            errors.append(f"{location}.{field}: must be a non-empty string")
    if genre.get("classifier_family") not in FAMILIES:
        errors.append(f"{location}.classifier_family: invalid family {genre.get('classifier_family')!r}")
    if genre.get("research_disposition") not in DISPOSITIONS:
        errors.append(f"{location}.research_disposition: invalid disposition {genre.get('research_disposition')!r}")
    for field in ("explicit_exclusions", "research_caveats"):
        values = genre.get(field)
        if not isinstance(values, list) or not all(is_nonempty_string(value) for value in values):
            errors.append(f"{location}.{field}: must be an array of non-empty strings")
    boundary_genres = genre.get("boundary_genres")
    if not isinstance(boundary_genres, list) or not boundary_genres:
        errors.append(f"{location}.boundary_genres: must contain at least one canonical neighbor")
    elif not all(boundary in canonical_names for boundary in boundary_genres):
        errors.append(f"{location}.boundary_genres: every boundary must be a live canonical genre")

    sources = genre.get("sources")
    if not isinstance(sources, list):
        errors.append(f"{location}.sources: must be an array")
        sources = []
    source_ids: list[str] = []
    source_urls: list[str] = []
    for index, source in enumerate(sources):
        validate_source(source, f"{location}.sources[{index}]", errors)
        if isinstance(source, dict) and is_nonempty_string(source.get("id")):
            source_ids.append(source["id"])
        if isinstance(source, dict) and is_https_url(source.get("url")):
            source_urls.append(source["url"])
    if len(source_ids) != len(set(source_ids)):
        errors.append(f"{location}.sources: source IDs must be unique within the genre")
    if len(source_urls) != len(set(source_urls)):
        errors.append(f"{location}.sources: repeated URLs do not count as independent sources")
    sources_by_id = {
        source["id"]: source
        for source in sources
        if isinstance(source, dict) and is_nonempty_string(source.get("id"))
    }

    definition_ids = genre.get("definition_source_ids")
    if not isinstance(definition_ids, list) or len(set(definition_ids)) < 3:
        errors.append(f"{location}.definition_source_ids: must contain at least three distinct source IDs")
        definition_ids = []
    unknown_definition_ids = sorted(set(definition_ids) - set(sources_by_id))
    if unknown_definition_ids:
        errors.append(f"{location}.definition_source_ids: unknown source IDs {unknown_definition_ids}")
    definition_sources = [sources_by_id[source_id] for source_id in set(definition_ids) if source_id in sources_by_id]
    if len({normalize(source["publisher"]) for source in definition_sources}) < 2:
        errors.append(f"{location}.definition_source_ids: must span at least two independent publishers")
    if not any(source.get("source_type") in STRONG_DEFINITION_SOURCE_TYPES for source in definition_sources):
        errors.append(f"{location}.definition_source_ids: must include a strong historical, institutional, artist, label, or scene source")

    for index, candidate in enumerate(candidates):
        validate_candidate(candidate, f"{location}.candidates[{index}]", sources_by_id, errors)

    if len(candidates) < 12:
        errors.append(f"{location}.candidates: requires at least 12 candidates, found {len(candidates)}")
        return True

    reference_counts = Counter(candidate.get("reference_role") for candidate in candidates if isinstance(candidate, dict))
    for role, minimum in {"foundational": 4, "representative": 4, "contemporary": 2, "boundary": 2}.items():
        if reference_counts[role] < minimum:
            errors.append(f"{location}.candidates: requires at least {minimum} {role} candidates")
    pool_counts = Counter(candidate.get("recommended_pool_role") for candidate in candidates if isinstance(candidate, dict))
    for role, minimum in {"training_anchor": 6, "holdout_candidate": 4, "boundary_review": 2}.items():
        if pool_counts[role] < minimum:
            errors.append(f"{location}.candidates: requires at least {minimum} {role} recommendations")

    artists = Counter(normalize(candidate.get("artist", "")) for candidate in candidates if isinstance(candidate, dict))
    labels = Counter(normalize(candidate.get("label", "")) for candidate in candidates if isinstance(candidate, dict))
    eras = {normalize(candidate.get("era_bucket", "")) for candidate in candidates if isinstance(candidate, dict)}
    if len(artists) < 8:
        errors.append(f"{location}.candidates: requires at least 8 distinct lead artists/acts")
    overused_artists = sorted(artist for artist, count in artists.items() if artist and count > 2)
    if overused_artists:
        errors.append(f"{location}.candidates: artist/act exceeds two candidates: {overused_artists}")
    if len(labels) < 4:
        errors.append(f"{location}.candidates: requires at least 4 distinct labels")
    label_limit = len(candidates) / 4
    overused_labels = sorted(label for label, count in labels.items() if label and count > label_limit)
    if overused_labels:
        errors.append(f"{location}.candidates: label exceeds 25% concentration: {overused_labels}")
    if len(eras) < 3:
        errors.append(f"{location}.candidates: requires at least 3 meaningful era or scene buckets")

    catalog_numbers = [
        normalize(candidate["catalog_number"])
        for candidate in candidates
        if isinstance(candidate, dict) and is_nonempty_string(candidate.get("catalog_number"))
    ]
    duplicate_catalogs = sorted(number for number, count in Counter(catalog_numbers).items() if count > 1)
    if duplicate_catalogs:
        errors.append(f"{location}.candidates: duplicate exact catalog numbers {duplicate_catalogs}")
    return True


def validate_document(document: Any, repo_root: Path, allow_incomplete: bool = False) -> tuple[list[str], dict[str, int]]:
    errors: list[str] = []
    summary = {"expected_genres": 0, "populated_genres": 0, "candidates": 0}
    if not isinstance(document, dict):
        return ["$: top-level JSON value must be an object"], summary
    errors.extend(find_forbidden_fields(document))
    required = {
        "schema_version",
        "playlist_name",
        "approved_training_playlist",
        "approved_holdout_playlist",
        "taxonomy_source",
        "research_completed_on",
        "excluded_genres",
        "genres",
    }
    require_keys(document, required, "$", errors)
    if document.get("schema_version") != 1:
        errors.append("$.schema_version: must equal 1")
    if document.get("playlist_name") != PLAYLIST_NAME:
        errors.append(f"$.playlist_name: must equal {PLAYLIST_NAME!r}")
    if document.get("approved_training_playlist") != TRAINING_PLAYLIST:
        errors.append(f"$.approved_training_playlist: must equal {TRAINING_PLAYLIST!r}")
    if document.get("approved_holdout_playlist") != HOLDOUT_PLAYLIST:
        errors.append(f"$.approved_holdout_playlist: must equal {HOLDOUT_PLAYLIST!r}")
    if not is_iso_date(document.get("research_completed_on")):
        errors.append("$.research_completed_on: must be an ISO date")

    taxonomy_source = document.get("taxonomy_source")
    if not isinstance(taxonomy_source, dict):
        errors.append("$.taxonomy_source: must be an object")
    else:
        if taxonomy_source.get("path") != CATALOG_PATH.as_posix():
            errors.append(f"$.taxonomy_source.path: must equal {CATALOG_PATH.as_posix()!r}")
        if not isinstance(taxonomy_source.get("commit"), str) or not COMMIT_RE.fullmatch(taxonomy_source["commit"]):
            errors.append("$.taxonomy_source.commit: must be a full lowercase Git commit hash")

    try:
        catalog = load_catalog(repo_root)
    except ValueError as exc:
        errors.append(str(exc))
        return errors, summary
    canonical_names = set(catalog)
    expected_genres = canonical_names - {EXPECTED_EXCLUSION}
    summary["expected_genres"] = len(expected_genres)
    if (
        len(catalog) != EXPECTED_CATALOG_COUNT
        or EXPECTED_EXCLUSION not in canonical_names
        or len(expected_genres) != EXPECTED_CORPUS_GENRE_COUNT
    ):
        errors.append(
            "taxonomy contract: expected "
            f"{EXPECTED_CATALOG_COUNT} canonical genres and exactly Experimental excluded "
            f"to leave {EXPECTED_CORPUS_GENRE_COUNT}"
        )

    exclusions = document.get("excluded_genres")
    expected_exclusions = [{"genre": EXPECTED_EXCLUSION, "reason": "anti-genre/umbrella category; excluded by operator decision"}]
    if exclusions != expected_exclusions:
        errors.append("$.excluded_genres: must contain exactly the operator-approved Experimental exclusion")

    genres = document.get("genres")
    if not isinstance(genres, list):
        errors.append("$.genres: must be an array")
        return errors, summary
    genre_names = [genre.get("genre") for genre in genres if isinstance(genre, dict)]
    if len(genre_names) != len(set(genre_names)):
        errors.append("$.genres: genre names must be unique")
    missing = sorted(expected_genres - set(genre_names))
    extra = sorted(set(genre_names) - expected_genres)
    if missing:
        errors.append(f"$.genres: missing canonical genres {missing}")
    if extra:
        errors.append(f"$.genres: unexpected or excluded genres {extra}")
    if len(genres) != EXPECTED_CORPUS_GENRE_COUNT:
        errors.append(
            f"$.genres: expected exactly {EXPECTED_CORPUS_GENRE_COUNT} genre records, "
            f"found {len(genres)}"
        )

    global_candidate_ids: dict[str, str] = {}
    global_identities: dict[tuple[str, str, str], str] = {}
    leakage_pools: dict[str, set[str]] = defaultdict(set)
    for index, genre in enumerate(genres):
        location = f"$.genres[{index}]"
        if not isinstance(genre, dict):
            errors.append(f"{location}: genre must be an object")
            continue
        populated = validate_genre(genre, location, canonical_names, allow_incomplete, errors)
        if populated:
            summary["populated_genres"] += 1
        candidates = genre.get("candidates")
        if not isinstance(candidates, list):
            continue
        summary["candidates"] += len(candidates)
        if genre.get("genre") == EXPECTED_EXCLUSION and candidates:
            errors.append(f"{location}.candidates: Experimental must never contain candidates")
        for candidate_index, candidate in enumerate(candidates):
            if not isinstance(candidate, dict):
                continue
            candidate_location = f"{location}.candidates[{candidate_index}]"
            candidate_id = candidate.get("id")
            if is_nonempty_string(candidate_id):
                normalized_id = normalize(candidate_id)
                if normalized_id in global_candidate_ids:
                    errors.append(f"{candidate_location}.id: duplicates {global_candidate_ids[normalized_id]}")
                else:
                    global_candidate_ids[normalized_id] = candidate_location
            identity = tuple(normalize(candidate.get(field, "")) for field in ("artist", "track_title", "mix_version"))
            if all(identity):
                if identity in global_identities:
                    errors.append(f"{candidate_location}: duplicate normalized artist/title/version identity; first at {global_identities[identity]}")
                else:
                    global_identities[identity] = candidate_location
            leakage_group = candidate.get("leakage_group")
            pool = candidate.get("recommended_pool_role")
            if is_nonempty_string(leakage_group) and pool in {"training_anchor", "holdout_candidate"}:
                leakage_pools[normalize(leakage_group)].add(pool)

    for leakage_group, pools in sorted(leakage_pools.items()):
        if pools == {"training_anchor", "holdout_candidate"}:
            errors.append(f"$.genres: leakage group {leakage_group!r} is split across training and holdout pools")
    if not allow_incomplete and summary["populated_genres"] != EXPECTED_CORPUS_GENRE_COUNT:
        errors.append(
            f"$.genres: complete mode requires {EXPECTED_CORPUS_GENRE_COUNT} populated genres, "
            f"found {summary['populated_genres']}"
        )
    if not allow_incomplete and summary["candidates"] < MIN_TOTAL_CANDIDATES:
        errors.append(
            f"$.genres: complete mode requires at least {MIN_TOTAL_CANDIDATES} candidates, "
            f"found {summary['candidates']}"
        )
    return errors, summary


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-incomplete", action="store_true", help="allow empty genre records while fully validating populated genres")
    parser.add_argument("corpus", type=Path, help="path to genre-reference-candidates.json")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        with args.corpus.open(encoding="utf-8") as corpus_file:
            document = json.load(corpus_file)
    except (OSError, json.JSONDecodeError) as exc:
        print(f"genre reference corpus: cannot load {args.corpus}: {exc}", file=sys.stderr)
        return 2
    errors, summary = validate_document(document, args.repo_root.resolve(), args.allow_incomplete)
    mode = "incomplete" if args.allow_incomplete else "complete"
    print(
        "genre reference corpus: "
        f"mode={mode}, expected_genres={summary['expected_genres']}, "
        f"populated_genres={summary['populated_genres']}, candidates={summary['candidates']}"
    )
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("genre reference corpus: validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
