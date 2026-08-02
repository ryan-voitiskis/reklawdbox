#!/usr/bin/env python3
"""Extract a frozen set of Essentia MusiCNN mel patches for Discogs-EffNet.

This research helper intentionally requires the managed Essentia interpreter.
It reads a private manifest and writes a private NumPy matrix outside Git.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np
from essentia.standard import FrameGenerator, MonoLoader, TensorflowInputMusiCNN


FRAME_SIZE = 512
HOP_SIZE = 256
PATCH_FRAMES = 128
MEL_BANDS = 96
SAMPLE_RATE = 16_000


def patch_starts(frame_count: int, patch_count: int) -> np.ndarray:
    if frame_count < PATCH_FRAMES:
        return np.zeros(patch_count, dtype=np.int64)
    return np.rint(np.linspace(0, frame_count - PATCH_FRAMES, patch_count)).astype(np.int64)


def mel_frames(file_path: str) -> np.ndarray:
    audio = MonoLoader(filename=file_path, sampleRate=SAMPLE_RATE, resampleQuality=4)()
    if audio.size == 0:
        raise ValueError("decoded audio is empty")
    if audio.size < FRAME_SIZE:
        audio = np.pad(audio, (0, FRAME_SIZE - audio.size))
    frontend = TensorflowInputMusiCNN()
    frames = [
        frontend(frame)
        for frame in FrameGenerator(
            audio,
            frameSize=FRAME_SIZE,
            hopSize=HOP_SIZE,
            startFromZero=False,
        )
    ]
    if not frames:
        raise ValueError("mel frontend produced no frames")
    return np.asarray(frames, dtype=np.float32)


def extract(manifest_path: Path, output_path: Path) -> dict[str, object]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    rows = manifest["rows"]
    patch_count = int(manifest["patches_per_track"])
    if patch_count != 12:
        raise ValueError(f"expected frozen patches_per_track=12, found {patch_count}")

    output_path.parent.mkdir(parents=True, exist_ok=True)
    patches = np.lib.format.open_memmap(
        output_path,
        mode="w+",
        dtype=np.float32,
        shape=(len(rows), patch_count, PATCH_FRAMES, MEL_BANDS),
    )
    frame_counts: list[int] = []
    for index, row in enumerate(rows):
        frames = mel_frames(row["file_path"])
        frame_counts.append(int(frames.shape[0]))
        if frames.shape[0] < PATCH_FRAMES:
            frames = np.pad(frames, ((0, PATCH_FRAMES - frames.shape[0]), (0, 0)))
        for patch_index, start in enumerate(patch_starts(frames.shape[0], patch_count)):
            patches[index, patch_index] = frames[start : start + PATCH_FRAMES]
        if (index + 1) % 25 == 0 or index + 1 == len(rows):
            print(f"mel patches: {index + 1}/{len(rows)}", file=sys.stderr, flush=True)
    patches.flush()
    return {
        "rows": len(rows),
        "patches_per_track": patch_count,
        "shape": list(patches.shape),
        "frame_size": FRAME_SIZE,
        "hop_size": HOP_SIZE,
        "mel_bands": MEL_BANDS,
        "sample_rate": SAMPLE_RATE,
        "minimum_source_frames": min(frame_counts, default=0),
        "maximum_source_frames": max(frame_counts, default=0),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    summary = extract(args.manifest, args.output)
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
