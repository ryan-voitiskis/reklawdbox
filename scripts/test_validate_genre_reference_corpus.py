from __future__ import annotations

import copy
import tempfile
import unittest
from pathlib import Path

from scripts.validate_genre_reference_corpus import CATALOG_PATH, load_catalog, validate_document


REPO_ROOT = Path(__file__).resolve().parents[1]


def slug(value: str) -> str:
    return "".join(character.lower() for character in value if character.isalnum())


def source(source_id: str, publisher: str, source_type: str, url: str) -> dict:
    return {
        "id": source_id,
        "title": f"Synthetic evidence {source_id}",
        "publisher": publisher,
        "url": url,
        "source_type": source_type,
        "claim": "Synthetic evidence used only to exercise deterministic validation.",
        "accessed_on": "2026-07-18",
    }


def build_genre(genre: str, family: str, *, populated: bool = True) -> dict:
    genre_slug = slug(genre)
    sources = [
        source("definition-a", "Synthetic Archive A", "archival", f"https://archive-a.example/{genre_slug}"),
        source("definition-b", "Synthetic Institution B", "institutional", f"https://institution-b.example/{genre_slug}"),
        source("definition-c", "Synthetic Scene C", "scene_history", f"https://scene-c.example/{genre_slug}"),
        source("release-metadata", "Synthetic Metadata D", "release_metadata", f"https://metadata-d.example/{genre_slug}"),
    ]
    candidates = []
    if populated:
        roles = ["foundational"] * 4 + ["representative"] * 4 + ["contemporary"] * 2 + ["boundary"] * 2
        pools = ["training_anchor"] * 6 + ["holdout_candidate"] * 4 + ["boundary_review"] * 2
        for index, (role, pool) in enumerate(zip(roles, pools, strict=True)):
            candidates.append(
                {
                    "id": f"{genre_slug}-{index + 1:02d}",
                    "artist": f"{genre} Synthetic Artist {index % 10}",
                    "track_title": f"{genre} Synthetic Track {index + 1}",
                    "mix_version": "Original Mix",
                    "original_release": f"{genre} Synthetic Release {index + 1}",
                    "label": f"{genre} Synthetic Label {index % 4}",
                    "original_year": 1980 + index * 4,
                    "catalog_number": f"SYN-{genre_slug.upper()}-{index + 1:02d}",
                    "reference_role": role,
                    "era_bucket": f"synthetic-era-{index % 3}",
                    "substyle_scene": "Synthetic fixture scene; not real research data.",
                    "canonicality_rationale": "Synthetic rationale used only to test validator invariants.",
                    "canonicality_source_ids": ["definition-a", "definition-b"],
                    "metadata_source_ids": ["release-metadata"],
                    "acquisitions": [
                        {
                            "store": "Synthetic Store",
                            "url": f"https://store.example/{genre_slug}/{index + 1}",
                            "formats": ["WAV", "FLAC"],
                            "accessed_on": "2026-07-18",
                            "australian_region_caveat": None,
                        }
                    ],
                    "confidence": "high" if index < 8 else "medium",
                    "leakage_group": f"{genre_slug}-synthetic-release-{index + 1}",
                    "recommended_pool_role": pool,
                }
            )
    boundary = "House" if genre != "House" else "Techno"
    return {
        "genre": genre,
        "classifier_family": family,
        "research_disposition": "audio_reference",
        "working_definition": f"Synthetic bounded definition for {genre}.",
        "explicit_exclusions": ["Synthetic neighboring meaning excluded for fixture validation."],
        "boundary_genres": [boundary],
        "sources": sources,
        "definition_source_ids": ["definition-a", "definition-b", "definition-c"],
        "research_caveats": [],
        "candidates": candidates,
    }


