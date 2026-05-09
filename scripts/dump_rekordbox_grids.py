#!/usr/bin/env python3
"""Dump Rekordbox per-track beat grids to JSON for the dub_stab example.

Reads master.db (via pyrekordbox) to find each track's AnalysisDataPath,
then parses the PQTZ tag in the ANLZ .DAT file. Emits one JSON object per
track keyed by file path:

    {"<file_path>": {"beats": [t0, t1, ...], "bars": [t0, t4, ...]}, ...}

Times are in seconds. `bars` is the subset of beats where bit==1 (downbeats).
"""

import json
import os
import sys

from pyrekordbox import Rekordbox6Database
from pyrekordbox.anlz import AnlzFile

ANLZ_ROOT = os.path.expanduser("~/Library/Pioneer/rekordbox/share")


def grid_for_track(content) -> dict | None:
    rel = content.AnalysisDataPath
    if not rel:
        return None
    path = ANLZ_ROOT + rel
    if not os.path.exists(path):
        print(f"  missing ANLZ: {path}", file=sys.stderr)
        return None
    try:
        af = AnlzFile.parse_file(path)
    except Exception as e:
        # Don't abort the whole batch if one ANLZ is corrupt.
        print(f"  failed to parse {path}: {e}", file=sys.stderr)
        return None
    pqtz = af.getall("PQTZ")
    if not pqtz:
        return None
    bits, _bpms, times = pqtz[0]
    if len(times) == 0:
        # Empty PQTZ would produce a zero-bar grid that silently makes the
        # downstream histogram look on-beat — refuse instead.
        return None
    beats = [float(t) for t in times]
    bars = [float(t) for t, b in zip(times, bits) if int(b) == 1]
    return {"beats": beats, "bars": bars}


def main():
    track_ids = [int(x) for x in sys.argv[1:]] if len(sys.argv) > 1 else None
    db = Rekordbox6Database()
    out: dict[str, dict] = {}
    if track_ids is None:
        print("usage: dump_rekordbox_grids.py <track_id> [track_id...]", file=sys.stderr)
        sys.exit(1)
    for tid in track_ids:
        c = db.get_content(ID=tid)
        if c is None:
            print(f"  track {tid} not found", file=sys.stderr)
            continue
        grid = grid_for_track(c)
        if grid is None:
            print(f"  no grid for {tid} ({c.Title})", file=sys.stderr)
            continue
        out[c.FolderPath] = grid
        print(
            f"  {tid}: {c.Title!r:40} beats={len(grid['beats'])} bars={len(grid['bars'])}",
            file=sys.stderr,
        )
    json.dump(out, sys.stdout, indent=2)


if __name__ == "__main__":
    main()
