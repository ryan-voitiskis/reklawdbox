#!/usr/bin/env python3
"""Evaluate the frozen Plan 073 holdout after every blind verdict is sealed."""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

import build_genre_intelligence_corpus as corpus


EXPERIMENT_ID = "genre-intelligence-precision-first-evaluation-v1"
METHOD_STATUS = "sealed_precision_first_exact_primary_parent_evaluation"
SUPPORTED_PARENTS = ["Ambient", "Electro", "House", "Reggae", "Techno"]
EXPECTED_HOLDOUT_ROWS = 150
EXPECTED_OFFERS = 35
MINIMUM_OFFERS = 30
MINIMUM_COVERAGE = 0.20
MINIMUM_AGGREGATE_PRECISION = 0.90
MINIMUM_PARENT_OFFERS = 5
MINIMUM_PARENT_PRECISION = 0.80
MINIMUM_PAIRED_IMPROVEMENT = 0.05
SENSITIVITY_CONFIDENCES = {"high", "medium"}
EXPECTED_HASHES = {
    "predictions_sha256": (
        "0fae375e636631cac88a6762f3c64ed69caf50b602a737fde61d5b122034e1ff"
    ),
    "review_manifest_sha256": (
        "7f8c84c706f50638533b62e5478cdab0ccd88caf3fd96cdc6c9391afe37e5993"
    ),
    "holdout_input_summary_sha256": (
        "9bdc70094721f827ae4b461ced24f96227ce3a5377bc414e6a916bf981e8d2a3"
    ),
    "decoded_audio_isolation_sha256": (
        "3b11b896511c0571aec64f23f4ed7711fb11b5738d0cbb95b12609374168b3ac"
    ),
}
EXPECTED_MODEL_SHA256 = (
    "2633db33edc2e2af4e3e42adae9cea945f4453f7524522c4f4fecca8530f30df"
)
EXPECTED_BATCHES = {f"P{index:02d}" for index in range(1, 7)}
EXPECTED_BATCH_SIZES = [6, 6, 6, 6, 6, 5]
CONFIG_FIELDS = {
    "schema_version",
    "experiment_id",
    "scorer_source_sha256",
    *EXPECTED_HASHES,
    "verdict_sha256",
    "policy",
}
POLICY = {
    "minimum_offers": MINIMUM_OFFERS,
    "minimum_coverage": MINIMUM_COVERAGE,
    "minimum_aggregate_precision": MINIMUM_AGGREGATE_PRECISION,
    "minimum_parent_offers": MINIMUM_PARENT_OFFERS,
    "minimum_parent_precision": MINIMUM_PARENT_PRECISION,
    "minimum_paired_v033_precision_improvement": MINIMUM_PAIRED_IMPROVEMENT,
    "supported_parents": SUPPORTED_PARENTS,
    "exact_primary_parent_only": True,
    "ambiguous_and_skip_are_incorrect": True,
    "alternatives_never_receive_credit": True,
    "confidence_does_not_change_primary_gate": True,
    "sensitivity_confidences": sorted(SENSITIVITY_CONFIDENCES),
    "paired_comparison_uses_rows_where_o3_and_v033_both_offer": True,
}
SELECTED_FIELDS = {
    "album",
    "artist",
    "artist_group",
    "code",
    "file_path",
    "position",
    "release_group",
    "source_row_id_private",
    "title",
    "track_id",
}


