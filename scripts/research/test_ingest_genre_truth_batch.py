import json
import tempfile
import unittest
from pathlib import Path

import ingest_genre_truth_batch as ingest


def mapping() -> dict:
    return {
        "experiment_id": "test-batch",
        "roster_sha256": "roster-sha",
        "selected": [
            {
                "position": 1,
                "code": "T-01",
                "track_id": "1",
                "file_path": "/music/one.flac",
                "artist": "Artist One",
                "title": "One",
                "album": "Release One",
                "artist_group": "artist one",
                "release_group": "artist one\0release one",
            },
            {
                "position": 2,
                "code": "T-02",
                "track_id": "2",
                "file_path": "/music/two.flac",
                "artist": "Artist Two",
                "title": "Two",
                "album": "Release Two",
                "artist_group": "artist two",
                "release_group": "artist two\0release two",
            },
        ],
    }


def verdicts() -> dict:
    return {
        "batch_id": "test-batch",
        "reviewer": "operator",
        "reviewed_at": "2026-08-03T00:00:00Z",
        "rows": [
            {
                "code": "T-01",
                "outcome": "label",
                "genre": "Breakbeat",
                "confidence": "medium",
                "confidence_raw": "medium to high",
                "alternatives": [],
                "notes": "broken rhythm",
            },
            {
                "code": "T-02",
                "outcome": "skip",
                "genre": None,
                "confidence": None,
                "alternatives": ["Ambient"],
                "notes": "not useful",
            },
        ],
    }


def fake_identity(path: Path, _ffmpeg: str) -> dict:
    name = path.name
    return {
        "file_sha256": f"file-{name}",
        "file_size": 100,
        "file_mtime_ns": 200,
        "decoded_pcm_sha256": f"pcm-{name}",
        "decoded_pcm_bytes": 400,
        "decoded_pcm_format": "f32le_mono_48000hz",
    }


class IngestGenreTruthBatchTests(unittest.TestCase):
    def test_twenty_row_batch_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            batch_mapping = mapping()
            batch_mapping["selected"] = []
            batch_verdicts = verdicts()
            batch_verdicts["rows"] = []
            for index in range(20):
                code = f"T-{index + 1:02d}"
                batch_mapping["selected"].append(
                    {
                        "position": index + 1,
                        "code": code,
                        "track_id": str(index + 1),
                        "file_path": f"/music/{index + 1}.flac",
                        "artist": f"Artist {index + 1}",
                        "title": f"Title {index + 1}",
                        "album": f"Release {index + 1}",
                        "artist_group": f"artist {index + 1}",
                        "release_group": f"artist {index + 1}\0release {index + 1}",
                    }
                )
                batch_verdicts["rows"].append(
                    {
                        "code": code,
                        "outcome": "label",
                        "genre": "House",
                        "confidence": "high",
                        "alternatives": [],
                        "notes": "",
                    }
                )
            result = ingest.ingest(
                batch_mapping,
                batch_verdicts,
                root / "ledger.jsonl",
                root / "snapshot.json",
                "ffmpeg",
                fake_identity,
            )
            self.assertEqual(result["records_added"], 20)

    def test_ingest_is_append_only_idempotent_and_builds_snapshot(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ledger = root / "ledger.jsonl"
            snapshot = root / "snapshot.json"
            first = ingest.ingest(
                mapping(), verdicts(), ledger, snapshot, "ffmpeg", fake_identity
            )
            first_ledger = ledger.read_bytes()
            second = ingest.ingest(
                mapping(), verdicts(), ledger, snapshot, "ffmpeg", fake_identity
            )
            self.assertEqual(first["records_added"], 2)
            self.assertEqual(second["records_added"], 0)
            self.assertEqual(ledger.read_bytes(), first_ledger)
            result = json.loads(snapshot.read_text())
            self.assertEqual(result["active_review_records"], 2)
            self.assertEqual(result["model_eligible_rows"], 1)
            self.assertEqual(result["genre_counts"], {"Breakbeat": 1})
            self.assertEqual(result["outcome_counts"], {"label": 1, "skip": 1})
            self.assertEqual(result["rows"][0]["confidence"], "medium")

    def test_changed_verdict_requires_explicit_supersession(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ledger = root / "ledger.jsonl"
            snapshot = root / "snapshot.json"
            original = verdicts()
            ingest.ingest(
                mapping(), original, ledger, snapshot, "ffmpeg", fake_identity
            )
            changed = verdicts()
            changed["rows"][0]["genre"] = "House"
            with self.assertRaisesRegex(ValueError, "must explicitly supersede"):
                ingest.ingest(
                    mapping(), changed, ledger, snapshot, "ffmpeg", fake_identity
                )

    def test_low_confidence_label_is_retained_but_not_model_eligible(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            values = verdicts()
            values["rows"] = [values["rows"][0]]
            values["rows"][0]["confidence"] = "low"
            one_mapping = mapping()
            one_mapping["selected"] = [one_mapping["selected"][0]]
            result = ingest.ingest(
                one_mapping,
                values,
                root / "ledger.jsonl",
                root / "snapshot.json",
                "ffmpeg",
                fake_identity,
            )
            self.assertEqual(result["model_eligible_rows"], 0)

    def test_noncanonical_label_is_rejected(self) -> None:
        values = verdicts()["rows"][0]
        values["genre"] = "Trace"
        with self.assertRaisesRegex(ValueError, "canonical parent genre"):
            ingest.validate_verdict(values)

    def test_duplicate_decoded_audio_requires_explicit_supersession(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def duplicate_identity(_path: Path, _ffmpeg: str) -> dict:
                value = fake_identity(Path("same.flac"), _ffmpeg)
                value["decoded_pcm_sha256"] = "same-decoded-audio"
                return value

            with self.assertRaisesRegex(ValueError, "already has active record"):
                ingest.ingest(
                    mapping(),
                    verdicts(),
                    root / "ledger.jsonl",
                    root / "snapshot.json",
                    "ffmpeg",
                    duplicate_identity,
                )


if __name__ == "__main__":
    unittest.main()
