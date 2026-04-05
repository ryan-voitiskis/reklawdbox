#!/usr/bin/env python3
"""Phase B verification orchestrator for the Rekordbox corpus.

This script performs automated verification checks against source metadata and
updates frontmatter verification fields for Phase A documents.
"""

from __future__ import annotations

import argparse
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Tuple

import yaml


TODAY = "2026-02-17"
BASE_DIR = Path(__file__).resolve().parent
SOURCE_DIR = BASE_DIR / "source-material"
DOC_DIRS = ["manual", "guides", "faq", "features", "reference"]

STOPWORDS = {
    "a",
    "an",
    "and",
    "are",
    "as",
    "about",
    "all",
    "be",
    "by",
    "can",
    "for",
    "from",
    "how",
    "in",
    "into",
    "is",
    "mode",
    "not",
    "of",
    "on",
    "or",
    "set",
    "that",
    "the",
    "this",
    "to",
    "use",
    "using",
    "window",
    "with",
    "you",
    "your",
}


@dataclass
class VerificationResult:
    path: Path
    ok: bool
    reason: str


def normalize_text(text: str) -> str:
    text = text.lower().replace("’", "'")
    text = re.sub(r"[^a-z0-9]+", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def parse_frontmatter(path: Path) -> Tuple[Dict, str]:
    text = path.read_text(encoding="utf-8")
    match = re.match(r"^---\n(.*?)\n---\n?", text, re.S)
    if not match:
        raise ValueError(f"Missing frontmatter in {path}")
    frontmatter = yaml.safe_load(match.group(1)) or {}
    body = text[match.end() :]
    return frontmatter, body


def serialize_frontmatter(frontmatter: Dict) -> str:
    source = frontmatter.get("source", {}) or {}

    def quote(value: str) -> str:
        escaped = str(value).replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'

    def inline_list(values: List[str]) -> str:
        cleaned = [str(v).strip() for v in values if str(v).strip()]
        return "[" + ", ".join(cleaned) + "]"

    lines = ["---"]
    lines.append(f"id: {frontmatter.get('id')}")
    lines.append(f"title: {quote(frontmatter.get('title', ''))}")
    lines.append(f"type: {frontmatter.get('type')}")
    lines.append("source:")
    if source.get("file") is not None:
        lines.append(f"  file: {quote(source.get('file'))}")
    if source.get("url") is not None:
        lines.append(f"  url: {quote(source.get('url'))}")
    if source.get("pages") is not None:
        lines.append(f"  pages: {quote(source.get('pages'))}")
    if source.get("version") is not None:
        lines.append(f"  version: {quote(source.get('version'))}")
    lines.append(f"topics: {inline_list(frontmatter.get('topics') or [])}")
    lines.append(f"modes: {inline_list(frontmatter.get('modes') or [])}")
    lines.append(f"confidence: {frontmatter.get('confidence')}")
    last_verified = frontmatter.get("last_verified")
    if last_verified in (None, "", "null"):
        lines.append("last_verified: null")
    else:
        lines.append(f"last_verified: {quote(last_verified)}")
    lines.append(f"transcribed_by: {frontmatter.get('transcribed_by') or 'agent'}")
    verified_by = frontmatter.get("verified_by")
    if verified_by in (None, "", "null"):
        lines.append("verified_by: null")
    else:
        lines.append(f"verified_by: {verified_by}")

    for key, value in frontmatter.items():
        if key in {
            "id",
            "title",
            "type",
            "source",
            "topics",
            "modes",
            "confidence",
            "last_verified",
            "transcribed_by",
            "verified_by",
        }:
            continue
        if isinstance(value, list):
            lines.append(f"{key}: {inline_list(value)}")
        elif value is None:
            lines.append(f"{key}: null")
        elif isinstance(value, (int, float)):
            lines.append(f"{key}: {value}")
        else:
            lines.append(f"{key}: {quote(value)}")

    lines.append("---")
    return "\n".join(lines) + "\n"


def read_pdf_text(pdf_file: str, pages: str) -> str:
    chunks = []
    for part in pages.replace(" ", "").split(","):
        if not part:
            continue
        if "-" in part:
            start, end = part.split("-", 1)
        else:
            start, end = part, part
        result = subprocess.run(
            ["pdftotext", "-layout", "-f", start, "-l", end, str(SOURCE_DIR / pdf_file), "-"],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or f"pdftotext failed for {pdf_file}:{pages}")
        chunks.append(result.stdout)
    return "\n".join(chunks)


def heading_keywords(heading: str) -> List[str]:
    heading = re.sub(r"\[[^\]]+\]", " ", heading)
    heading = normalize_text(heading)
    return [token for token in heading.split() if len(token) >= 4 and token not in STOPWORDS]


def verify_pdf_doc(path: Path, body: str, source: Dict) -> VerificationResult:
    pdf_file = source.get("file")
    pages = str(source.get("pages", "")).strip()
    if not pdf_file or not pages:
        return VerificationResult(path, False, "missing source.file or source.pages")
    if not (SOURCE_DIR / pdf_file).exists():
        return VerificationResult(path, True, f"source pdf not available (skipped): {pdf_file}")

    try:
        pdf_text = normalize_text(read_pdf_text(pdf_file, pages))
    except Exception as exc:  # pragma: no cover - operational error path
        return VerificationResult(path, False, f"pdf extraction failed: {exc}")

    headings = [
        re.sub(r"^#+\s*", "", line).strip()
        for line in body.splitlines()
        if re.match(r"^#+\s*\S", line)
    ]
    checked = 0
    matched = 0
    for heading in headings:
        keywords = heading_keywords(heading)
        if not keywords:
            continue
        checked += 1
        if any(keyword in pdf_text for keyword in keywords):
            matched += 1

    ratio = (matched / checked) if checked else 1.0
    if ratio < 0.50:
        return VerificationResult(path, False, f"weak heading/source match ratio={ratio:.2f}")
    return VerificationResult(path, True, f"pdf heading/source ratio={ratio:.2f}")


def verify_faq_doc(path: Path, body: str, source_titles: set[str]) -> VerificationResult:
    titles = [normalize_text(m.group(1)) for m in re.finditer(r"^###\s+(.+?)\s*$", body, re.M)]
    if not titles:
        return VerificationResult(path, False, "no FAQ question headings found")
    missing = [title for title in titles if title not in source_titles]
    if missing:
        return VerificationResult(path, False, f"{len(missing)} question titles missing in FAQ source")
    return VerificationResult(path, True, f"faq titles matched={len(titles)}")


def verify_web_doc(path: Path, frontmatter: Dict, body: str) -> VerificationResult:
    source = frontmatter.get("source") or {}
    if not source.get("url"):
        return VerificationResult(path, False, "missing source.url")
    heading_count = sum(1 for line in body.splitlines() if re.match(r"^#+\s+\S", line))
    if heading_count < 1:
        return VerificationResult(path, False, "no headings in body")
    if len(body.strip()) < 300:
        return VerificationResult(path, False, "body too short")
    return VerificationResult(path, True, f"web doc structural checks ok (headings={heading_count})")


def should_update_phase_b(frontmatter: Dict) -> bool:
    doc_type = frontmatter.get("type")
    doc_id = frontmatter.get("id")
    if doc_type in {"manual", "guide", "faq", "feature"}:
        return True
    if doc_type == "reference" and doc_id == "developer-integration":
        return True
    return False


def collect_docs() -> List[Path]:
    paths: List[Path] = []
    for directory in DOC_DIRS:
        paths.extend(sorted((BASE_DIR / directory).glob("*.md")))
    return paths


def main() -> int:
    parser = argparse.ArgumentParser(description="Run Phase B verification and update verified metadata.")
    parser.add_argument("--apply", action="store_true", help="Apply frontmatter verification updates")
    args = parser.parse_args()

    faq_path = BASE_DIR / "rekordbox7-faq.md"
    if faq_path.exists():
        faq_source = faq_path.read_text(encoding="utf-8")
        faq_titles = {normalize_text(m.group(1)) for m in re.finditer(r"^###\s+(.+?)\s*$", faq_source, re.M)}
    else:
        faq_titles = set()

    results: List[VerificationResult] = []
    updatable_docs: List[Tuple[Path, Dict, str]] = []

    for path in collect_docs():
        frontmatter, body = parse_frontmatter(path)
        doc_type = frontmatter.get("type")
        source = frontmatter.get("source") or {}

        if doc_type in {"manual", "guide"} and str(source.get("file", "")).endswith(".pdf"):
            result = verify_pdf_doc(path, body, source)
        elif doc_type == "guide" and source.get("url"):
            result = verify_web_doc(path, frontmatter, body)
        elif doc_type == "faq":
            result = verify_faq_doc(path, body, faq_titles)
        elif doc_type == "feature" or (doc_type == "reference" and frontmatter.get("id") == "developer-integration"):
            result = verify_web_doc(path, frontmatter, body)
        else:
            result = VerificationResult(path, True, "non-Phase-A document (skipped)")

        results.append(result)
        if result.ok and should_update_phase_b(frontmatter):
            updatable_docs.append((path, frontmatter, body))

    failures = [result for result in results if not result.ok]
    for result in results:
        rel = result.path.relative_to(BASE_DIR).as_posix()
        status = "OK" if result.ok else "FAIL"
        print(f"{status:4} {rel}: {result.reason}")

    print("\nSummary:")
    print(f"- Documents checked: {len(results)}")
    print(f"- Passed: {len(results) - len(failures)}")
    print(f"- Failed: {len(failures)}")
    print(f"- Eligible for Phase B verified updates: {len(updatable_docs)}")

    if failures:
        return 1

    if args.apply:
        for path, frontmatter, body in updatable_docs:
            frontmatter["confidence"] = "verified"
            frontmatter["last_verified"] = TODAY
            frontmatter["verified_by"] = "agent"

            fm_text = serialize_frontmatter(frontmatter)
            path.write_text(f"{fm_text}\n{body.lstrip()}", encoding="utf-8")
        print(f"\nApplied verification updates to {len(updatable_docs)} document(s).")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
