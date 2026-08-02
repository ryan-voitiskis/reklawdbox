#!/usr/bin/env python3
"""Extract one frozen Plan 066 representation without reading truth labels."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import time
from pathlib import Path
from typing import Any, Protocol


EXPERIMENT_ID = "broad-genre-next-model-representations-v1"
SAMPLE_RATE = 48_000
OUTPUT_DIMENSION = 512
MODE_CONFIG = {
    "openl3": {
        "excerpt_count": 12,
        "excerpt_seconds": 1,
        "padding": "zero",
        "model_name": "openl3-music-mel128-emb512-3.onnx",
    },
    "clap": {
        "excerpt_count": 3,
        "excerpt_seconds": 10,
        "padding": "repeat",
        "model_name": "laion/clap-htsat-unfused",
        "model_revision": "8fa0f1c6d0433df6e97c127f64b2a1d6c0dcda8a",
    },
}


class ArrayLike(Protocol):
    shape: tuple[int, ...]
    dtype: Any

    def astype(self, dtype: Any, copy: bool = ...) -> ArrayLike: ...


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json_sha256(value: Any) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode("utf-8")).hexdigest()


def even_excerpt_starts(
    sample_count: int, excerpt_samples: int, excerpt_count: int
) -> list[int]:
    if sample_count < 0 or excerpt_samples <= 0 or excerpt_count <= 0:
        raise ValueError("sample and excerpt counts must be valid")
    maximum = max(0, sample_count - excerpt_samples)
    if excerpt_count == 1:
        return [maximum // 2]
    return [
        round(maximum * index / (excerpt_count - 1))
        for index in range(excerpt_count)
    ]


def pad_excerpt(values: ArrayLike, required: int, padding: str) -> ArrayLike:
    import numpy as np

    array = np.asarray(values, dtype=np.float32)
    if array.ndim != 1:
        raise ValueError("decoded audio must be one-dimensional")
    if len(array) >= required:
        return array[:required].copy()
    if padding == "zero":
        return np.pad(array, (0, required - len(array))).astype(np.float32)
    if padding == "repeat":
        if len(array) == 0:
            return np.zeros(required, dtype=np.float32)
        repeats = math.ceil(required / len(array))
        return np.tile(array, repeats)[:required].astype(np.float32, copy=False)
    raise ValueError(f"unknown padding mode {padding!r}")


def evenly_spaced_excerpts(
    values: ArrayLike,
    excerpt_samples: int,
    excerpt_count: int,
    padding: str,
) -> ArrayLike:
    import numpy as np

    array = np.asarray(values, dtype=np.float32)
    starts = even_excerpt_starts(len(array), excerpt_samples, excerpt_count)
    output = [
        pad_excerpt(array[start : start + excerpt_samples], excerpt_samples, padding)
        for start in starts
    ]
    return np.stack(output).astype(np.float32, copy=False)


def aggregate_patch_embeddings(values: ArrayLike) -> ArrayLike:
    import numpy as np

    embeddings = np.asarray(values, dtype=np.float64)
    if embeddings.ndim != 2 or embeddings.shape[1] != OUTPUT_DIMENSION:
        raise ValueError(
            f"patch embeddings must have shape (n, {OUTPUT_DIMENSION})"
        )
    if not np.all(np.isfinite(embeddings)):
        raise ValueError("patch embeddings contain non-finite values")
    norms = np.linalg.norm(embeddings, axis=1, keepdims=True)
    normalized = embeddings / np.maximum(norms, 1e-12)
    track = normalized.mean(axis=0)
    track /= max(float(np.linalg.norm(track)), 1e-12)
    return track.astype(np.float32)


def decode_audio(path: Path, ffmpeg: str) -> tuple[ArrayLike, bytes]:
    import numpy as np

    completed = subprocess.run(
        [
            ffmpeg,
            "-v",
            "error",
            "-nostdin",
            "-i",
            str(path),
            "-map",
            "a:0",
            "-ac",
            "1",
            "-ar",
            str(SAMPLE_RATE),
            "-f",
            "f32le",
            "-",
        ],
        capture_output=True,
        check=True,
    )
    audio = np.frombuffer(completed.stdout, dtype="<f4").astype(np.float32)
    if len(audio) == 0 or not np.all(np.isfinite(audio)):
        raise ValueError(f"decoded audio is empty or non-finite: {path}")
    return audio, completed.stdout


def source_sha256(path: str, pcm: bytes) -> str:
    digest = hashlib.sha256()
    encoded_path = path.encode("utf-8")
    digest.update(len(encoded_path).to_bytes(8, "little"))
    digest.update(encoded_path)
    digest.update(len(pcm).to_bytes(8, "little"))
    digest.update(pcm)
    return digest.hexdigest()


class OpenL3Extractor:
    def __init__(self, model_path: Path):
        import essentia.standard as es
        import onnxruntime as ort

        self.es = es
        self.window = es.Windowing(size=2048, normalized=False)
        self.spectrum = es.Spectrum(size=2048)
        self.mel_bands = es.MelBands(
            highFrequencyBound=SAMPLE_RATE / 2,
            inputSize=1025,
            log=False,
            lowFrequencyBound=0,
            normalize="unit_tri",
            numberBands=128,
            sampleRate=SAMPLE_RATE,
            type="magnitude",
            warpingFormula="slaneyMel",
            weighting="linear",
        )
        options = ort.SessionOptions()
        options.intra_op_num_threads = 1
        options.inter_op_num_threads = 1
        self.model = ort.InferenceSession(
            str(model_path),
            sess_options=options,
            providers=["CPUExecutionProvider"],
        )
        self.input_name = self.model.get_inputs()[0].name
        self.output_name = self.model.get_outputs()[0].name

    def mel_patch(self, excerpt: ArrayLike) -> ArrayLike:
        import numpy as np

        frames = self.es.FrameGenerator(
            excerpt,
            frameSize=2048,
            hopSize=242,
            validFrameThresholdRatio=0.5,
        )
        values = np.asarray(
            [self.mel_bands(self.spectrum(self.window(frame))) for frame in frames],
            dtype=np.float32,
        )
        values = 10.0 * np.log10(np.maximum(1e-10, values))
        values = np.maximum(values, values.max() - 80.0)
        values -= values.max()
        if values.shape != (199, 128):
            raise ValueError(f"OpenL3 mel patch shape {values.shape} != (199, 128)")
        return values

    def __call__(self, excerpts: ArrayLike) -> ArrayLike:
        import numpy as np

        patches = np.stack([self.mel_patch(excerpt) for excerpt in excerpts])
        batch = np.transpose(patches, (0, 2, 1))[:, :, :, None]
        output = np.asarray(
            self.model.run([self.output_name], {self.input_name: batch})[0]
        ).squeeze()
        if output.shape != (len(patches), OUTPUT_DIMENSION):
            raise ValueError(
                f"OpenL3 output shape {output.shape} != "
                f"({len(patches)}, {OUTPUT_DIMENSION})"
            )
        return output.astype(np.float32, copy=False)


class ClapExtractor:
    def __init__(self, model_path: Path, device: str):
        import torch
        from transformers import ClapFeatureExtractor, ClapModel

        if device == "auto":
            device = "mps" if torch.backends.mps.is_available() else "cpu"
        if device not in {"cpu", "mps"}:
            raise ValueError("CLAP device must be auto, cpu, or mps")
        torch.manual_seed(0)
        self.torch = torch
        self.device = device
        self.processor = ClapFeatureExtractor.from_pretrained(
            model_path, local_files_only=True
        )
        self.model = ClapModel.from_pretrained(model_path, local_files_only=True)
        self.model.eval().to(device)

    def __call__(self, excerpts: ArrayLike) -> ArrayLike:
        import numpy as np

        arrays = [np.asarray(excerpt, dtype=np.float32) for excerpt in excerpts]
        if any(len(excerpt) != SAMPLE_RATE * 10 for excerpt in arrays):
            raise ValueError("CLAP excerpts must be exact ten-second arrays")
        inputs = self.processor(
            arrays,
            sampling_rate=SAMPLE_RATE,
            return_tensors="pt",
        )
        inputs = {name: value.to(self.device) for name, value in inputs.items()}
        with self.torch.inference_mode():
            output = self.model.get_audio_features(**inputs)
        values = output.detach().cpu().numpy()
        if values.shape != (len(arrays), OUTPUT_DIMENSION):
            raise ValueError(
                f"CLAP output shape {values.shape} != "
                f"({len(arrays)}, {OUTPUT_DIMENSION})"
            )
        return values.astype(np.float32, copy=False)


def extractor_for(mode: str, model_path: Path, device: str) -> Any:
    if mode == "openl3":
        return OpenL3Extractor(model_path)
    if mode == "clap":
        return ClapExtractor(model_path, device)
    raise ValueError(f"unsupported mode {mode!r}")


def initialize_or_resume_work(
    work_dir: Path, config: dict[str, Any], rows: int
) -> tuple[ArrayLike, ArrayLike, ArrayLike]:
    import numpy as np

    work_dir.mkdir(parents=True, exist_ok=True)
    config_path = work_dir / "config.json"
    if config_path.exists():
        existing = json.loads(config_path.read_text(encoding="utf-8"))
        if existing != config:
            raise ValueError("work directory configuration differs from frozen run")
        embeddings = np.lib.format.open_memmap(
            work_dir / "embeddings.npy", mode="r+"
        )
        done = np.lib.format.open_memmap(work_dir / "done.npy", mode="r+")
        source_hashes = np.lib.format.open_memmap(
            work_dir / "source-sha256.npy", mode="r+"
        )
    else:
        config_path.write_text(
            json.dumps(config, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        embeddings = np.lib.format.open_memmap(
            work_dir / "embeddings.npy",
            mode="w+",
            dtype=np.float32,
            shape=(rows, OUTPUT_DIMENSION),
        )
        embeddings.fill(np.nan)
        done = np.lib.format.open_memmap(
            work_dir / "done.npy", mode="w+", dtype=np.bool_, shape=(rows,)
        )
        done.fill(False)
        source_hashes = np.lib.format.open_memmap(
            work_dir / "source-sha256.npy",
            mode="w+",
            dtype="S64",
            shape=(rows,),
        )
        source_hashes[:] = b""
        embeddings.flush()
        done.flush()
        source_hashes.flush()
    expected = {
        "embeddings": (rows, OUTPUT_DIMENSION),
        "done": (rows,),
        "source_hashes": (rows,),
    }
    observed = {
        "embeddings": embeddings.shape,
        "done": done.shape,
        "source_hashes": source_hashes.shape,
    }
    if observed != expected:
        raise ValueError(f"work artifact shapes {observed} != {expected}")
    return embeddings, done, source_hashes


def run(args: argparse.Namespace) -> dict[str, Any]:
    import numpy as np

    if args.mode not in MODE_CONFIG:
        raise ValueError(f"unsupported mode {args.mode!r}")
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    rows = manifest["rows"]
    mode_config = MODE_CONFIG[args.mode]
    model_sha = (
        sha256_file(args.model / "pytorch_model.bin")
        if args.mode == "clap"
        else sha256_file(args.model)
    )
    extractor_sha = sha256_file(Path(__file__))
    config = {
        "experiment_id": EXPERIMENT_ID,
        "mode": args.mode,
        "mode_config": mode_config,
        "manifest_sha256": sha256_file(args.manifest),
        "manifest_corpus_fingerprint": manifest["corpus_fingerprint"],
        "model_sha256": model_sha,
        "extractor_source_sha256": extractor_sha,
        "sample_rate": SAMPLE_RATE,
        "rows": len(rows),
        "output_dimension": OUTPUT_DIMENSION,
        "ffmpeg": args.ffmpeg,
        "device": args.device if args.mode == "clap" else "cpu",
    }
    embeddings, done, source_hashes = initialize_or_resume_work(
        args.work_dir, config, len(rows)
    )
    extractor = extractor_for(args.mode, args.model, args.device)
    excerpt_samples = SAMPLE_RATE * int(mode_config["excerpt_seconds"])
    initial_completed = int(np.sum(done))
    started = time.monotonic()
    for row_index, row in enumerate(rows):
        if bool(done[row_index]):
            continue
        path = Path(row["file_path"])
        if not path.is_file():
            raise ValueError(f"manifest audio file is missing: {path}")
        audio, pcm = decode_audio(path, args.ffmpeg)
        excerpts = evenly_spaced_excerpts(
            audio,
            excerpt_samples,
            int(mode_config["excerpt_count"]),
            str(mode_config["padding"]),
        )
        patch_embeddings = extractor(excerpts)
        embeddings[row_index] = aggregate_patch_embeddings(patch_embeddings)
        source_hashes[row_index] = source_sha256(str(path), pcm).encode("ascii")
        embeddings.flush()
        source_hashes.flush()
        done[row_index] = True
        done.flush()
        completed = int(np.sum(done))
        if completed % 10 == 0 or completed == len(rows):
            elapsed = max(time.monotonic() - started, 1e-9)
            print(
                json.dumps(
                    {
                        "mode": args.mode,
                        "completed": completed,
                        "rows": len(rows),
                        "new_rows_per_second": (
                            (completed - initial_completed) / elapsed
                        ),
                    },
                    sort_keys=True,
                ),
                flush=True,
            )

    if not np.all(done):
        raise ValueError("representation extraction ended with unfinished rows")
    values = np.asarray(embeddings, dtype=np.float32)
    if values.shape != (len(rows), OUTPUT_DIMENSION) or not np.all(
        np.isfinite(values)
    ):
        raise ValueError("final embedding artifact is malformed")
    norms = np.linalg.norm(values, axis=1)
    if not np.allclose(norms, 1.0, rtol=1e-5, atol=1e-5):
        raise ValueError("final track embeddings are not L2-normalized")
    ordered_source = [value.decode("ascii") for value in source_hashes]
    if any(len(value) != 64 for value in ordered_source):
        raise ValueError("one or more decoded source hashes are missing")
    np.savez_compressed(args.output, embeddings=values)
    result = {
        **config,
        "method_status": "frozen_label_blind_representation_extraction",
        "ordered_source_sha256": canonical_json_sha256(ordered_source),
        "feature_artifact_sha256": sha256_file(args.output),
        "feature_artifact_bytes": args.output.stat().st_size,
        "dependencies": {
            "numpy": np.__version__,
        },
    }
    if args.mode == "openl3":
        import essentia
        import onnxruntime as ort

        result["dependencies"]["essentia"] = essentia.__version__
        result["dependencies"]["onnxruntime"] = ort.__version__
    else:
        import torch
        import transformers

        result["dependencies"]["torch"] = torch.__version__
        result["dependencies"]["transformers"] = transformers.__version__
        result["resolved_device"] = extractor.device
    args.summary.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    os.chmod(args.output, 0o600)
    os.chmod(args.summary, 0o600)
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", required=True, choices=sorted(MODE_CONFIG))
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("--device", default="auto")
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    result = run(args)
    print(
        json.dumps(
            {
                "mode": result["mode"],
                "rows": result["rows"],
                "model_sha256": result["model_sha256"],
                "extractor_source_sha256": result["extractor_source_sha256"],
                "ordered_source_sha256": result["ordered_source_sha256"],
                "feature_artifact_sha256": result["feature_artifact_sha256"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
