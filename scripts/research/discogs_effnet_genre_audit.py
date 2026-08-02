#!/usr/bin/env python3
"""Build a frozen, private whole-library genre-audit listening batch."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

import discogs_effnet_genre_eval as base


EXPERIMENT_ID = "genre-audit-consensus-v1"
SEED = "genre-audit-consensus-v1-frozen"
RANKED_COUNT = 4
CONTROL_COUNT = 2
MIN_TARGET_SUPPORT = 8
MIN_TARGET_PRECISION = 0.60
ALLOWED_CONFIDENCE = {"high", "medium"}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_hash(*parts: object) -> str:
    value = "|".join(str(part) for part in (SEED, *parts))
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def normalized(value: str) -> str:
    return " ".join(re.findall(r"[a-z0-9]+", value.casefold()))


def release_group(row: dict[str, Any]) -> str:
    artist = normalized(row["artist"])
    release = normalized(row["album"]) or normalized(row["title"])
    return f"{artist}\0{release}"


def top_two_margin(scores: np.ndarray, indices: list[int]) -> tuple[str, float, float]:
    ranked = sorted(
        ((float(scores[index]), -index, base.CANONICAL[index]) for index in indices),
        reverse=True,
    )
    if not ranked:
        raise ValueError("cannot rank an empty candidate set")
    best_score, _, best_genre = ranked[0]
    second_score = ranked[1][0] if len(ranked) > 1 else 0.0
    return best_genre, best_score, best_score - second_score


def fit_full_centroids(
    embeddings: np.ndarray,
    arrangement: np.ndarray,
    truths: list[str],
) -> dict[str, Any]:
    means = np.nanmean(arrangement, axis=0)
    means = np.where(np.isfinite(means), means, 0.0)
    stddev = np.nanstd(arrangement, axis=0)
    active = np.isfinite(stddev) & (stddev > 1e-9)
    if not np.any(active):
        active = np.ones(arrangement.shape[1], dtype=bool)
        stddev = np.ones(arrangement.shape[1], dtype=np.float64)
    filled = np.where(np.isfinite(arrangement), arrangement, means)
    arrangement_z = (filled[:, active] - means[active]) / stddev[active]

    embedding_centroids: dict[str, np.ndarray] = {}
    arrangement_centroids: dict[str, np.ndarray] = {}
    for genre in sorted(set(truths), key=base.CANONICAL_INDEX.__getitem__):
        mask = np.asarray([truth == genre for truth in truths])
        embedding_centroids[genre] = base.l2_normalize(embeddings[mask].mean(axis=0))
        arrangement_centroids[genre] = arrangement_z[mask].mean(axis=0)
    return {
        "means": means,
        "stddev": stddev,
        "active": active,
        "embedding_centroids": embedding_centroids,
        "arrangement_centroids": arrangement_centroids,
    }


def external_fusion_predictions(
    style_scores: np.ndarray,
    embeddings: np.ndarray,
    arrangement: np.ndarray,
    centroid_state: dict[str, Any],
    style_supported: set[str],
) -> list[dict[str, Any]]:
    means = centroid_state["means"]
    stddev = centroid_state["stddev"]
    active = centroid_state["active"]
    filled = np.where(np.isfinite(arrangement), arrangement, means)
    arrangement_z = (filled[:, active] - means[active]) / stddev[active]
    embedding_centroids = centroid_state["embedding_centroids"]
    arrangement_centroids = centroid_state["arrangement_centroids"]
    candidates = sorted(
        style_supported | set(embedding_centroids),
        key=base.CANONICAL_INDEX.__getitem__,
    )

    output = []
    for row_index in range(style_scores.shape[0]):
        scored = []
        for genre in candidates:
            style = float(style_scores[row_index, base.CANONICAL_INDEX[genre]])
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
                                arrangement_z[row_index]
                                - arrangement_centroids[genre]
                            )
                        )
                    )
                )
                arrangement_similarity = math.exp(-distance)
            score = 0.70 * style + 0.20 * embedding_similarity + 0.10 * arrangement_similarity
            scored.append((score, -base.CANONICAL_INDEX[genre], genre))
        scored.sort(reverse=True)
        best_score, _, best_genre = scored[0]
        output.append(
            {
                "genre": best_genre,
                "score": best_score,
                "margin": best_score - scored[1][0],
            }
        )
    return output


def qualifying_targets(stage_b: dict[str, Any]) -> dict[str, dict[str, float | int]]:
    metrics = stage_b["configurations"]["fixed_70_20_10_fusion"]["metrics"]["per_genre"]
    return {
        genre: {
            "support": int(values["support"]),
            "precision": float(values["precision"]),
        }
        for genre, values in metrics.items()
        if int(values["support"]) >= MIN_TARGET_SUPPORT
        and float(values["precision"]) >= MIN_TARGET_PRECISION
    }


def enrich_rows(
    rows: list[dict[str, Any]],
    style_scores: np.ndarray,
    fusion: list[dict[str, Any]],
    qualifying: dict[str, dict[str, float | int]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    modeled_indices = [base.CANONICAL_INDEX[genre] for genre in base.MODELED_CANONICAL]
    ranked = []
    controls = []
    for row_index, row in enumerate(rows):
        style_genre, style_score, style_margin = top_two_margin(
            style_scores[row_index], modeled_indices
        )
        baseline = row["baseline_recommendation"]
        fused = fusion[row_index]
        current = row["current_genre"]
        if (
            baseline is None
            or row["baseline_confidence"] not in ALLOWED_CONFIDENCE
            or baseline != style_genre
            or baseline != fused["genre"]
            or baseline not in qualifying
        ):
            continue
        enriched = {
            **row,
            "hidden_target": baseline,
            "style_score": style_score,
            "style_margin": style_margin,
            "fusion_score": fused["score"],
            "fusion_margin": fused["margin"],
            "target_support": qualifying[baseline]["support"],
            "target_precision": qualifying[baseline]["precision"],
            "release_group": release_group(row),
            "stable_hash": stable_hash(row["track_id"], row["file_path"]),
        }
        if current == baseline:
            controls.append(enriched)
        else:
            enriched["cross_family"] = base.family(current) != base.family(baseline)
            ranked.append(enriched)
    return ranked, controls


def ranked_key(row: dict[str, Any]) -> tuple[Any, ...]:
    return (
        0 if row["cross_family"] else 1,
        0 if row["baseline_confidence"] == "high" else 1,
        -float(row["target_precision"]),
        -float(row["fusion_margin"]),
        -float(row["style_margin"]),
        row["stable_hash"],
    )


def select_ranked(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    selected = []
    targets = set()
    groups = set()
    family_counts: Counter[str] = Counter()
    for row in sorted(rows, key=ranked_key):
        target = row["hidden_target"]
        target_family = base.family(target)
        if (
            target in targets
            or row["release_group"] in groups
            or family_counts[target_family] >= 2
        ):
            continue
        selected.append({**row, "cohort": "ranked"})
        targets.add(target)
        groups.add(row["release_group"])
        family_counts[target_family] += 1
        if len(selected) == RANKED_COUNT:
            return selected
    raise ValueError(
        f"frozen ranked rule produced {len(selected)} rows; required {RANKED_COUNT}"
    )


def select_controls(
    rows: list[dict[str, Any]], ranked: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    used_groups = {row["release_group"] for row in ranked}
    selected = []
    used_targets = set()
    for ranked_row in ranked:
        target = ranked_row["hidden_target"]
        if target in used_targets:
            continue
        candidates = [
            row
            for row in rows
            if row["hidden_target"] == target and row["release_group"] not in used_groups
        ]
        candidates.sort(
            key=lambda row: (
                abs(float(row["bpm"]) - float(ranked_row["bpm"])),
                row["stable_hash"],
            )
        )
        if not candidates:
            continue
        chosen = {**candidates[0], "cohort": "control"}
        selected.append(chosen)
        used_targets.add(target)
        used_groups.add(chosen["release_group"])
        if len(selected) == CONTROL_COUNT:
            return selected
    raise ValueError(
        f"frozen control rule produced {len(selected)} rows; required {CONTROL_COUNT}"
    )


def private_selected_row(row: dict[str, Any], position: int) -> dict[str, Any]:
    return {
        "position": position,
        "code": f"GA{position:02d}",
        "cohort": row["cohort"],
        "track_id": row["track_id"],
        "file_path": row["file_path"],
        "artist": row["artist"],
        "title": row["title"],
        "album": row["album"],
        "bpm": row["bpm"],
        "current_genre": row["current_genre"],
        "hidden_target": row["hidden_target"],
        "baseline_confidence": row["baseline_confidence"],
        "target_support": row["target_support"],
        "target_precision": row["target_precision"],
        "style_score": row["style_score"],
        "style_margin": row["style_margin"],
        "fusion_score": row["fusion_score"],
        "fusion_margin": row["fusion_margin"],
        "release_group": row["release_group"],
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    audit = json.loads(args.audit_manifest.read_text(encoding="utf-8"))
    development = json.loads(args.development_manifest.read_text(encoding="utf-8"))
    stage_b = json.loads(args.stage_b.read_text(encoding="utf-8"))
    metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    if audit["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("unexpected audit experiment ID")
    if audit["development_corpus_fingerprint"] != development["corpus_fingerprint"]:
        raise ValueError("audit and development corpus fingerprints differ")
    if development["corpus_fingerprint"] != stage_b["corpus_fingerprint"]:
        raise ValueError("development manifest and Stage B fingerprints differ")
    if sha256_file(args.development_features) != stage_b["feature_artifact"]["sha256"]:
        raise ValueError("development feature artifact SHA-256 differs from Stage B")
    if sha256_file(args.model) != audit["model_sha256"]:
        raise ValueError("model SHA-256 differs from frozen audit manifest")
    if sha256_file(args.metadata) != audit["metadata_sha256"]:
        raise ValueError("metadata SHA-256 differs from frozen audit manifest")

    development_artifact = np.load(args.development_features, allow_pickle=False)
    development_rows = development["rows"]
    development_truths = [row["truth"] for row in development_rows]
    if len(development_truths) != len(development_artifact["embeddings"]):
        raise ValueError("development manifest and feature row counts differ")
    centroid_state = fit_full_centroids(
        development_artifact["embeddings"].astype(np.float64),
        development_artifact["arrangement"].astype(np.float64),
        development_truths,
    )

    rows = audit["rows"]
    style_probabilities, embeddings = base.infer_features(
        args.model,
        args.mels,
        len(rows),
        int(audit["patches_per_track"]),
        args.intra_op_threads,
    )
    style_scores = base.canonical_style_scores(style_probabilities, metadata["classes"])
    arrangement = np.asarray(
        [row["arrangement_dynamic"] for row in rows], dtype=np.float64
    )
    style_supported = {
        genre
        for label in metadata["classes"]
        if (genre := base.class_to_canonical(label)) is not None
    }
    fusion = external_fusion_predictions(
        style_scores,
        embeddings,
        arrangement,
        centroid_state,
        style_supported,
    )
    qualifying = qualifying_targets(stage_b)
    ranked_pool, control_pool = enrich_rows(
        rows, style_scores, fusion, qualifying
    )
    ranked = select_ranked(ranked_pool)
    controls = select_controls(control_pool, ranked)
    roster = sorted(
        [*ranked, *controls],
        key=lambda row: stable_hash("shuffle", row["track_id"], row["cohort"]),
    )
    selected = [private_selected_row(row, index + 1) for index, row in enumerate(roster)]

    np.savez_compressed(
        args.candidate_feature_artifact,
        style_scores=style_scores,
        embeddings=embeddings,
        arrangement=arrangement,
    )
    roster_sha = hashlib.sha256(
        json.dumps(selected, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()
    return {
        "experiment_id": EXPERIMENT_ID,
        "method_status": "frozen_private_read_only_audit",
        "candidate_corpus_fingerprint": audit["candidate_corpus_fingerprint"],
        "development_corpus_fingerprint": development["corpus_fingerprint"],
        "inputs": {
            "audit_manifest_sha256": sha256_file(args.audit_manifest),
            "development_manifest_sha256": sha256_file(args.development_manifest),
            "development_feature_sha256": sha256_file(args.development_features),
            "stage_b_result_sha256": sha256_file(args.stage_b),
            "candidate_feature_sha256": sha256_file(args.candidate_feature_artifact),
            "model_sha256": sha256_file(args.model),
            "metadata_sha256": sha256_file(args.metadata),
        },
        "universe": {
            "library_rows": audit["library_rows"],
            "excluded_playlist_rows": audit["excluded_playlist_rows"],
            "missing_file_rows": audit["missing_file_rows"],
            "candidate_input_rows": audit["candidate_input_rows"],
            "canonical_candidate_rows": audit["canonical_candidate_rows"],
            "usable_rows": audit["usable_rows"],
            "ranked_pool_rows": len(ranked_pool),
            "control_pool_rows": len(control_pool),
            "qualifying_target_genres": len(qualifying),
        },
        "frozen_rule": {
            "ranked_count": RANKED_COUNT,
            "control_count": CONTROL_COUNT,
            "minimum_target_support": MIN_TARGET_SUPPORT,
            "minimum_target_precision": MIN_TARGET_PRECISION,
            "allowed_baseline_confidence": sorted(ALLOWED_CONFIDENCE),
            "seed_sha256": hashlib.sha256(SEED.encode("utf-8")).hexdigest(),
        },
        "cohort_counts": {"ranked": len(ranked), "control": len(controls)},
        "roster_sha256": roster_sha,
        "selected": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--audit-manifest", required=True, type=Path)
    parser.add_argument("--development-manifest", required=True, type=Path)
    parser.add_argument("--development-features", required=True, type=Path)
    parser.add_argument("--stage-b", required=True, type=Path)
    parser.add_argument("--model", required=True, type=Path)
    parser.add_argument("--metadata", required=True, type=Path)
    parser.add_argument("--mels", required=True, type=Path)
    parser.add_argument("--candidate-feature-artifact", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--intra-op-threads", type=int, default=1)
    args = parser.parse_args()
    result = run(args)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "experiment_id": result["experiment_id"],
                "universe": result["universe"],
                "cohort_counts": result["cohort_counts"],
                "roster_sha256": result["roster_sha256"],
                "candidate_feature_sha256": result["inputs"][
                    "candidate_feature_sha256"
                ],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
