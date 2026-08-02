from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np


MODULE_PATH = Path(__file__).with_name("genre_review_assistant.py")
SPEC = importlib.util.spec_from_file_location("genre_review_assistant", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(MODULE_PATH.parent))
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def reference(
    artist: str,
    title: str,
    genre: str,
    *,
    album: str = "Reference Album",
) -> dict[str, object]:
    return {
        "artist": artist,
        "title": title,
        "album": album,
        "truth": genre,
        "file_path": f"/{artist}-{title}.wav",
    }


def record(
    track_id: str,
    genre: str,
    support: int,
    affinity: float,
) -> dict[str, object]:
    return {
        "track_id": track_id,
        "file_path": f"/{track_id}.wav",
        "artist": f"Artist {track_id}",
        "title": f"Title {track_id}",
        "album": f"Album {track_id}",
        "current_genre": genre,
        "verified_genre_support": support,
        "current_reference_support": support,
        "current_affinity": affinity,
        "release_group": f"group-{track_id}",
        "stable_hash": MODULE.stable_hash(track_id),
        "hints": [{"genre": genre}],
    }


class GenreReviewAssistantTests(unittest.TestCase):
    def test_parse_reference_xml_decodes_local_file_locations(self) -> None:
        xml = """<?xml version="1.0"?>
<DJ_PLAYLISTS><COLLECTION Entries="1">
<TRACK Name="A &amp; B" Artist="Test Artist" Album="Release" Genre="House"
Location="file://localhost/Users/test/Music/Test%20Artist%20-%20A%20%26%20B.wav"/>
</COLLECTION></DJ_PLAYLISTS>
"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "references.xml"
            path.write_text(xml, encoding="utf-8")
            references = MODULE.parse_reference_xml(path)
        key = "/Users/test/Music/Test Artist - A & B.wav"
        self.assertEqual(references[key]["title"], "A & B")
        self.assertEqual(references[key]["genre"], "House")

    def test_parse_reference_xml_resolves_compatibility_aliases(self) -> None:
        xml = """<?xml version="1.0"?>
<DJ_PLAYLISTS><COLLECTION Entries="1">
<TRACK Name="Dub" Artist="Test Artist" Album="Release" Genre="Dub Reggae"
Location="file://localhost/Users/test/Music/Dub.wav"/>
</COLLECTION></DJ_PLAYLISTS>
"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "references.xml"
            path.write_text(xml, encoding="utf-8")
            references = MODULE.parse_reference_xml(path)
        self.assertEqual(references["/Users/test/Music/Dub.wav"]["genre"], "Dub")

    def test_normalized_embeddings_reject_zero_length_rows(self) -> None:
        with self.assertRaisesRegex(ValueError, "zero-length"):
            MODULE.normalized_embeddings(np.asarray([[0.0, 0.0]]), "test")

    def test_genre_affinities_exclude_same_artist_and_bound_references(self) -> None:
        candidate = {
            "artist": "Same Artist, Shared Collaborator",
            "album": "Candidate",
            "title": "Candidate",
        }
        references = [
            reference("Same Artist", "Leak", "House"),
            reference("Other Artist; Shared Collaborator", "Leak Two", "House"),
            reference("House A", "One", "House"),
            reference("House B", "Two", "House"),
            reference("House C", "Three", "House"),
            reference("Techno A", "One", "Techno"),
            reference("Techno B", "Two", "Techno"),
            reference("Techno C", "Three", "Techno"),
        ]
        embeddings = MODULE.normalized_embeddings(
            np.asarray(
                [
                    [1.0, 0.0],
                    [1.0, 0.0],
                    [0.99, 0.01],
                    [0.98, 0.02],
                    [0.97, 0.03],
                    [0.2, 0.8],
                    [0.1, 0.9],
                    [0.0, 1.0],
                ]
            ),
            "references",
        )
        affinities = MODULE.genre_affinities(
            candidate,
            np.asarray([1.0, 0.0]),
            references,
            embeddings,
        )
        self.assertEqual(set(affinities), {"House", "Techno"})
        self.assertEqual(len(affinities["House"]["references"]), 2)
        self.assertNotIn(
            "Same Artist",
            [item["artist"] for item in affinities["House"]["references"]],
        )
        self.assertNotIn(
            "Other Artist; Shared Collaborator",
            [item["artist"] for item in affinities["House"]["references"]],
        )

    def test_distinct_reference_selection_avoids_repeated_collaborators(self) -> None:
        references = [
            reference("Artist A; Shared", "One", "House"),
            reference("Artist B, Shared", "Two", "House"),
            reference("Artist C", "Three", "House"),
        ]
        selected = MODULE.distinct_reference_indices(
            np.asarray([0.9, 0.8, 0.7]),
            [0, 1, 2],
            references,
            2,
        )
        self.assertEqual(selected, [0, 2])

    def test_build_hints_keeps_current_first_and_two_alternatives(self) -> None:
        affinities = {
            genre: {
                "genre": genre,
                "affinity": affinity,
                "reference_support": 3,
                "references": [],
            }
            for genre, affinity in [
                ("House", 0.5),
                ("Techno", 0.9),
                ("Minimal", 0.8),
                ("Dub", 0.7),
            ]
        }
        hints = MODULE.build_hints("House", affinities)
        self.assertEqual(
            [hint["genre"] for hint in hints], ["House", "Techno", "Minimal"]
        )
        self.assertEqual(hints[0]["role"], "current_genre_context")
        self.assertTrue(
            all(hint["role"] == "alternative_listening_hint" for hint in hints[1:])
        )

    def test_vocabulary_uses_relative_quantiles_without_instrument_claims(self) -> None:
        development = np.asarray(
            [
                [1.0, 10.0, 100.0, 1000.0],
                [2.0, 20.0, 200.0, 2000.0],
                [3.0, 30.0, 300.0, 3000.0],
                [4.0, 40.0, 400.0, 4000.0],
            ]
        )
        cues = MODULE.vocabulary_cues(
            125.0,
            np.asarray([1.0, 25.0, 400.0, 2500.0]),
            development,
        )
        descriptions = [cue["description"] for cue in cues]
        self.assertIn("even loudness contour", descriptions)
        self.assertIn("frequent spectral change", descriptions)
        self.assertFalse(any("kick" in value or "swing" in value for value in descriptions))

    def test_selection_is_deterministic_and_enforces_family_diversity(self) -> None:
        records = [
            record("house", "House", 4, 0.8),
            record("deep-house", "Deep House", 5, 0.9),
            record("tech-house", "Tech House", 6, 0.95),
            record("techno", "Techno", 7, 0.7),
            record("dub", "Dub", 8, 0.6),
        ]
        first = MODULE.select_batch(records, batch_size=3)
        second = MODULE.select_batch(list(reversed(records)), batch_size=3)
        self.assertEqual(
            [item["track_id"] for item in first],
            [item["track_id"] for item in second],
        )
        families = [MODULE.base.family(item["current_genre"]) for item in first]
        self.assertLessEqual(max(families.count(value) for value in set(families)), 2)
        self.assertEqual(len({item["current_genre"] for item in first}), 3)

    def test_selection_fails_without_relaxing_quota(self) -> None:
        with self.assertRaisesRegex(ValueError, "required 2"):
            MODULE.select_batch([record("house", "House", 4, 0.8)], batch_size=2)


if __name__ == "__main__":
    unittest.main()
