You are orchestrating a deep audit of a codebase subsystem. The user will
specify which subsystem to audit after this prompt. Your job is to drive
the entire process from discovery through implementation using subagents,
working in sequential phases.

## Phase 1: Discovery

Launch an Explore subagent to map the subsystem. It should find all
relevant modules, trace the architecture, and identify the key files,
data flows, and external boundaries. Use the subsystem description the
user provided to seed the search but explore broadly — don't limit to
only the named files or keywords.

Read the agent's findings carefully. If the subsystem is large or has
clearly separable concerns, note how to split the audit work.

## Phase 2: Audit

Based on the discovery results, launch audit subagents to examine the
code for bugs, logic errors, security issues, edge cases, and fragile
patterns. Each subagent should:

- Focus on a specific concern area (e.g., "SQL injection surface",
  "concurrency and locking", "error handling and recovery")
- Read the actual source code, not just search for patterns
- Cite exact file paths and line numbers for every finding
- Verify each finding is real before reporting — no false positives

Split the work across multiple subagents if the subsystem is large
enough to benefit. Run them in parallel when they examine independent
files. Run them sequentially when one area's findings inform another.

Take your time here. Thoroughness matters more than speed.

## Phase 3: Triage

Collect all findings from the audit subagents. For each finding,
categorize it:

- **Must-fix**: Bugs, security issues, data corruption risks, correctness
  errors. These get fixed in this session.
- **Should-fix now**: Logic errors, fragile patterns, missing validation
  that could cause real problems. These get fixed in this session unless
  there is a concrete reason to defer (e.g., requires design discussion,
  touches too many callers, needs input from the user).
- **Defer**: Code quality improvements, minor edge cases, style issues.
  Note these for follow-up but do not implement them.

If any finding is ambiguous — you're not sure if it's a real issue or
a false positive — launch a subagent to investigate further before
categorizing it. Do not guess. The triage must be accurate.

Present the categorized list to the user and wait for confirmation
before proceeding. State clearly what you plan to fix and what you
plan to defer.

## Phase 4: Implement

Implement all must-fix and should-fix-now items. Work through them in
an order that keeps the codebase buildable after each change. If fixes
interact or share files, group them.

After completing the implementation, build the project to verify it
compiles.

## Phase 5: Review

Launch these review subagents in parallel on the changes you just made:

- **pr-review-toolkit:code-reviewer** — correctness, regressions,
  adherence to project conventions
- **pr-review-toolkit:silent-failure-hunter** — swallowed errors,
  inadequate error handling, bad fallback behavior
- **pr-review-toolkit:pr-test-analyzer** — test coverage gaps for
  the new or modified code

Fix any issues they surface. Then run the test suite. If tests fail,
fix and re-run until clean.

## Phase 6: Report

Present a summary:

- What was found (total findings by category)
- What was fixed (with brief descriptions)
- What was deferred (with reasons, for follow-up sessions)
- Test results

Draft a conventional commit message (`type(scope): imperative summary`)
that accurately reflects the changes made. Present it to the user for
approval, then commit when confirmed.
