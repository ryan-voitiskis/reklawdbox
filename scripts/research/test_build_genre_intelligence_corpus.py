import unittest

import build_genre_intelligence_corpus as corpus
import ingest_genre_truth_batch as ingest


def base_row(index: int, truth: str, artist: str | None = None) -> tuple[dict, dict]:
    path = f"/music/{index}.flac"
    artist_name = artist or f"Artist {index}"
    return (
        {"file_path": path, "truth": truth},
        {
            "track_id": str(index),
            "file_path": path,
            "artist": artist_name,
            "title": f"Title {index}",
            "album": f"Album {index}",
        },
    )


def build(base_rows: list[dict], library_rows: list[dict], truth_rows=None) -> dict:
    return corpus.build_corpus(
        {"corpus_fingerprint": "test-base", "rows": base_rows},
        {"corpus_fingerprint": "test-truth", "rows": truth_rows or []},
        library_rows,
        base_manifest_sha256="base-sha",
        truth_snapshot_sha256="truth-sha",
        enforce_frozen_base=False,
    )


class BuildGenreIntelligenceCorpusTests(unittest.TestCase):
    def test_taxonomy_matches_frozen_broad_parent_semantics(self) -> None:
        self.assertEqual(
            corpus.taxonomy_semantic_sha256(),
            corpus.EXPECTED_TAXONOMY_SEMANTIC_SHA256,
        )
        self.assertEqual(set(corpus.PARENT_GENRES), set(ingest.PARENT_GENRES))

    def test_maps_fine_truth_and_passes_a_diverse_parent(self) -> None:
        pairs = [base_row(index, "Deep House") for index in range(1, 21)]
        result = build(
            [pair[0] for pair in pairs], [pair[1] for pair in pairs]
        )
        self.assertEqual(result["accepted_rows"], 20)
        self.assertEqual(result["release_scope"], ["House"])
        self.assertEqual(result["support"]["House"]["accepted"]["rows"], 20)
        self.assertTrue(result["support"]["House"]["supported"])

    def test_deterministically_caps_an_overrepresented_artist(self) -> None:
        pairs = []
        for index in range(1, 26):
            artist = "Dominant" if index <= 9 else f"Artist {index}"
            pairs.append(base_row(index, "Ambient", artist))
        first = build([pair[0] for pair in pairs], [pair[1] for pair in pairs])
        second = build([pair[0] for pair in pairs], [pair[1] for pair in pairs])
        balanced = first["support"]["Ambient"]["balanced"]
        self.assertEqual(balanced["rows"], 20)
        self.assertEqual(balanced["max_artist_rows"], 4)
        self.assertEqual(balanced["max_artist_share"], 0.20)
        self.assertTrue(first["support"]["Ambient"]["supported"])
        self.assertEqual(
            first["model_ready_corpus_fingerprint"],
            second["model_ready_corpus_fingerprint"],
        )

    def test_appends_only_high_or_medium_blind_snapshot_rows(self) -> None:
        base, live = base_row(1, "Minimal")
        blind = {
            "record_id": "review-1",
            "track_id": "2",
            "file_path": "/music/2.flac",
            "artist": "Blind Artist",
            "title": "Blind Title",
            "album": "Blind Album",
            "artist_group": "blind artist",
            "release_group": "blind artist\0blind album",
            "decoded_pcm_sha256": "decoded",
            "canonical_parent_genre": "Tech House",
            "confidence": "medium",
            "provenance": {"kind": "operator_blind_review"},
        }
        result = build([base], [live], [blind])
        self.assertEqual(result["accepted_rows"], 2)
        self.assertEqual(result["support"]["Tech House"]["accepted"]["rows"], 1)

        blind["confidence"] = "low"
        with self.assertRaisesRegex(ValueError, "non-eligible confidence"):
            build([base], [live], [blind])

    def test_duplicate_path_is_rejected(self) -> None:
        base, live = base_row(1, "Minimal")
        blind = {
            "record_id": "review-1",
            "track_id": "1",
            "file_path": base["file_path"],
            "artist": "Artist 1",
            "title": "Title 1",
            "album": "Album 1",
            "artist_group": "artist 1",
            "release_group": "artist 1\0album 1",
            "decoded_pcm_sha256": "decoded",
            "canonical_parent_genre": "Minimal",
            "confidence": "high",
            "provenance": {"kind": "operator_blind_review"},
        }
        with self.assertRaisesRegex(ValueError, "duplicate accepted path"):
            build([base], [live], [blind])


if __name__ == "__main__":
    unittest.main()
