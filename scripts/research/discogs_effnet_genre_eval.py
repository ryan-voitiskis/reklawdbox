#!/usr/bin/env python3
"""Evaluate frozen Discogs-EffNet style and fusion representations offline."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import numpy as np
import onnxruntime as ort


CANONICAL = [
    "2-Step Garage",
    "Acid",
    "Afro House",
    "Ambient",
    "Ambient Techno",
    "Bassline",
    "Breakbeat",
    "Broken Beat",
    "Dancehall",
    "Deep House",
    "Deep Techno",
    "Disco",
    "Downtempo",
    "Drum & Bass",
    "Dub",
    "Dub Techno",
    "Dubstep",
    "EBM",
    "Electro",
    "Experimental",
    "Footwork",
    "Future Garage",
    "Gabber",
    "Garage",
    "Gospel House",
    "Grime",
    "Happy Hardcore",
    "Hard Techno",
    "Hard Trance",
    "Hardcore",
    "Hardstyle",
    "Highlife",
    "Hip Hop",
    "House",
    "IDM",
    "Italo Disco",
    "Italodance",
    "Jazz",
    "Jungle",
    "Minimal",
    "Pop",
    "Progressive House",
    "Psytrance",
    "R&B",
    "Reggae",
    "Rock",
    "Speed Garage",
    "Synth-pop",
    "Tech House",
    "Techno",
    "Trance",
    "Trip-Hop",
    "UK Funky",
]
CANONICAL_INDEX = {genre: index for index, genre in enumerate(CANONICAL)}
MODELED_CANONICAL = [genre for genre in CANONICAL if genre != "Experimental"]

HOUSE = {
    "2-Step Garage",
    "Afro House",
    "Deep House",
    "Disco",
    "Garage",
    "Gospel House",
    "House",
    "Italo Disco",
    "Italodance",
    "Progressive House",
    "Speed Garage",
    "Tech House",
    "UK Funky",
}
TECHNO = {
    "Acid",
    "Ambient Techno",
    "Deep Techno",
    "Dub Techno",
    "EBM",
    "Electro",
    "Hard Techno",
    "Minimal",
    "Psytrance",
    "Techno",
    "Trance",
}
HARDCORE = {"Gabber", "Happy Hardcore", "Hard Trance", "Hardcore", "Hardstyle"}
BASS = {
    "Bassline",
    "Breakbeat",
    "Broken Beat",
    "Drum & Bass",
    "Dubstep",
    "Footwork",
    "Future Garage",
    "Grime",
    "Jungle",
}
DOWNTEMPO = {"Ambient", "Downtempo", "Dub", "Experimental", "IDM", "Trip-Hop"}

STYLE_ALIASES = {
    "Drum n Bass": "Drum & Bass",
    "Garage House": "House",
    "Italo-Disco": "Italo Disco",
    "Juke": "Footwork",
    "Minimal Techno": "Minimal",
    "Nu-Disco": "Disco",
    "Progressive Trance": "Trance",
    "Psy-Trance": "Psytrance",
    "Tech Trance": "Trance",
    "Trip Hop": "Trip-Hop",
    "UK Garage": "Garage",
}


@dataclass
class GenreMetrics:
    support: int
    predicted: int
    exact: int
    recall: float
    precision: float
    f1: float
    leading_confusions: list[dict[str, Any]]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def family(genre: str) -> str:
    if genre in HOUSE:
        return "House"
    if genre in TECHNO:
        return "Techno"
    if genre in HARDCORE:
        return "Hardcore"
    if genre in BASS:
        return "Bass"
    if genre in DOWNTEMPO:
        return "Downtempo"
    return "Other"


def safe_fraction(numerator: int, denominator: int) -> float:
    return numerator / denominator if denominator else 0.0


def safe_f1(precision: float, recall: float) -> float:
    return 2 * precision * recall / (precision + recall) if precision + recall else 0.0


def class_to_canonical(label: str) -> str | None:
    if "---" not in label:
        return None
    category, style = label.split("---", 1)
    if category == "Electronic":
        target = STYLE_ALIASES.get(style, style)
        return target if target in MODELED_CANONICAL else None
    if category == "Hip Hop":
        target = STYLE_ALIASES.get(style, style)
        return target if target in MODELED_CANONICAL else "Hip Hop"
    if category == "Jazz":
        return "Jazz"
    if category == "Pop":
        return "Pop"
    if category == "Reggae":
        target = STYLE_ALIASES.get(style, style)
        return target if target in {"Dancehall", "Dub", "Reggae"} else "Reggae"
    if category == "Rock":
        return "Rock"
    if category == "Funk / Soul":
        if style == "Disco":
            return "Disco"
        if style in {"Contemporary R&B", "Rhythm & Blues"}:
            return "R&B"
        return None
    if category == "Folk, World, & Country" and style == "Highlife":
        return "Highlife"
    return None


def canonical_style_scores(style_probabilities: np.ndarray, classes: list[str]) -> np.ndarray:
    scores = np.zeros((style_probabilities.shape[0], len(CANONICAL)), dtype=np.float32)
    for class_index, label in enumerate(classes):
        genre = class_to_canonical(label)
        if genre is None:
            continue
        genre_index = CANONICAL_INDEX[genre]
        scores[:, genre_index] = np.maximum(
            scores[:, genre_index], style_probabilities[:, class_index]
        )
    scores[:, CANONICAL_INDEX["Experimental"]] = 0.0
    return scores


def l2_normalize(values: np.ndarray, axis: int = -1) -> np.ndarray:
    norms = np.linalg.norm(values, axis=axis, keepdims=True)
    return values / np.maximum(norms, 1e-12)


def infer_features(
    model_path: Path,
    mel_path: Path,
    row_count: int,
    patches_per_track: int,
    intra_op_threads: int,
) -> tuple[np.ndarray, np.ndarray]:
    options = ort.SessionOptions()
    options.intra_op_num_threads = intra_op_threads
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(
        str(model_path), sess_options=options, providers=["CPUExecutionProvider"]
    )
    model_input = session.get_inputs()[0]
    outputs = session.get_outputs()
    style_output = next(output for output in outputs if output.shape[-1] == 400)
    embedding_output = next(output for output in outputs if output.shape[-1] == 1280)
    mels = np.load(mel_path, mmap_mode="r")
    expected_shape = (row_count, patches_per_track, 128, 96)
    if mels.shape != expected_shape:
        raise ValueError(f"mel matrix shape {mels.shape} != {expected_shape}")

    flat = mels.reshape((-1, 128, 96))
    style_batches: list[np.ndarray] = []
    embedding_batches: list[np.ndarray] = []
    for start in range(0, flat.shape[0], 64):
        batch = np.asarray(flat[start : start + 64], dtype=np.float32)
        style, embedding = session.run(
            [style_output.name, embedding_output.name], {model_input.name: batch}
        )
        style_batches.append(style)
        embedding_batches.append(embedding)
    patch_styles = np.concatenate(style_batches).reshape((row_count, patches_per_track, 400))
    patch_embeddings = np.concatenate(embedding_batches).reshape(
        (row_count, patches_per_track, 1280)
    )
    track_styles = patch_styles.mean(axis=1)
    track_embeddings = l2_normalize(l2_normalize(patch_embeddings, axis=2).mean(axis=1))
    return track_styles.astype(np.float32), track_embeddings.astype(np.float32)


def predictions_from_style(scores: np.ndarray) -> list[str]:
    modeled_indices = [CANONICAL_INDEX[genre] for genre in MODELED_CANONICAL]
    selected = np.argmax(scores[:, modeled_indices], axis=1)
    return [MODELED_CANONICAL[index] for index in selected]


def fit_fold_centroids(
    embeddings: np.ndarray,
    arrangement: np.ndarray,
    truths: list[str],
    folds: np.ndarray,
    held_out_fold: int,
) -> tuple[dict[str, np.ndarray], dict[str, np.ndarray], np.ndarray, np.ndarray, np.ndarray]:
    train = folds != held_out_fold
    train_arrangement = arrangement[train]
    means = np.nanmean(train_arrangement, axis=0)
    means = np.where(np.isfinite(means), means, 0.0)
    filled = np.where(np.isfinite(arrangement), arrangement, means)
    stddev = np.nanstd(train_arrangement, axis=0)
    active = np.isfinite(stddev) & (stddev > 1e-9)
    if not np.any(active):
        active = np.ones(arrangement.shape[1], dtype=bool)
        stddev = np.ones(arrangement.shape[1], dtype=np.float64)
    arrangement_z = (filled[:, active] - means[active]) / stddev[active]

    embedding_centroids: dict[str, np.ndarray] = {}
    arrangement_centroids: dict[str, np.ndarray] = {}
    for genre in sorted(set(truth for index, truth in enumerate(truths) if train[index])):
        indices = np.asarray([train[index] and truth == genre for index, truth in enumerate(truths)])
        embedding_centroids[genre] = l2_normalize(embeddings[indices].mean(axis=0))
        arrangement_centroids[genre] = arrangement_z[indices].mean(axis=0)
    return embedding_centroids, arrangement_centroids, arrangement_z, means, stddev


def predictions_from_fusion(
    style_scores: np.ndarray,
    embeddings: np.ndarray,
    arrangement: np.ndarray,
    truths: list[str],
    folds: np.ndarray,
) -> list[str]:
    predictions = [""] * len(truths)
    style_supported = {
        CANONICAL[index]
        for index in range(len(CANONICAL))
        if np.any(style_scores[:, index] > 0)
    }
    for held_out_fold in sorted(set(int(value) for value in folds)):
        embedding_centroids, arrangement_centroids, arrangement_z, _, _ = fit_fold_centroids(
            embeddings, arrangement, truths, folds, held_out_fold
        )
        candidates = sorted(
            style_supported | set(embedding_centroids), key=CANONICAL_INDEX.__getitem__
        )
        for row_index in np.where(folds == held_out_fold)[0]:
            best: tuple[float, int, str] | None = None
            for genre in candidates:
                style = float(style_scores[row_index, CANONICAL_INDEX[genre]])
                embedding_similarity = 0.5
                if genre in embedding_centroids:
                    cosine = float(np.dot(embeddings[row_index], embedding_centroids[genre]))
                    embedding_similarity = (max(-1.0, min(1.0, cosine)) + 1.0) / 2.0
                arrangement_similarity = 0.5
                if genre in arrangement_centroids:
                    distance = float(
                        np.sqrt(
                            np.mean(
                                np.square(
                                    arrangement_z[row_index] - arrangement_centroids[genre]
                                )
                            )
                        )
                    )
                    arrangement_similarity = math.exp(-distance)
                score = 0.70 * style + 0.20 * embedding_similarity + 0.10 * arrangement_similarity
                candidate = (score, -CANONICAL_INDEX[genre], genre)
                if best is None or candidate > best:
                    best = candidate
            predictions[row_index] = best[2] if best else "Ambient"
    return predictions


def aggregate_metrics(truths: list[str], predictions: list[str], folds: np.ndarray) -> dict[str, Any]:
    support = len(truths)
    exact = sum(truth == predicted for truth, predicted in zip(truths, predictions, strict=True))
    same_family = sum(
        family(truth) == family(predicted)
        for truth, predicted in zip(truths, predictions, strict=True)
    )
    same_family_confusions = sum(
        truth != predicted and family(truth) == family(predicted)
        for truth, predicted in zip(truths, predictions, strict=True)
    )
    per_genre: dict[str, GenreMetrics] = {}
    for genre in sorted(set(truths), key=CANONICAL_INDEX.__getitem__):
        genre_indices = [index for index, truth in enumerate(truths) if truth == genre]
        genre_support = len(genre_indices)
        predicted_count = predictions.count(genre)
        genre_exact = sum(predictions[index] == genre for index in genre_indices)
        recall = safe_fraction(genre_exact, genre_support)
        precision = safe_fraction(genre_exact, predicted_count)
        confusion = Counter(
            predictions[index] for index in genre_indices if predictions[index] != genre
        )
        leading = [
            {"recommended": predicted, "count": count}
            for predicted, count in sorted(confusion.items(), key=lambda item: (-item[1], item[0]))[:3]
        ]
        per_genre[genre] = GenreMetrics(
            support=genre_support,
            predicted=predicted_count,
            exact=genre_exact,
            recall=recall,
            precision=precision,
            f1=safe_f1(precision, recall),
            leading_confusions=leading,
        )
    macro_recall = float(np.mean([metric.recall for metric in per_genre.values()]))
    macro_f1 = float(np.mean([metric.f1 for metric in per_genre.values()]))
    fold_metrics = []
    for fold in sorted(set(int(value) for value in folds)):
        indices = np.where(folds == fold)[0]
        fold_truths = [truths[index] for index in indices]
        fold_predictions = [predictions[index] for index in indices]
        fold_genres = sorted(set(fold_truths), key=CANONICAL_INDEX.__getitem__)
        fold_f1 = []
        for genre in fold_genres:
            truth_count = fold_truths.count(genre)
            predicted_count = fold_predictions.count(genre)
            true_positive = sum(
                truth == genre and predicted == genre
                for truth, predicted in zip(fold_truths, fold_predictions, strict=True)
            )
            recall = safe_fraction(true_positive, truth_count)
            precision = safe_fraction(true_positive, predicted_count)
            fold_f1.append(safe_f1(precision, recall))
        fold_metrics.append(
            {
                "fold": fold,
                "support": len(indices),
                "exact_accuracy": safe_fraction(
                    sum(
                        truth == predicted
                        for truth, predicted in zip(fold_truths, fold_predictions, strict=True)
                    ),
                    len(indices),
                ),
                "macro_f1": float(np.mean(fold_f1)) if fold_f1 else 0.0,
            }
        )
    return {
        "support": support,
        "exact": exact,
        "exact_accuracy": safe_fraction(exact, support),
        "macro_recall": macro_recall,
        "macro_f1": macro_f1,
        "same_family_accuracy": safe_fraction(same_family, support),
        "same_family_confusion_rate": safe_fraction(same_family_confusions, support),
        "per_genre": {genre: asdict(metric) for genre, metric in per_genre.items()},
        "folds": fold_metrics,
    }


def gate(metrics: dict[str, Any], baseline: dict[str, Any]) -> dict[str, Any]:
    baseline_folds = {row["fold"]: row for row in baseline["folds"]}
    every_fold_improves = all(
        row["macro_f1"] > baseline_folds[row["fold"]]["macro_f1"]
        for row in metrics["folds"]
    )
    recall_losses = []
    for genre, baseline_genre in baseline["per_genre"].items():
        if baseline_genre["support"] < 10:
            continue
        current = metrics["per_genre"][genre]
        if current["recall"] < baseline_genre["recall"] - 0.10 - 1e-12:
            recall_losses.append(genre)
    checks = {
        "macro_f1_improvement": metrics["macro_f1"] >= baseline["macro_f1"] + 0.08 - 1e-12,
        "macro_recall_improvement": metrics["macro_recall"]
        >= baseline["macro_recall"] + 0.08 - 1e-12,
        "exact_accuracy_improvement": metrics["exact_accuracy"]
        >= baseline["exact_accuracy"] + 0.05 - 1e-12,
        "every_fold_macro_f1_improves": every_fold_improves,
        "same_family_non_regression": metrics["same_family_accuracy"]
        >= baseline["same_family_accuracy"] - 1e-12,
        "per_genre_recall_non_regression": not recall_losses,
        "genres_losing_more_than_0_10_recall": recall_losses,
    }
    checks["passed"] = all(
        value for key, value in checks.items() if key != "genres_losing_more_than_0_10_recall"
    )
    return checks


def run(args: argparse.Namespace) -> dict[str, Any]:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    stage_a = json.loads(args.stage_a.read_text(encoding="utf-8"))
    metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    if manifest["corpus_fingerprint"] != stage_a["corpus_fingerprint"]:
        raise ValueError("Stage A and embedding manifest corpus fingerprints differ")
    model_sha = sha256_file(args.model)
    metadata_sha = sha256_file(args.metadata)
    if model_sha != manifest["model_sha256"]:
        raise ValueError("model SHA-256 differs from frozen manifest")
    if metadata_sha != manifest["metadata_sha256"]:
        raise ValueError("metadata SHA-256 differs from frozen manifest")
    rows = manifest["rows"]
    truths = [row["truth"] for row in rows]
    folds = np.asarray([row["fold"] for row in rows], dtype=np.int64)
    arrangement = np.asarray([row["arrangement_dynamic"] for row in rows], dtype=np.float64)
    style_probabilities, embeddings = infer_features(
        args.model,
        args.mels,
        len(rows),
        int(manifest["patches_per_track"]),
        args.intra_op_threads,
    )
    style_scores = canonical_style_scores(style_probabilities, metadata["classes"])
    style_predictions = predictions_from_style(style_scores)
    fusion_predictions = predictions_from_fusion(
        style_scores, embeddings, arrangement, truths, folds
    )
    style_metrics = aggregate_metrics(truths, style_predictions, folds)
    fusion_metrics = aggregate_metrics(truths, fusion_predictions, folds)
    baseline = stage_a["baseline"]
    style_gate = gate(style_metrics, baseline)
    fusion_gate = gate(fusion_metrics, baseline)

    np.savez_compressed(
        args.feature_artifact,
        style_scores=style_scores,
        embeddings=embeddings,
        arrangement=arrangement,
        truth_indices=np.asarray([CANONICAL_INDEX[truth] for truth in truths], dtype=np.int16),
        folds=folds.astype(np.int8),
    )
    feature_sha = sha256_file(args.feature_artifact)
    supported = sorted(
        {genre for label in metadata["classes"] if (genre := class_to_canonical(label))},
        key=CANONICAL_INDEX.__getitem__,
    )
    truth_genres = sorted(set(truths), key=CANONICAL_INDEX.__getitem__)
    passed = [
        name
        for name, outcome in [
            ("style_projection", style_gate),
            ("fixed_70_20_10_fusion", fusion_gate),
        ]
        if outcome["passed"]
    ]
    return {
        "experiment_id": "discogs-effnet-genre-evaluation-v2-expanded-corpus",
        "method_status": "pre_registered_expanded_corpus_development_evaluation",
        "corpus_fingerprint": manifest["corpus_fingerprint"],
        "rows": len(rows),
        "fold_count": manifest["fold_count"],
        "patches_per_track": manifest["patches_per_track"],
        "preprocessing": {
            "sample_rate": 16_000,
            "frame_size": 512,
            "hop_size": 256,
            "mel_bands": 96,
            "patch_frames": 128,
            "selection": "12 evenly spaced patches across full decoded duration",
            "aggregation": "mean probabilities; mean of patch-L2 embeddings then track L2",
        },
        "model": {
            "url": manifest["model_url"],
            "sha256": model_sha,
            "metadata_url": manifest["metadata_url"],
            "metadata_sha256": metadata_sha,
            "onnxruntime_version": ort.__version__,
            "numpy_version": np.__version__,
        },
        "style_mapping": {
            "mapped_model_classes": sum(
                class_to_canonical(label) is not None for label in metadata["classes"]
            ),
            "supported_canonical_genres": supported,
            "unsupported_truth_genres": [genre for genre in truth_genres if genre not in supported],
        },
        "feature_artifact": {
            "path": str(args.feature_artifact),
            "sha256": feature_sha,
        },
        "baseline": {
            key: baseline[key]
            for key in [
                "support",
                "exact_accuracy",
                "macro_recall",
                "macro_f1",
                "same_family_accuracy",
                "folds",
            ]
        },
        "configurations": {
            "style_projection": {"metrics": style_metrics, "gate": style_gate},
            "fixed_70_20_10_fusion": {"metrics": fusion_metrics, "gate": fusion_gate},
        },
        "selected_configuration": passed[0] if passed else None,
        "outcome": "representation_passed_development_gate" if passed else "bounded_negative",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--stage-a", required=True, type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--metadata", required=True, type=Path)
    parser.add_argument("--mels", required=True, type=Path)
    parser.add_argument("--feature-artifact", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--intra-op-threads", type=int, default=4)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = {
        "output": str(args.output),
        "outcome": result["outcome"],
        "selected_configuration": result["selected_configuration"],
        "feature_artifact": result["feature_artifact"],
        "configurations": {
            name: {
                "exact_accuracy": value["metrics"]["exact_accuracy"],
                "macro_recall": value["metrics"]["macro_recall"],
                "macro_f1": value["metrics"]["macro_f1"],
                "same_family_accuracy": value["metrics"]["same_family_accuracy"],
                "gate": value["gate"],
            }
            for name, value in result["configurations"].items()
        },
    }
    print(json.dumps(summary, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