def json_file(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def batch_label(experiment_id: str) -> str:
    match = re.fullmatch(
        r"genre-intelligence-precision-first-v1-h(\d{2})", experiment_id
    )
    if match is None:
        raise ValueError(f"unexpected review batch ID: {experiment_id!r}")
    return f"P{match.group(1)}"


def unique_by_batch(
    paths: Iterable[Path], *, expected_prefix: str
) -> dict[str, tuple[Path, dict[str, Any]]]:
    result: dict[str, tuple[Path, dict[str, Any]]] = {}
    for path in paths:
        value = json_file(path)
        label = batch_label(str(value.get(expected_prefix, "")))
        if label in result:
            raise ValueError(f"duplicate review batch: {label}")
        result[label] = (path, value)
    return result


def validate_config(path: Path) -> dict[str, Any]:
    config = json_file(path)
    if set(config) != CONFIG_FIELDS:
        raise ValueError("evaluation config fields differ")
    if config["schema_version"] != 1 or config["experiment_id"] != EXPERIMENT_ID:
        raise ValueError("evaluation config identity differs")
    for field, expected in EXPECTED_HASHES.items():
        if config[field] != expected:
            raise ValueError(f"frozen {field} differs")
    if config["policy"] != POLICY:
        raise ValueError("frozen evaluation policy differs")
    verdict_hashes = config["verdict_sha256"]
    if not isinstance(verdict_hashes, dict) or set(verdict_hashes) != EXPECTED_BATCHES:
        raise ValueError("frozen verdict hashes must cover P01 through P06")
    if config["scorer_source_sha256"] != corpus.sha256_file(Path(__file__)):
        raise ValueError("scorer source differs from frozen config")
    return config


def validate_inputs(
    config: dict[str, Any],
    *,
    predictions_path: Path,
    review_manifest_path: Path,
    holdout_input_summary_path: Path,
    decoded_audio_isolation_path: Path,
    mapping_paths: list[Path],
    verdict_paths: list[Path],
) -> tuple[
    dict[str, Any],
    dict[str, Any],
    dict[str, tuple[Path, dict[str, Any]]],
    dict[str, tuple[Path, dict[str, Any]]],
    dict[str, Any],
]:
    observed = {
        "predictions_sha256": corpus.sha256_file(predictions_path),
        "review_manifest_sha256": corpus.sha256_file(review_manifest_path),
        "holdout_input_summary_sha256": corpus.sha256_file(
            holdout_input_summary_path
        ),
        "decoded_audio_isolation_sha256": corpus.sha256_file(
            decoded_audio_isolation_path
        ),
    }
    for field, value in observed.items():
        if value != config[field]:
            raise ValueError(f"{field} differs from the frozen evaluation config")

    predictions = json_file(predictions_path)
    manifest = json_file(review_manifest_path)
    mappings = unique_by_batch(mapping_paths, expected_prefix="experiment_id")
    verdicts = unique_by_batch(verdict_paths, expected_prefix="batch_id")
    if set(mappings) != EXPECTED_BATCHES or set(verdicts) != EXPECTED_BATCHES:
        raise ValueError("mapping and verdict inputs must cover P01 through P06")

    if predictions.get("method_status") != "frozen_model_powered_holdout_inference":
        raise ValueError("prediction method status differs")
    if (
        predictions.get("holdout_rows") != EXPECTED_HOLDOUT_ROWS
        or predictions.get("offers") != EXPECTED_OFFERS
        or predictions.get("abstentions")
        != EXPECTED_HOLDOUT_ROWS - EXPECTED_OFFERS
        or predictions.get("one_qualified") != EXPECTED_OFFERS
        or predictions.get("multi_qualified") != 0
        or not predictions.get("all_predictions_frozen_before_review")
        or not predictions.get("frozen_model_byte_match")
        or not predictions.get("reference_implementation_match")
        or predictions.get("identity_values_exposed") is not False
        or predictions.get("model_artifact_sha256") != EXPECTED_MODEL_SHA256
    ):
        raise ValueError("sealed prediction contract differs")
    if (
        predictions.get("inputs", {}).get("holdout_input_summary")
        != EXPECTED_HASHES["holdout_input_summary_sha256"]
        or predictions.get("inputs", {}).get("decoded_audio_isolation")
        != EXPECTED_HASHES["decoded_audio_isolation_sha256"]
    ):
        raise ValueError("prediction isolation inputs differ")

    if manifest.get("method_status") != "all_offers_partitioned_before_listening":
        raise ValueError("review manifest method status differs")
    if (
        manifest.get("offers") != EXPECTED_OFFERS
        or manifest.get("batch_size_cap") != 6
        or manifest.get("predictions_sha256") != config["predictions_sha256"]
        or not manifest.get("model_and_sampling_fields_absent_from_mappings")
    ):
        raise ValueError("review manifest contract differs")
    manifest_batches = {str(row["batch"]): row for row in manifest.get("batches", [])}
    if set(manifest_batches) != EXPECTED_BATCHES:
        raise ValueError("review manifest batches differ")
    if [
        manifest_batches[f"P{index:02d}"]["rows"] for index in range(1, 7)
    ] != EXPECTED_BATCH_SIZES:
        raise ValueError("review batch sizes differ")

    mapping_hashes: dict[str, str] = {}
    verdict_hashes: dict[str, str] = {}
    for label in sorted(EXPECTED_BATCHES):
        mapping_path, mapping = mappings[label]
        verdict_path, verdict = verdicts[label]
        mapping_hash = corpus.sha256_file(mapping_path)
        if mapping_hash != manifest_batches[label]["mapping_sha256"]:
            raise ValueError(f"{label} mapping hash differs")
        if mapping.get("selection_rule", {}).get(
            "model_and_sampling_fields_absent"
        ) is not True:
            raise ValueError(f"{label} mapping privacy flag differs")
        selected = mapping.get("selected", [])
        for row in selected:
            if set(row) != SELECTED_FIELDS:
                raise ValueError(f"{label} mapping fields differ")
        if len(selected) != manifest_batches[label]["rows"]:
            raise ValueError(f"{label} mapping row count differs")
        if len(verdict.get("rows", [])) != manifest_batches[label]["rows"]:
            raise ValueError(f"{label} verdict row count differs")
        if verdict.get("batch_id") != mapping.get("experiment_id"):
            raise ValueError(f"{label} verdict and mapping IDs differ")
        mapping_hashes[label] = mapping_hash
        verdict_hashes[label] = corpus.sha256_file(verdict_path)
        if verdict_hashes[label] != config["verdict_sha256"][label]:
            raise ValueError(f"{label} verdict hash differs from frozen config")

    return predictions, manifest, mappings, verdicts, {
        **observed,
        "mapping_sha256": mapping_hashes,
        "verdict_sha256": verdict_hashes,
    }


def validate_isolation(
    predictions: dict[str, Any], summary: dict[str, Any], decoded: dict[str, Any]
) -> dict[str, Any]:
    leakage = summary.get("leakage", {})
    expected_zero = {
        "accepted_truth_path_overlap",
        "development_artist_overlap",
        "development_path_overlap",
        "development_release_overlap",
        "first_consumed_artist_overlap",
        "first_consumed_path_overlap",
        "first_consumed_release_overlap",
        "missing_files",
        "research_playlist_track_overlap",
        "second_consumed_artist_overlap",
        "second_consumed_path_overlap",
        "second_consumed_release_overlap",
    }
    zero_checks = {name: leakage.get(name) for name in sorted(expected_zero)}
    per_reference = decoded.get("per_reference", {})
    references_clean = (
        set(per_reference)
        == {"development", "first_consumed_holdout", "second_consumed_holdout"}
        and all(row.get("holdout_overlap") == 0 for row in per_reference.values())
        and all(
            row.get("rows") == row.get("unique_decoded_audio")
            for row in per_reference.values()
        )
    )
    passed = (
        summary.get("rows") == EXPECTED_HOLDOUT_ROWS
        and summary.get("identity_values_exposed") is False
        and all(value == 0 for value in zero_checks.values())
        and decoded.get("passed") is True
        and decoded.get("identity_values_exposed") is False
        and decoded.get("cross_partition_decoded_audio_overlap") == 0
        and decoded.get("holdout_rows") == EXPECTED_HOLDOUT_ROWS
        and decoded.get("holdout_unique_decoded_audio") == EXPECTED_HOLDOUT_ROWS
        and decoded.get("reference_rows")
        == decoded.get("reference_unique_decoded_audio")
        and references_clean
        and predictions.get("inputs", {}).get("holdout_input_summary")
        == EXPECTED_HASHES["holdout_input_summary_sha256"]
        and predictions.get("inputs", {}).get("decoded_audio_isolation")
        == EXPECTED_HASHES["decoded_audio_isolation_sha256"]
    )
    return {
        "passed": passed,
        **zero_checks,
        "cross_partition_decoded_audio_overlap": decoded.get(
            "cross_partition_decoded_audio_overlap"
        ),
        "holdout_unique_decoded_audio": decoded.get("holdout_unique_decoded_audio"),
        "reference_unique_decoded_audio": decoded.get(
            "reference_unique_decoded_audio"
        ),
    }


def join_rows(
    predictions: dict[str, Any],
    mappings: dict[str, tuple[Path, dict[str, Any]]],
    verdicts: dict[str, tuple[Path, dict[str, Any]]],
) -> list[dict[str, Any]]:
    prediction_rows = predictions.get("rows", [])
    prediction_by_id = {str(row["row_id"]): row for row in prediction_rows}
    if len(prediction_by_id) != EXPECTED_HOLDOUT_ROWS:
        raise ValueError("sealed prediction row IDs are not unique")
    offered_ids = {
        row_id for row_id, row in prediction_by_id.items() if bool(row.get("offered"))
    }
    joined = []
    mapped_ids = set()
    for label in sorted(mappings):
        mapping = mappings[label][1]
        verdict = verdicts[label][1]
        verdict_by_code = {str(row["code"]): row for row in verdict.get("rows", [])}
        selected = mapping.get("selected", [])
        if len(verdict_by_code) != len(selected):
            raise ValueError(f"{label} verdict codes are not unique")
        expected_codes = {str(row["code"]) for row in selected}
        if set(verdict_by_code) != expected_codes:
            raise ValueError(f"{label} verdict codes differ from the mapping")
        for mapping_row in sorted(selected, key=lambda row: int(row["position"])):
            code = str(mapping_row["code"])
            row_id = str(mapping_row["source_row_id_private"])
            if row_id in mapped_ids or row_id not in prediction_by_id:
                raise ValueError("review mapping row IDs differ")
            mapped_ids.add(row_id)
            prediction = prediction_by_id[row_id]
            if not prediction.get("offered"):
                raise ValueError("review mapping contains an abstained row")
            suggested_parent = prediction.get("suggested_parent")
            if suggested_parent not in SUPPORTED_PARENTS:
                raise ValueError("offered row has an unsupported suggested parent")
            v033_parent = prediction.get("v033_parent")
            if v033_parent is not None and v033_parent not in corpus.PARENT_GENRES:
                raise ValueError("v0.33 row has an unsupported parent")
            human = verdict_by_code[code]
            outcome = human.get("outcome")
            parent = human.get("genre")
            if outcome not in {"label", "ambiguous", "skip"}:
                raise ValueError("unsupported human verdict outcome")
            if outcome == "label" and parent not in corpus.PARENT_GENRES:
                raise ValueError("human label has an unsupported parent")
            if outcome != "label" and parent is not None:
                raise ValueError("non-label verdict unexpectedly has a parent")
            joined.append(
                {
                    "batch": label,
                    "code": code,
                    "source_row_id_private": row_id,
                    "suggested_parent": suggested_parent,
                    "v033_parent": v033_parent,
                    "outcome": outcome,
                    "human_parent": parent,
                    "confidence": human.get("confidence"),
                    "alternatives": human.get("alternatives", []),
                    "exact_match": outcome == "label" and parent == suggested_parent,
                    "v033_exact_match": outcome == "label" and parent == v033_parent,
                }
            )
    if mapped_ids != offered_ids or len(joined) != EXPECTED_OFFERS:
        raise ValueError("blind verdicts do not cover every offered row exactly once")
    return joined


def precision(correct: int, offers: int) -> float | None:
    return correct / offers if offers else None


def paired_v033(rows: list[dict[str, Any]]) -> dict[str, Any]:
    paired = [row for row in rows if row.get("v033_parent") is not None]
    candidate_correct = sum(bool(row["exact_match"]) for row in paired)
    baseline_correct = sum(bool(row["v033_exact_match"]) for row in paired)
    candidate_precision = precision(candidate_correct, len(paired))
    baseline_precision = precision(baseline_correct, len(paired))
    improvement = (
        candidate_precision - baseline_precision
        if candidate_precision is not None and baseline_precision is not None
        else None
    )
    return {
        "paired_offers": len(paired),
        "o3_correct": candidate_correct,
        "o3_offered_precision": candidate_precision,
        "v033_correct": baseline_correct,
        "v033_offered_precision": baseline_precision,
        "precision_improvement": improvement,
    }


def evaluate_rows(joined: list[dict[str, Any]]) -> dict[str, Any]:
    correct = sum(bool(row["exact_match"]) for row in joined)
    aggregate_precision = precision(correct, len(joined))
    per_parent = {}
    for parent in SUPPORTED_PARENTS:
        rows = [row for row in joined if row["suggested_parent"] == parent]
        parent_correct = sum(bool(row["exact_match"]) for row in rows)
        parent_precision = precision(parent_correct, len(rows))
        gated = len(rows) >= MINIMUM_PARENT_OFFERS
        per_parent[parent] = {
            "offers": len(rows),
            "correct": parent_correct,
            "incorrect": len(rows) - parent_correct,
            "offered_precision": parent_precision,
            "gate_applies": gated,
            "gate_passed": (
                parent_precision >= MINIMUM_PARENT_PRECISION if gated else None
            ),
        }
    sensitivity_rows = [
        row for row in joined if row.get("confidence") in SENSITIVITY_CONFIDENCES
    ]
    sensitivity_correct = sum(bool(row["exact_match"]) for row in sensitivity_rows)
    paired = paired_v033(joined)
    outcome_counts = Counter(str(row["outcome"]) for row in joined)
    human_parent_counts = Counter(
        str(row["human_parent"])
        for row in joined
        if row["outcome"] == "label"
    )
    gates = {
        "offers_at_least_30": len(joined) >= MINIMUM_OFFERS,
        "coverage_at_least_0_20": len(joined) / EXPECTED_HOLDOUT_ROWS
        >= MINIMUM_COVERAGE,
        "aggregate_precision_at_least_0_90": (
            aggregate_precision is not None
            and aggregate_precision >= MINIMUM_AGGREGATE_PRECISION
        ),
        "every_gated_parent_precision_at_least_0_80": all(
            row["gate_passed"] is not False for row in per_parent.values()
        ),
        "paired_v033_precision_improvement_at_least_0_05": (
            paired["precision_improvement"] is not None
            and paired["precision_improvement"] >= MINIMUM_PAIRED_IMPROVEMENT
        ),
    }
    return {
        "aggregate": {
            "offers": len(joined),
            "correct": correct,
            "incorrect": len(joined) - correct,
            "offered_precision": aggregate_precision,
        },
        "per_suggested_parent": per_parent,
        "paired_v033": paired,
        "high_medium_confidence_sensitivity": {
            "offers": len(sensitivity_rows),
            "correct": sensitivity_correct,
            "incorrect": len(sensitivity_rows) - sensitivity_correct,
            "offered_precision": precision(sensitivity_correct, len(sensitivity_rows)),
        },
        "outcomes": dict(sorted(outcome_counts.items())),
        "human_parent_labels": dict(sorted(human_parent_counts.items())),
        "metric_gates": gates,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = validate_config(args.config)
    predictions, manifest, mappings, verdicts, observed = validate_inputs(
        config,
        predictions_path=args.predictions,
        review_manifest_path=args.review_manifest,
        holdout_input_summary_path=args.holdout_input_summary,
        decoded_audio_isolation_path=args.decoded_audio_isolation,
        mapping_paths=args.mapping,
        verdict_paths=args.verdict,
    )
    isolation = validate_isolation(
        predictions,
        json_file(args.holdout_input_summary),
        json_file(args.decoded_audio_isolation),
    )
    joined = join_rows(predictions, mappings, verdicts)
    evaluation = evaluate_rows(joined)
    release_gate = {
        **evaluation["metric_gates"],
        "isolation_and_leakage_checks_passed": isolation["passed"],
    }
    release_gate["passed"] = all(release_gate.values())
    result = {
        "schema_version": 1,
        "experiment_id": EXPERIMENT_ID,
        "method_status": METHOD_STATUS,
        "config_sha256": corpus.sha256_file(args.config),
        "inputs": observed,
        "policy": POLICY,
        "holdout": {
            "rows": predictions["holdout_rows"],
            "offers": predictions["offers"],
            "abstentions": predictions["abstentions"],
            "coverage": predictions["coverage"],
            "predictions_frozen_before_review": predictions[
                "all_predictions_frozen_before_review"
            ],
            "all_verdicts_frozen_before_prediction_join": True,
            "track_identity_values_exposed_in_result": False,
            "review_manifest_method_status": manifest["method_status"],
        },
        "isolation": isolation,
        "evaluation": evaluation,
        "release_gate": release_gate,
        "rows": joined,
    }
    corpus.atomic_write(
        args.output,
        json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False).encode(
            "utf-8"
        )
        + b"\n",
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--predictions", required=True, type=Path)
    parser.add_argument("--review-manifest", required=True, type=Path)
    parser.add_argument("--holdout-input-summary", required=True, type=Path)
    parser.add_argument("--decoded-audio-isolation", required=True, type=Path)
    parser.add_argument("--mapping", action="append", required=True, type=Path)
    parser.add_argument("--verdict", action="append", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = run(args)
    print(
        json.dumps(
            {
                "output": str(args.output),
                "output_sha256": corpus.sha256_file(args.output),
                "holdout": result["holdout"],
                "isolation": result["isolation"],
                "evaluation": result["evaluation"],
                "release_gate": result["release_gate"],
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
