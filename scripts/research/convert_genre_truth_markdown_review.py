#!/usr/bin/env python3
"""Convert prediction-blind Markdown reviews into validated per-batch TSVs."""

from __future__ import annotations

import argparse
import csv
import io
import json
import re
from pathlib import Path
from typing import Any

import build_genre_intelligence_corpus as corpus


HEADING = re.compile(r"^### (GIP\d{2}-\d{2}): (.+?) – (.+)$")
FIELD = re.compile(r"^- (Verdict|Confidence|Alternatives|Notes):\s*(.*)$")
FIELDS = ["Verdict", "Confidence", "Alternatives", "Notes"]
TSV_HEADERS = [
    "position",
    "code",
    "artist",
    "title",
    "verdict",
    "confidence",
    "alternatives",
    "notes",
]


def parse_markdown(value: str) -> dict[str, dict[str, str]]:
    rows: dict[str, dict[str, str]] = {}
    current: dict[str, str] | None = None
    current_field: str | None = None
    for line_number, line in enumerate(value.splitlines(), start=1):
        heading = HEADING.fullmatch(line)
        if heading is not None:
            code = heading.group(1)
            if code in rows:
                raise ValueError(f"duplicate Markdown review code {code}")
            current = {
                "code": code,
                "artist": heading.group(2),
                "title": heading.group(3),
            }
            rows[code] = current
            current_field = None
            continue
        field = FIELD.fullmatch(line)
        if field is not None:
            if current is None:
                raise ValueError(
                    f"answer field before a track heading on line {line_number}"
                )
            name = field.group(1)
            if name in current:
                raise ValueError(f"duplicate {name} field for {current['code']}")
            current[name] = field.group(2).strip()
            current_field = name
            continue
        if current is not None and current_field is not None and line.strip():
            current[current_field] = f"{current[current_field]} {line.strip()}".strip()
    for code, row in rows.items():
        missing = [field for field in FIELDS if field not in row]
        if missing:
            raise ValueError(f"Markdown review fields missing for {code}: {missing}")
        if not row["Verdict"]:
            raise ValueError(f"Markdown verdict is blank for {code}")
        if not row["Confidence"]:
            raise ValueError(f"Markdown confidence is blank for {code}")
    if not rows:
        raise ValueError("Markdown review contains no track rows")
    return rows


def batch_label(mapping: dict[str, Any]) -> str:
    match = re.fullmatch(
        r"genre-intelligence-precision-first-v1-h(\d{2})",
        str(mapping.get("experiment_id", "")),
    )
    if match is None:
        raise ValueError("unexpected precision-first review mapping ID")
    return f"P{match.group(1)}"


def tsv_bytes(mapping: dict[str, Any], reviews: dict[str, dict[str, str]]) -> bytes:
    selected = mapping.get("selected")
    if not isinstance(selected, list) or not selected:
        raise ValueError("review mapping has no selected rows")
    output = io.StringIO(newline="")
    writer = csv.DictWriter(output, fieldnames=TSV_HEADERS, delimiter="\t")
    writer.writeheader()
    for selected_row in sorted(selected, key=lambda row: int(row["position"])):
        code = str(selected_row["code"])
        review = reviews.get(code)
        if review is None:
            raise ValueError(f"Markdown review is missing {code}")
        if (
            review["artist"] != str(selected_row["artist"])
            or review["title"] != str(selected_row["title"])
        ):
            raise ValueError(f"Markdown review identity differs for {code}")
        writer.writerow(
            {
                "position": selected_row["position"],
                "code": code,
                "artist": selected_row["artist"],
                "title": selected_row["title"],
                "verdict": review["Verdict"],
                "confidence": review["Confidence"],
                "alternatives": review["Alternatives"],
                "notes": review["Notes"],
            }
        )
    return output.getvalue().encode("utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mapping", action="append", required=True, type=Path)
    parser.add_argument("--review-md", action="append", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--output-manifest", required=True, type=Path)
    args = parser.parse_args()

    reviews: dict[str, dict[str, str]] = {}
    review_sources = []
    for path in args.review_md:
        parsed = parse_markdown(path.read_text(encoding="utf-8"))
        overlap = set(reviews).intersection(parsed)
        if overlap:
            raise ValueError(
                f"duplicate codes across Markdown files: {sorted(overlap)}"
            )
        reviews.update(parsed)
        review_sources.append(
            {"path": str(path), "sha256": corpus.sha256_file(path), "rows": len(parsed)}
        )

    mappings = []
    expected_codes = set()
    for path in args.mapping:
        mapping = json.loads(path.read_text(encoding="utf-8"))
        label = batch_label(mapping)
        codes = {str(row["code"]) for row in mapping.get("selected", [])}
        if expected_codes.intersection(codes):
            raise ValueError("review mappings contain duplicate codes")
        expected_codes.update(codes)
        mappings.append((label, path, mapping))
    if set(reviews) != expected_codes:
        raise ValueError(
            "Markdown and mapping codes differ: "
            f"missing={sorted(expected_codes - set(reviews))}, "
            f"extra={sorted(set(reviews) - expected_codes)}"
        )

    args.output_dir.mkdir(parents=True, exist_ok=True)
    outputs = []
    for label, mapping_path, mapping in sorted(mappings):
        output_path = args.output_dir / f"precision-first-review-{label.lower()}.tsv"
        corpus.atomic_write(output_path, tsv_bytes(mapping, reviews))
        outputs.append(
            {
                "batch": label,
                "rows": len(mapping["selected"]),
                "mapping_path": str(mapping_path),
                "mapping_sha256": corpus.sha256_file(mapping_path),
                "output_path": str(output_path),
                "output_sha256": corpus.sha256_file(output_path),
            }
        )
    manifest = {
        "schema_version": 1,
        "method_status": "prediction_blind_markdown_converted_to_tsv",
        "review_sources": review_sources,
        "rows": len(reviews),
        "outputs": outputs,
        "model_fields_read": False,
    }
    corpus.atomic_write(
        args.output_manifest,
        json.dumps(manifest, indent=2, sort_keys=True).encode("utf-8") + b"\n",
    )
    print(
        json.dumps(
            {
                "rows": len(reviews),
                "batch_rows": [row["rows"] for row in outputs],
                "output_manifest_sha256": corpus.sha256_file(args.output_manifest),
                "model_fields_read": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
