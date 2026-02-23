# Rekordbox Software Orchestration Plan

## Scope

Production-grade corpus integration for `reklawdbox`:

1. Manifest-first corpus retrieval integrated into MCP tool flows.
2. Corpus-backed XML and genre-tagging workflows.
3. CI automation for corpus validation/verification.
4. Evaluation suite for routing and task-success quality.
5. Optional FAQ dedup pass.

## Baseline Context

- Corpus root: `docs/rekordbox/`
- Entry points: `docs/rekordbox/manifest.yaml`, `docs/rekordbox/README.md`
- Reference anchors:
  - `docs/rekordbox/reference/xml-import-export.md`
  - `docs/rekordbox/reference/glossary.md`
  - `docs/rekordbox/reference/developer-integration.md`
- Validation scripts:
  - `bash docs/rekordbox/validate-corpus.sh`
  - `python3 docs/rekordbox/verify-phase-b.py`

## CI/Test Command Baseline

- `cargo test`
- `cargo test -- --ignored`
- `cargo build --release`
- `bash docs/rekordbox/validate-corpus.sh`
- `python3 docs/rekordbox/verify-phase-b.py`

No existing GitHub Actions workflow files were found in `.github/workflows/` at orchestration start.

## Wave Status

<!-- dprint-ignore -->
| Wave | Name | Status | Gate |
|---|---|---|---|
| Wave 0 | Setup (orchestrator) | done | Tracker exists with full wave map + ownership |
| Wave 1 | Discovery & Design | done | Merged design note with concrete edits, acceptance criteria, risks |
| Wave 2 | Retrieval Integration | done | Retrieval tests pass + no regressions |
| Wave 3 | Corpus-Backed Workflows | done | XML + genre paths are corpus-informed + provenance exposed |
| Wave 4 | CI Automation | done | CI config valid + corpus jobs runnable |
| Wave 5 | Evaluation Suite | done | Eval suite executable with explicit thresholds |
| Wave 6 | FAQ Dedup (optional) | pending | No validator regressions + auditable dedup changes |

## Subagent Assignment Table

<!-- dprint-ignore -->
| ID | Wave | Type | Owner Scope | Deliverable |
|---|---|---|---|---|
| D1 | Wave 1 | explorer | MCP integration map | file-level routing/response integration points |
| D2 | Wave 1 | explorer | XML + genre workflow map | touch points + data-flow notes |
| D3 | Wave 1 | explorer | CI map | workflow files/sections to patch |
| D4 | Wave 1 | explorer | Test/eval map | proposed eval test files + command targets |
| R1 | Wave 2 | worker | Manifest retrieval core | filtered retrieval + ranking/sort + tests |
| R2 | Wave 2 | worker | MCP integration wiring | handlers return corpus-backed outputs + provenance |
| R3 | Wave 2 | worker | Robustness | cache/index init, error/fallback paths, tests |
| W1 | Wave 3 | worker | XML workflow | corpus-guided XML behavior |
| W2 | Wave 3 | worker | Genre workflow | corpus-guided genre mapping/decisions |
| W3 | Wave 3 | worker | Provenance output | source paths exposed in workflow responses |
| C1 | Wave 4 | worker | CI validator job | add `validate-corpus.sh` CI step |
| C2 | Wave 4 | worker | CI verifier job | add `verify-phase-b.py` CI step |
| C3 | Wave 4 | worker | CI polish | fail-fast, logs/artifacts for corpus checks |
| E1 | Wave 5 | worker | Routing evals | prompt->doc routing tests |
| E2 | Wave 5 | worker | Task-success evals | workflow outcome tests |
| E3 | Wave 5 | worker | Eval harness | command/script + threshold enforcement |
| F1 | Wave 6 | worker | FAQ duplicate detection | duplicate report |
| F2 | Wave 6 | worker | FAQ dedup edits | safe dedup patch |
| F3 | Wave 6 | worker | Post-dedup validation | validation rerun + manifest update if needed |

## Wave Execution Map

### Wave 0 - Setup

- Read `AGENTS.md`, host guide (`CLAUDE.md` when applicable), corpus README + manifest.
- Confirm CI/test command baseline.
- Create this tracker.

### Wave 1 - Discovery & Design

- Run D1-D4 in parallel.
- Merge discovery into this tracker with:
  - concrete file edits
  - acceptance criteria per workstream
  - risk list

### Wave 1 Design Note (Merged)

#### Discovery Summary