def family_for(genre: str) -> str:
    house = {
        "2-Step Garage", "Afro House", "Deep House", "Disco", "Garage", "Gospel House", "House",
        "Italo Disco", "Italodance", "Progressive House", "Speed Garage", "Tech House", "UK Funky",
    }
    techno = {"Acid", "Ambient Techno", "Deep Techno", "Dub Techno", "EBM", "Electro", "Hard Techno", "Minimal", "Psytrance", "Techno", "Trance"}
    hardcore = {"Gabber", "Happy Hardcore", "Hard Trance", "Hardcore", "Hardstyle"}
    bass = {"Bassline", "Breakbeat", "Broken Beat", "Drum & Bass", "Dubstep", "Footwork", "Future Garage", "Grime", "Jungle"}
    downtempo = {"Ambient", "Downtempo", "Dub", "IDM", "Trip-Hop"}
    if genre in house:
        return "House"
    if genre in techno:
        return "Techno"
    if genre in hardcore:
        return "Hardcore"
    if genre in bass:
        return "Bass"
    if genre in downtempo:
        return "Downtempo"
    return "Other"


class GenreReferenceCorpusValidatorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.repo_root = Path(self.temp_dir.name)
        catalog = load_catalog(REPO_ROOT)
        catalog_file = self.repo_root / CATALOG_PATH
        catalog_file.parent.mkdir(parents=True)
        quoted = ",\n    ".join(f'"{genre}"' for genre in catalog)
        catalog_file.write_text(f"pub const GENRES: &[&str] = &[\n    {quoted},\n];\n", encoding="utf-8")
        in_scope = [genre for genre in catalog if genre != "Experimental"]
        self.valid_document = {
            "schema_version": 1,
            "playlist_name": "genre_reference_candidates",
            "approved_training_playlist": "genre_verified",
            "approved_holdout_playlist": "genre_reference_holdout",
            "taxonomy_source": {
                "path": CATALOG_PATH.as_posix(),
                "commit": "08c98f95322a85776eb42b83489b1d8f0d4029d9",
            },
            "research_completed_on": "2026-07-18",
            "excluded_genres": [
                {
                    "genre": "Experimental",
                    "reason": "anti-genre/umbrella category; excluded by operator decision",
                }
            ],
            "genres": [build_genre(genre, family_for(genre)) for genre in in_scope],
        }

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def validate(self, document: dict, *, allow_incomplete: bool = False) -> list[str]:
        errors, _ = validate_document(document, self.repo_root, allow_incomplete)
        return errors

    def assert_error_contains(self, errors: list[str], text: str) -> None:
        self.assertTrue(any(text in error for error in errors), f"expected {text!r} in errors:\n" + "\n".join(errors))

    def test_exact_valid_minimal_fixture(self) -> None:
        errors, summary = validate_document(self.valid_document, self.repo_root)
        self.assertEqual([], errors)
        self.assertEqual({"expected_genres": 52, "populated_genres": 52, "candidates": 624}, summary)

    def test_incomplete_mode_allows_empty_genres_but_validates_populated_genres(self) -> None:
        document = copy.deepcopy(self.valid_document)
        for genre in document["genres"][1:]:
            genre["candidates"] = []
            genre["sources"] = []
            genre["definition_source_ids"] = []
            genre["working_definition"] = ""
        errors, summary = validate_document(document, self.repo_root, allow_incomplete=True)
        self.assertEqual([], errors)
        self.assertEqual(1, summary["populated_genres"])

        document["genres"][0]["candidates"] = document["genres"][0]["candidates"][:11]
        errors = self.validate(document, allow_incomplete=True)
        self.assert_error_contains(errors, "requires at least 12 candidates")

    def test_missing_and_extra_canonical_genre(self) -> None:
        document = copy.deepcopy(self.valid_document)
        document["genres"][0]["genre"] = "Not A Canonical Genre"
        errors = self.validate(document)
        self.assert_error_contains(errors, "missing canonical genres")
        self.assert_error_contains(errors, "unexpected or excluded genres")

    def test_experimental_candidate_leakage(self) -> None:
        document = copy.deepcopy(self.valid_document)
        document["genres"][0]["genre"] = "Experimental"
        errors = self.validate(document)
        self.assert_error_contains(errors, "Experimental must never contain candidates")

    def test_insufficient_and_imbalanced_candidate_roles(self) -> None:
        document = copy.deepcopy(self.valid_document)
        candidates = document["genres"][0]["candidates"]
        candidates[0]["reference_role"] = "representative"
        candidates[6]["recommended_pool_role"] = "training_anchor"
        errors = self.validate(document)
        self.assert_error_contains(errors, "at least 4 foundational")
        self.assert_error_contains(errors, "at least 4 holdout_candidate")

    def test_duplicate_track_version_normalization(self) -> None:
        document = copy.deepcopy(self.valid_document)
        first, second = document["genres"][0]["candidates"][:2]
        second["artist"] = f" {first['artist'].upper()}! "
        second["track_title"] = first["track_title"].replace(" ", "-")
        second["mix_version"] = "ORIGINAL-MIX"
        errors = self.validate(document)
        self.assert_error_contains(errors, "duplicate normalized artist/title/version identity")

    def test_insufficient_independent_canonicality_sources(self) -> None:
        document = copy.deepcopy(self.valid_document)
        candidate = document["genres"][0]["candidates"][0]
        candidate["canonicality_source_ids"] = ["definition-a"]
        errors = self.validate(document)
        self.assert_error_contains(errors, "at least two distinct source IDs")

        candidate["canonicality_source_ids"] = ["definition-a", "definition-c"]
        document["genres"][0]["sources"][2]["publisher"] = "Synthetic Archive A"
        errors = self.validate(document)
        self.assert_error_contains(errors, "two independent publishers and URLs")

    def test_missing_or_invalid_acquisition_url_and_date(self) -> None:
        document = copy.deepcopy(self.valid_document)
        acquisition = document["genres"][0]["candidates"][0]["acquisitions"][0]
        acquisition["url"] = "http://store.example/search?q=track"
        acquisition["accessed_on"] = "2026-02-31"
        errors = self.validate(document)
        self.assert_error_contains(errors, "direct HTTPS URL")
        self.assert_error_contains(errors, "must be an ISO date")

    def test_artist_and_label_concentration(self) -> None:
        document = copy.deepcopy(self.valid_document)
        candidates = document["genres"][0]["candidates"]
        for candidate in candidates:
            candidate["artist"] = "One Synthetic Artist"
            candidate["label"] = "One Synthetic Label"
        errors = self.validate(document)
        self.assert_error_contains(errors, "at least 8 distinct lead artists")
        self.assert_error_contains(errors, "artist/act exceeds two candidates")
        self.assert_error_contains(errors, "at least 4 distinct labels")
        self.assert_error_contains(errors, "label exceeds 25% concentration")

    def test_leakage_group_split_across_training_and_holdout(self) -> None:
        document = copy.deepcopy(self.valid_document)
        candidates = document["genres"][0]["candidates"]
        candidates[6]["leakage_group"] = candidates[0]["leakage_group"]
        errors = self.validate(document)
        self.assert_error_contains(errors, "split across training and holdout pools")

    def test_forbidden_private_field(self) -> None:
        document = copy.deepcopy(self.valid_document)
        document["genres"][0]["candidates"][0]["track_id"] = "private-id"
        errors = self.validate(document)
        self.assert_error_contains(errors, "forbidden private-data field")

    def test_repeated_source_url_does_not_count_twice(self) -> None:
        document = copy.deepcopy(self.valid_document)
        sources = document["genres"][0]["sources"]
        sources[1]["url"] = sources[0]["url"]
        errors = self.validate(document)
        self.assert_error_contains(errors, "repeated URLs")


if __name__ == "__main__":
    unittest.main()
