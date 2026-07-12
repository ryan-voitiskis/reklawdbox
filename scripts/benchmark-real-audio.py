#!/usr/bin/env python3
"""Run and compare the versioned private real-audio DSP benchmark."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "stratum-dsp/benchmarks/real-audio-v1/manifest.json"


def load(path: Path) -> dict:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def write_atomic(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as output:
        json.dump(value, output, indent=2)
        output.write("\n")
        temporary = Path(output.name)
    temporary.replace(path)


def metric(result: dict, dotted: str):
    value = result
    for part in dotted.split("."):
        if value is None:
            return None
        value = value.get(part)
    return value


def validate_result(result: dict) -> list[str]:
    failures = []
    if result.get("grid_source") != "rekordbox_pqtz":
        failures.append("grid source is not rekordbox_pqtz")
    analysis = result.get("analysis", {})
    if not analysis.get("dub_stab"):
        failures.append("dub_stab analysis is absent")
    for name in ("bpm", "bpm_confidence", "grid_stability"):
        value = analysis.get(name)
        if not isinstance(value, (int, float)):
            failures.append(f"analysis.{name} is not numeric")
    return failures


def run(args: argparse.Namespace) -> int:
    manifest = load(args.manifest)
    audio_root_value = args.audio_root or os.environ.get(manifest["audio_root_env"])
    grid_json_value = args.grid_json or os.environ.get(manifest["grid_json_env"])
    if not audio_root_value:
        raise SystemExit(f"set {manifest['audio_root_env']} or pass --audio-root")
    if not grid_json_value:
        raise SystemExit(f"set {manifest['grid_json_env']} or pass --grid-json")
    audio_root = Path(audio_root_value)
    grid_json = Path(grid_json_value)
    if not grid_json.is_file():
        raise SystemExit(f"grid JSON does not exist: {grid_json}")

    binary = args.binary or ROOT / "target/release/examples/real_audio_benchmark"
    if not args.no_build:
        subprocess.run(
            ["cargo", "build", "--release", "-p", "stratum-dsp", "--example", "real_audio_benchmark"],
            cwd=ROOT,
            check=True,
        )

    results = []
    failures = []
    for index, track in enumerate(manifest["tracks"], 1):
        audio_path = audio_root / track["relative_path"]
        print(f"[{index:02}/{len(manifest['tracks'])}] {track['label']}", file=sys.stderr)
        if not audio_path.is_file():
            failures.append(f"{track['track_id']}: missing {audio_path}")
            continue
        completed = subprocess.run(
            [str(binary), str(track["track_id"]), str(audio_path), track["relative_path"], str(grid_json), str(manifest["analysis_window_seconds"])],
            cwd=ROOT,
            text=True,
            capture_output=True,
        )
        if completed.returncode:
            failures.append(f"{track['track_id']}: {completed.stderr.strip()}")
            continue
        try:
            result = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            failures.append(f"{track['track_id']}: invalid runner JSON: {error}")
            continue
        failures.extend(f"{track['track_id']}: {failure}" for failure in validate_result(result))
        results.append(result)

    output = {
        "schema_version": 1,
        "corpus_id": manifest["corpus_id"],
        "corpus_version": manifest["corpus_version"],
        "analysis_window_seconds": manifest["analysis_window_seconds"],
        "results": results,
    }
    write_atomic(args.output, output)
    if failures:
        print("benchmark run failed:", file=sys.stderr)
        print("\n".join(f"- {failure}" for failure in failures), file=sys.stderr)
        return 1
    print(f"wrote {len(results)} results to {args.output}")
    return 0


def compare(args: argparse.Namespace) -> int:
    manifest = load(args.manifest)
    baseline = load(args.baseline)
    candidate = load(args.candidate)
    failures = []
    for field in ("schema_version", "corpus_id", "corpus_version", "analysis_window_seconds"):
        if baseline.get(field) != candidate.get(field):
            failures.append(f"{field}: baseline={baseline.get(field)!r}, candidate={candidate.get(field)!r}")

    old = {row["track_id"]: row for row in baseline["results"]}
    new = {row["track_id"]: row for row in candidate["results"]}
    if old.keys() != new.keys():
        failures.append("track IDs differ between baseline and candidate")
    tolerances = manifest["comparison_tolerances"]
    print("track  bpm delta  stab-rate delta  template-score delta")
    for track_id in sorted(old.keys() & new.keys()):
        left, right = old[track_id], new[track_id]
        for fingerprint in ("audio_sha256", "grid_sha256"):
            if left[fingerprint] != right[fingerprint]:
                failures.append(f"{track_id}: {fingerprint} changed")
        for path in manifest["exact_match_fields"]:
            if metric(left, path) != metric(right, path):
                failures.append(f"{track_id}: {path} changed")
        deltas = []
        for path, tolerance_name in (
            ("analysis.bpm", "bpm_absolute"),
            ("analysis.dub_stab.stab_onset_rate", "dub_stab_rate_absolute"),
            ("analysis.dub_stab.template_match.score", "template_score_absolute"),
        ):
            before, after = metric(left, path), metric(right, path)
            delta = None if before is None or after is None else abs(after - before)
            deltas.append(delta)
            if (before is None) != (after is None):
                failures.append(f"{track_id}: {path} presence changed")
            elif delta is not None and delta > tolerances[tolerance_name]:
                failures.append(f"{track_id}: {path} delta {delta:.4f} exceeds {tolerances[tolerance_name]:.4f}")
        shown = ["n/a" if value is None else f"{value:.4f}" for value in deltas]
        print(f"{track_id:<9} {shown[0]:>9} {shown[1]:>16} {shown[2]:>20}")
        before_peak = metric(left, "analysis.dub_stab.histogram_peak_bin")
        after_peak = metric(right, "analysis.dub_stab.histogram_peak_bin")
        if (before_peak is None) != (after_peak is None):
            failures.append(f"{track_id}: histogram peak presence changed")
        elif before_peak is not None:
            distance = abs(after_peak - before_peak)
            circular_distance = min(distance, 32 - distance)
            if circular_distance > tolerances["histogram_peak_bins"]:
                failures.append(
                    f"{track_id}: histogram peak moved {circular_distance} bins"
                )

    if failures:
        print("comparison failed:", file=sys.stderr)
        print("\n".join(f"- {failure}" for failure in failures), file=sys.stderr)
        return 1
    print("comparison passed")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    commands = root.add_subparsers(dest="command", required=True)
    run_parser = commands.add_parser("run")
    run_parser.add_argument("--audio-root")
    run_parser.add_argument("--grid-json")
    run_parser.add_argument("--output", type=Path, required=True)
    run_parser.add_argument("--binary", type=Path)
    run_parser.add_argument("--no-build", action="store_true")
    run_parser.set_defaults(function=run)
    compare_parser = commands.add_parser("compare")
    compare_parser.add_argument("--baseline", type=Path, required=True)
    compare_parser.add_argument("--candidate", type=Path, required=True)
    compare_parser.set_defaults(function=compare)
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    raise SystemExit(arguments.function(arguments))
