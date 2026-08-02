from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parent / "research" / "select_broad_genre_holdout.py"
SPEC = importlib.util.spec_from_file_location("select_broad_genre_holdout", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
holdout = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(holdout)


class BroadGenreHoldoutTests(unittest.TestCase):
    def test_aliases_project_only_to_frozen_broad_targets(self) -> None:
        self.assertEqual(holdout.broad_target("Dub Reggae"), "Reggae")
        self.assertEqual(holdout.broad_target("Reggae Dub"), "Reggae")
        self.assertEqual(holdout.broad_target("Ambient techno"), "Techno")
        self.assertEqual(holdout.broad_target("Deep House"), "House")
        self.assertEqual(holdout.broad_target("Tech House"), "Tech House")
        self.assertIsNone(holdout.broad_target("Experimental"))
        self.assertIsNone(holdout.broad_target("Not A Genre"))

    def test_parse_json_documents_handles_sqlcipher_pragma_and_query(self) -> None:
        value = '[{"ok":"ok"}]\n[{"track_id":"1"}]\n'
        self.assertEqual(
            holdout.parse_json_documents(value),
            [[{"ok": "ok"}], [{"track_id": "1"}]],
        )

    def test_selector_is_deterministic_isolated_and_target_capped(self) -> None:
        rows = []
        targets = [
            "Ambient",
            "Breakbeat",
            "House",
            "Techno",
            "Tech House",
            "Minimal",
        ]
        for target_index, target in enumerate(targets):
            for row_index in range(8):
                artist = f"artist-{target_index}-{row_index}"
                rows.append(
                    {
                        "track_id": f"{target_index}-{row_index}",
                        "file_path": f"/audio/{target_index}-{row_index}.wav",
                        "artist": artist,
                        "album": f"release-{target_index}-{row_index}",
                        "title": f"title-{row_index}",
                        "current_genre": target,
                        "broad_sampling_stratum": target,
                        "artist_group": artist,
                        "release_group": f"{artist}\0release-{row_index}",
                        "stable_hash": holdout.stable_hash(
                            target, target_index, row_index
                        ),
                    }
                )

        first = holdout.select_holdout(rows, count=24)
        second = holdout.select_holdout(list(reversed(rows)), count=24)
        self.assertEqual(
            [row["track_id"] for row in first],
            [row["track_id"] for row in second],
        )
        self.assertEqual(len(first), 24)
        self.assertEqual(len({row["artist_group"] for row in first}), 24)
        self.assertEqual(len({row["release_group"] for row in first}), 24)
        counts = {}
        for row in first:
            target = row["broad_sampling_stratum"]
            counts[target] = counts.get(target, 0) + 1
        self.assertTrue(all(value <= holdout.MAX_PER_TARGET for value in counts.values()))
        self.assertEqual(set(counts), set(targets))

    def test_selector_rejects_impossible_roster(self) -> None:
        row = {
            "track_id": "1",
            "file_path": "/audio/1.wav",
            "artist": "artist",
            "album": "release",
            "title": "title",
            "current_genre": "Techno",
            "broad_sampling_stratum": "Techno",
            "artist_group": "artist",
            "release_group": "artist\0release",
            "stable_hash": "0",
        }
        with self.assertRaisesRegex(ValueError, "required 2"):
            holdout.select_holdout([row], count=2)


if __name__ == "__main__":
    unittest.main()