- D1 (`MCP routing`): tool call routing and response construction are centralized in `src/tools.rs` with server bootstrap in `src/main.rs`. Integration should happen at tool-handler layer so all corpus-backed responses are generated in one place.
- D2 (`XML + genre map`): XML export path is `update_tracks`/`write_xml` in `src/tools.rs` -> `changes.rs` + `db.rs` -> `xml.rs`. Genre path is `suggest_normalizations`, `update_tracks`, and taxonomy helpers in `src/genre.rs`.
- D3 (`CI map`): no `.github/workflows/` currently exists. CI automation must be added as new workflow files.
- D4 (`test/eval map`): current tests cover low-level modules but not MCP routing in `src/tools.rs`; evaluation should be added under `tests/` with focused routing/task-success suites.

#### Concrete File Edits (Planned)

- Retrieval core:
  - Add `src/corpus.rs` for manifest loading, filtering (`topic`/`mode`/`type`), scoring/ranking, stable sorting, and memoized index initialization.
  - Update `src/main.rs` and `src/tools.rs` to initialize/use corpus retrieval state.
  - Update `Cargo.toml` for YAML parsing dependency (`serde_yaml`).
- MCP integration and provenance:
  - Update `src/tools.rs` handlers to consult retrieval results where corpus guidance is needed and include consulted doc paths in outputs.
  - Add shared response helper(s) in `src/tools.rs` to keep provenance shape consistent.
- Workflow integration:
  - Update XML-related handlers in `src/tools.rs` to consult:
    - `docs/rekordbox/reference/xml-import-export.md`
    - `docs/rekordbox/guides/xml-format-spec.md`
    - `docs/rekordbox/reference/developer-integration.md`
  - Update genre-normalization workflows (`suggest_normalizations`, related update paths) to include corpus topic mapping and provenance.
- Tests/evals:
  - Add retrieval unit tests in `src/corpus.rs` (or `tests/corpus_retrieval.rs`).
  - Add routing/task-success suites in `tests/tool_routing.rs` and `tests/task_eval.rs`.
  - Add eval harness script (`scripts/run-rekordbox-evals.sh`) and reporting thresholds.
- CI:
  - Add `.github/workflows/corpus-validation.yml` for corpus validate/verify commands plus `cargo test`.
  - Add fail-fast/log grouping and upload artifacts for corpus check output.

#### Acceptance Criteria By Workstream

- Retrieval (`R*`):
  - Manifest-first retrieval returns deterministic, stable-ranked docs for identical input.
  - Filtering by `topic`/`mode`/`type` is correct and covered by tests.
  - Failure modes (missing manifest, malformed YAML) return safe fallback behavior.
- Workflows (`W*`):
  - XML and genre responses are visibly corpus-informed and include `consulted_documents`.
  - XML guidance always includes reference docs above for XML operations.
  - Existing XML write/export behavior remains intact.
- CI (`C*`):
  - GitHub Actions runs `bash docs/rekordbox/validate-corpus.sh` and `python3 docs/rekordbox/verify-phase-b.py`.
  - Workflow syntax validates and jobs are isolated/readable in logs.
- Evaluation (`E*`):
  - Routing evals assert expected doc paths for representative prompts.
  - Task-success evals cover UI understanding, XML import/export, library management, USB export, preferences/settings.
  - Thresholds are explicit and fail the run when unmet.

#### Risks

- Corpus coupling risk: hardcoded doc-path assumptions can drift if corpus layout changes; mitigate with manifest-driven lookups and tests.
- Output shape risk: adding provenance fields can break existing consumers if response format changes unexpectedly; mitigate with additive fields only.
- CI runtime risk: adding full test suite to new workflow may increase execution time; mitigate with staged jobs and fail-fast.
- Flaky eval risk: prompt-like tests can become brittle; mitigate with deterministic fixtures and path-based assertions.
- Integration-test environment risk: ignored real-DB tests depend on external backup availability.

### Wave 2 - Retrieval Integration

- Run R1-R3 in parallel, then reconcile and test.

#### Wave 2 Progress Notes (R3 - Robustness)

- 2026-02-17: documented expected robustness behavior in `docs/rekordbox/README.md` for manifest index initialization, cache behavior, and unavailable/malformed-manifest fallback behavior.
- 2026-02-17: clarified fallback expectation to continue best-effort guidance (priority consultation order + XML anchors) instead of request hard-failure.
- 2026-02-17: orchestrator reconciled unintended out-of-scope Rust edits from worker execution and restored unrelated files to `HEAD`.
- 2026-02-17: Wave 2 gate checks passed (`cargo test corpus`, `cargo test`).
- 2026-02-17: Wave 2 marked `done`.

### Wave 3 - Corpus-Backed Workflows

- Run W1-W3 in parallel, then reconcile and test.

#### Wave 3 Progress Notes (W1/W2/W3)

