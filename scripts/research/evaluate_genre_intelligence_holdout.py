#!/usr/bin/env python3
"""Evaluate the frozen Plan 070 holdout after every blind verdict is sealed."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

import build_genre_intelligence_corpus as corpus


EXPERIMENT_ID = "genre-intelligence-holdout-evaluation-v1"
METHOD_STATUS = "sealed_holdout_exact_primary_parent_evaluation"
SUPPORTED_PARENTS = ["Ambient", "House", "Reggae", "Techno"]
EXPECTED_HOLDOUT_ROWS = 60
EXPECTED_OFFERS = 31
MINIMUM_OFFERS = 30
MINIMUM_AGGREGATE_PRECISION = 0.90
MINIMUM_PARENT_OFFERS = 5
MINIMUM_PARENT_PRECISION = 0.80
SENSITIVITY_CONFIDENCES = {"high", "medium"}
EXPECTED_HASHES = {
    "predictions_sha256": (
        "4e95f5cb364d2eb4966b5a3d0d1cbcdc2a30843f2b91475a86d4f350a40847b5"
    ),
    "review_manifest_sha256": (
        "bd06892fd97cef7b904676edd069d56546019ba742992c033ff277c9e0f81838"
    ),
    "holdout_input_summary_sha256": (
        "8423585a07fe00d4c6127d912931d6321ca5337dde2c18f53edb691f8d93d2a5"
    ),
    "development_exclusions_sha256": (
        "575b95fceada7565e3297a0420896bb32af83479a1bd5ff69c2b3b814e0c6c32"
    ),
    "decoded_audio_isolation_sha256": (
        "0e989353d7ae0cf7ca0978557225935753ff5c595f9af38722981eb3dcc903c6"
    ),
}
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
    "minimum_aggregate_precision": MINIMUM_AGGREGATE_PRECISION,
    "minimum_parent_offers": MINIMUM_PARENT_OFFERS,
    "minimum_parent_precision": MINIMUM_PARENT_PRECISION,
    "supported_parents": SUPPORTED_PARENTS,
    "exact_primary_parent_only": True,
    "ambiguous_and_skip_are_incorrect": True,
    "alternatives_never_receive_credit": True,
    "confidence_does_not_change_primary_gate": True,
    "sensitivity_confidences": sorted(SENSITIVITY_CONFIDENCES),
}
FORBIDDEN_MAPPING_FIELDS = {
    "current_genre",
    "internal_parent",
    "margin",
    "predicted_parent",
    "prediction",
    "sampling_stratum",
    "suggested_parent",
    "target_genre",
    "threshold",
}


def json_file(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected a JSON object: {path}")
    return value


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def pre_export_mapping_sha256(mapping: dict[str, Any]) -> str:
    original = {key: value for key, value in mapping.items() if key != "export"}
    payload = (
        json.dumps(original, indent=2, sort_keys=True, ensure_ascii=False).encode(
            "utf-8"
        )
        + b"\n"
    )
    return sha256_bytes(payload)


def batch_label(experiment_id: str) -> str:
    label = experiment_id.rsplit("-", 1)[-1].upper()
    if len(label) != 3 or not label.startswith("H") or not label[1:].isdigit():
        raise ValueError(f"unexpected review batch ID: {experiment_id!r}")
    return label


def unique_by_batch(
    paths: Iterable[Path], *, expected_prefix: str
) -> dict[str, tuple[Path, dict[str, Any]]]:
    result: dict[str, tuple[Path, dict[str, Any]]] = {}
    for path in paths:
        value = json_file(path)
        experiment_id = str(value.get(expected_prefix, ""))
        label = batch_label(experiment_id)
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
    if not isinstance(verdict_hashes, dict) or set(verdict_hashes) != {
        f"H{index:02d}" for index in range(1, 7)
    }:
        raise ValueError("frozen verdict hashes must cover H01 through H06")
    if config["scorer_source_sha256"] != corpus.sha256_file(Path(__file__)):
        raise ValueError("scorer source differs from frozen config")
    return config


def validate_inputs(
    config: dict[str, Any],
    *,
    predictions_path: Path,
    review_manifest_path: Path,
    holdout_input_summary_path: Path,
    development_exclusions_path: Path,
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
        "development_exclusions_sha256": corpus.sha256_file(
            development_exclusions_path
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
    expected_batches = {f"H{index:02d}" for index in range(1, 7)}
    if set(mappings) != expected_batches or set(verdicts) != expected_batches:
        raise ValueError("mapping and verdict inputs must cover H01 through H06")

    if predictions.get("method_status") != "frozen_full_fit_label_blind_holdout_inference":
        raise ValueError("prediction method status differs")
    if (
        predictions.get("holdout_rows") != EXPECTED_HOLDOUT_ROWS
        or predictions.get("offers") != EXPECTED_OFFERS
        or predictions.get("abstentions")
        != EXPECTED_HOLDOUT_ROWS - EXPECTED_OFFERS
        or not predictions.get("all_predictions_frozen_before_review")
        or predictions.get("identity_values_exposed") is not False
    ):
        raise ValueError("sealed prediction contract differs")
    if predictions.get("model_schema", {}).get("supported_parents") != SUPPORTED_PARENTS:
        raise ValueError("supported-parent order differs")
    if manifest.get("method_status") != "all_offers_partitioned_before_listening":
        raise ValueError("review manifest method status differs")
    if (
        manifest.get("offers") != EXPECTED_OFFERS
        or manifest.get("predictions_sha256") != config["predictions_sha256"]
        or not manifest.get("model_and_sampling_fields_absent_from_mappings")
    ):
        raise ValueError("review manifest contract differs")

    manifest_batches = {str(row["batch"]): row for row in manifest.get("batches", [])}
    if set(manifest_batches) != expected_batches:
        raise ValueError("review manifest batches differ")
    if [manifest_batches[f"H{index:02d}"]["rows"] for index in range(1, 7)] != [
        6,
        6,
        6,
        6,
        6,
        1,
    ]:
        raise ValueError("review batch sizes differ")

    observed_verdict_hashes: dict[str, str] = {}
    observed_mapping_hashes: dict[str, str] = {}
    for label in sorted(expected_batches):
        mapping_path, mapping = mappings[label]
        verdict_path, verdict = verdicts[label]
        mapping_hash = pre_export_mapping_sha256(mapping)
        if mapping_hash != manifest_batches[label]["mapping_sha256"]:
            raise ValueError(f"{label} pre-export mapping hash differs")
        if mapping.get("selection_rule", {}).get(
            "model_and_sampling_fields_absent"
        ) is not True:
            raise ValueError(f"{label} mapping privacy flag differs")
        for row in mapping.get("selected", []):
            overlap = FORBIDDEN_MAPPING_FIELDS.intersection(row)
            if overlap:
                raise ValueError(f"{label} mapping contains hidden fields: {overlap}")
        if len(mapping.get("selected", [])) != manifest_batches[label]["rows"]:
            raise ValueError(f"{label} mapping row count differs")
        if len(verdict.get("rows", [])) != manifest_batches[label]["rows"]:
            raise ValueError(f"{label} verdict row count differs")
        observed_mapping_hashes[label] = mapping_hash
        observed_verdict_hashes[label] = corpus.sha256_file(verdict_path)
        if observed_verdict_hashes[label] != config["verdict_sha256"][label]:
            raise ValueError(f"{label} verdict hash differs from the frozen config")

    return predictions, manifest, mappings, verdicts, {
        **observed,
        "mapping_pre_export_sha256": observed_mapping_hashes,
        "verdict_sha256": observed_verdict_hashes,
    }


def validate_isolation(
    predictions: dict[str, Any],
    summary: dict[str, Any],
    exclusions: dict[str, Any],
    decoded: dict[str, Any],
) -> dict[str, Any]:
    leakage = summary.get("leakage", {})
    exclusion_rows = exclusions.get("rows", [])
    passed = (
        summary.get("missing_files") == 0
        and leakage.get("accepted_truth_path_overlap") == 0
        and leakage.get("development_path_overlap") == 0
        and leakage.get("development_release_overlap") == 0
        and leakage.get("development_artist_overlap") == 1
        and summary.get("excluded_development_rows") == 1
        and len(exclusion_rows) == 1
        and "artist_group" in exclusion_rows[0].get("reasons", [])
        and decoded.get("passed") is True
        and decoded.get("cross_partition_decoded_audio_overlap") == 0
        and decoded.get("development_rows")
        == decoded.get("development_unique_decoded_audio")
        and decoded.get("holdout_rows") == decoded.get("holdout_unique_decoded_audio")
        and predictions.get("inputs", {}).get("holdout_input_summary_sha256")
        == EXPECTED_HASHES["holdout_input_summary_sha256"]
        and predictions.get("inputs", {}).get("development_exclusions_sha256")
        == EXPECTED_HASHES["development_exclusions_sha256"]
        and predictions.get("inputs", {}).get("decoded_audio_isolation_sha256")
        == EXPECTED_HASHES["decoded_audio_isolation_sha256"]
    )
    return {
        "passed": passed,
        "missing_files": summary.get("missing_files"),
        "accepted_truth_path_overlap": leakage.get("accepted_truth_path_overlap"),
        "development_path_overlap": leakage.get("development_path_overlap"),
        "development_release_overlap": leakage.get("development_release_overlap"),
        "development_artist_overlap_before_exclusion": leakage.get(
            "development_artist_overlap"
        ),
        "excluded_development_rows": summary.get("excluded_development_rows"),
        "cross_partition_decoded_audio_overlap": decoded.get(
            "cross_partition_decoded_audio_overlap"
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
            human = verdict_by_code[code]
            outcome = human.get("outcome")
            parent = human.get("genre")
            if outcome not in {"label", "ambiguous", "skip"}:
                raise ValueError("unsupported human verdict outcome")
            if outcome == "label" and parent not in corpus.PARENT_GENRES:
                raise ValueError("human label has an unsupported parent")
            if outcome != "label" and parent is not None:
                raise ValueError("non-label verdict unexpectedly has a parent")
            exact_match = outcome == "label" and parent == suggested_parent
            joined.append(
                {
                    "batch": label,
                    "code": code,
                    "source_row_id_private": row_id,
                    "suggested_parent": suggested_parent,
                    "outcome": outcome,
                    "human_parent": parent,
                    "confidence": human.get("confidence"),
                    "alternatives": human.get("alternatives", []),
                    "exact_match": exact_match,
                }
            )
    if mapped_ids != offered_ids or len(joined) != EXPECTED_OFFERS:
        raise ValueError("blind verdicts do not cover every offered row exactly once")
    return joined


def precision(correct: int, offers: int) -> float | None:
    return correct / offers if offers else None


def evaluate_rows(joined: list[dict[str, Any]]) -> dict[str, Any]:
    correct = sum(bool(row["exact_match"]) for row in joined)
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
    aggregate_precision = precision(correct, len(joined))
    outcome_counts = Counter(str(row["outcome"]) for row in joined)
    human_parent_counts = Counter(
        str(row["human_parent"])
        for row in joined
        if row["outcome"] == "label"
    )
    gate = {
        "offers_at_least_30": len(joined) >= MINIMUM_OFFERS,
        "aggregate_precision_at_least_0_90": (
            aggregate_precision is not None
            and aggregate_precision >= MINIMUM_AGGREGATE_PRECISION
        ),
        "every_gated_parent_precision_at_least_0_80": all(
            row["gate_passed"] is not False for row in per_parent.values()
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
        "high_medium_confidence_sensitivity": {
            "offers": len(sensitivity_rows),
            "correct": sensitivity_correct,
            "incorrect": len(sensitivity_rows) - sensitivity_correct,
            "offered_precision": precision(sensitivity_correct, len(sensitivity_rows)),
        },
        "outcomes": dict(sorted(outcome_counts.items())),
        "human_parent_labels": dict(sorted(human_parent_counts.items())),
        "metric_gates": gate,
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    config = validate_config(args.config)
    predictions, manifest, mappings, verdicts, observed = validate_inputs(
        config,
        predictions_path=args.predictions,
        review_manifest_path=args.review_manifest,
        holdout_input_summary_path=args.holdout_input_summary,
        development_exclusions_path=args.development_exclusions,
        decoded_audio_isolation_path=args.decoded_audio_isolation,
        mapping_paths=args.mapping,
        verdict_paths=args.verdict,
    )
    summary = json_file(args.holdout_input_summary)
    exclusions = json_file(args.development_exclusions)
    decoded = json_file(args.decoded_audio_isolation)
    isolation = validate_isolation(predictions, summary, exclusions, decoded)
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
            "coverage": predictions["offers"] / predictions["holdout_rows"],
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
    parser.add_argument("--development-exclusions", required=True, type=Path)
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
