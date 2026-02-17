# Rekordbox Software Orchestration Handover

Date: 2026-02-17  
Scope: Post-wave handover after Waves 0-5 completion

## Current State

- Waves completed: `0, 1, 2, 3, 4, 5`
- Wave deferred: `6` (optional FAQ dedup)
- Tracker of record: `docs/rekordbox/SOFTWARE-ORCHESTRATION-PLAN.md`

## What Landed

- Manifest-first corpus retrieval and ranking:
  - `src/corpus.rs`
  - `Cargo.toml` / `Cargo.lock` (`serde_yaml`)
- MCP workflow provenance integration (XML + genre paths):
  - `src/tools.rs`
- Eval modules + harness:
  - `src/eval_routing.rs`
  - `src/eval_tasks.rs`
  - `scripts/run-rekordbox-evals.sh`
  - `src/main.rs` module wiring
- CI automation:
  - `.github/workflows/corpus-ci.yml`
- Docs/operational guidance:
  - `docs/rekordbox/README.md`
  - `docs/rekordbox/reference/developer-integration.md`
  - `docs/rekordbox/SOFTWARE-ORCHESTRATION-PLAN.md`

## Verified Checks (Latest Run)

- `bash docs/rekordbox/validate-corpus.sh` -> `0 errors, 4 warnings` (expected boundary overlaps)
- `python3 docs/rekordbox/verify-phase-b.py` -> pass (`65 checked, 0 failed`)
- `cargo test` -> pass (`52 passed, 0 failed, 21 ignored`)
- `scripts/run-rekordbox-evals.sh` -> pass

## Recommended Next Steps

### P0: Merge Hygiene and Contract Guardrails

1. Review and decide JSON contract policy for `write_xml` no-change path in `src/tools.rs`.
   - Current behavior returns JSON with provenance fields.
   - Previous behavior returned plain text.
   - If strict backwards compatibility is required, restore plain text or add explicit compatibility note in changelog/release notes.
2. Split and commit changes into small PR-ready commits (Conventional Commits).
3. Open CI run on GitHub and verify `.github/workflows/corpus-ci.yml` behavior in real runners.

### P1: Hardening

1. Add router-level integration tests that call tool names through rmcp router (not only direct method calls), focused on:
   - XML no-change response shape/provenance
   - genre update/suggestion provenance
2. Run ignored integration tests with real backup when available:
   - `cargo test -- --ignored`
3. Decide whether to keep or remove dead-code warnings in `src/corpus.rs`:
   - keep as-is, or
   - consume currently-unused manifest fields in reporting, or
   - add targeted allow attributes if deliberate.

### P2: Optional Corpus Cleanup

1. Execute Wave 6 (FAQ dedup) only if requested:
   - detect near-duplicates
   - apply provenance-safe edits
   - re-run corpus validation and verification

## Suggested Commit Stack

1. `feat(retrieval): add manifest-first corpus lookup and ranking`
2. `feat(workflows): add corpus provenance to xml and genre tool responses`
3. `ci(corpus): add consolidated corpus validation and verification workflow`
4. `test(eval): add routing and task-success eval suites with harness`
5. `docs(orchestration): update orchestration tracker and provenance guidance`

## Operator Checklist

1. `git status --short` (verify only intended files are staged)
2. `cargo test`
3. `scripts/run-rekordbox-evals.sh`
4. `bash docs/rekordbox/validate-corpus.sh`
5. `python3 docs/rekordbox/verify-phase-b.py`
6. Stage/commit in planned split
7. Push and validate GitHub Actions run

## Notes for Next Operator

- During earlier wave execution, an out-of-scope edit incident occurred from worker activity; unrelated formatting changes were audited and reverted. Keep watching `git status` for scope drift.
- `docs/prompts/` remains untracked in the working tree and was intentionally left untouched.