- 2026-02-17: W1 updated `src/tools.rs` to include provenance on XML no-change path and added task-level tests for XML + genre response provenance.
- 2026-02-17: W2 updated tracker checkpoints and provisional Wave 3 gate checklist.
- 2026-02-17: W3 updated `docs/rekordbox/reference/developer-integration.md` with workflow response provenance fields and fallback semantics.
- 2026-02-17: orchestrator reconciled Wave 3 outputs and validated with `cargo test tools::tests::` and full `cargo test`.

#### Wave 3 Provisional Gate Checks (For Reconciliation)

- [x] XML workflow outputs are corpus-informed for XML operations and include XML-reference-grounded guidance.
- [x] Genre workflow outputs are corpus-informed for normalization/mapping decisions.
- [x] Provenance is exposed in workflow responses via additive consulted-document metadata (for example `consulted_documents`).
- [x] Corpus-informed behavior is manifest-driven (no hardcoded path-only coupling) and preserves existing XML write/export behavior.

#### Wave 3 Assumptions and Open Questions

- Assumption: Wave 2 provenance shape (`consulted_documents`-style additive field) remains the compatibility target for Wave 3 responses.
- Assumption: Manifest entries for XML and genre reference docs remain stable through Wave 3 reconciliation.
- Open question (non-blocking): Should genre responses expose only top-ranked consulted docs or the full consulted set when rankings tie closely?
- Open question (non-blocking): Should fallback/error-path responses expose consulted-document provenance whenever corpus lookup partially succeeds?
- Critical blockers: none currently identified.

### Wave 4 - CI Automation

- Run C1-C3 in parallel.

#### Wave 4 Progress Notes (C1/C2/C3)

- 2026-02-17: C1/C2/C3 produced CI workflow drafts for corpus validation and verification.
- 2026-02-17: orchestrator reconciled to a single consolidated workflow: `.github/workflows/corpus-ci.yml`.
- 2026-02-17: local dry-run commands passed:
  - `bash docs/rekordbox/validate-corpus.sh`
  - `python3 docs/rekordbox/verify-phase-b.py`
  - `cargo test`
- 2026-02-17: Wave 4 marked `done`.

### Wave 5 - Evaluation Suite

- Run E1-E3 in parallel.

#### Wave 5 Progress Notes (E1/E2/E3)

- 2026-02-17: E1 added routing eval suite in `src/eval_routing.rs` with prompt-to-expected-doc cases and explicit thresholds.
- 2026-02-17: E2 added task-success eval suite in `src/eval_tasks.rs` covering UI understanding, XML import/export, library management, USB export, and preferences/settings.
- 2026-02-17: E3 added eval harness script `scripts/run-rekordbox-evals.sh` and wired modules in `src/main.rs`.
- 2026-02-17: orchestrator gate checks passed:
  - `scripts/run-rekordbox-evals.sh`
  - `cargo test`
- 2026-02-17: Wave 5 marked `done`.

### Wave 6 - FAQ Dedup (optional)

- Run only if time permits or explicitly requested.

## Blocking Issues and Decisions

- Wave 1 D2 initial subagent prompt failed policy filtering. Decision: retry with narrower technical scope; retry succeeded and no functionality impact.
- Wave 2 R3 scope was documentation-only. Decision: rely on orchestrator gate testing (`cargo test corpus`, `cargo test`) to close runtime confirmation after R1/R2 integration.
- Wave 2 worker side effect: unrelated Rust files were modified during worker execution. Decision: audit diffs, preserve only planned Wave 2 files, restore unrelated files to `HEAD`.
- Wave 4 produced overlapping workflow files. Decision: keep one consolidated workflow (`.github/workflows/corpus-ci.yml`) and remove redundant per-job duplicates to avoid duplicate CI runs.

## Deliverables Log

- Added tracker: `docs/rekordbox/SOFTWARE-ORCHESTRATION-PLAN.md`.
- Wave 1 discovery complete via D1/D2/D3/D4 and merged into this plan.
- Wave 2 R3 documentation pass complete for robustness behavior and fallback expectations.
- Wave 2 retrieval integration complete (`src/corpus.rs`, `src/tools.rs`, `src/main.rs`, `Cargo.toml`, `Cargo.lock`) with tests passing.
- Wave 3 corpus-backed workflow integration and provenance verification complete.
- Wave 4 CI automation complete with consolidated workflow in `.github/workflows/corpus-ci.yml`.
- Wave 5 evaluation suite complete in `src/eval_routing.rs`, `src/eval_tasks.rs`, and `scripts/run-rekordbox-evals.sh`.
- Wave 6 FAQ dedup intentionally deferred (optional, non-blocking).
- Added operator handoff: `docs/rekordbox/SOFTWARE-ORCHESTRATION-HANDOVER.md`.
