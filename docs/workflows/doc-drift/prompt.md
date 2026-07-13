You are auditing documentation for drift against the actual codebase. Start
with the automated contract, then investigate semantic accuracy and user
clarity that the structural checker cannot prove.

## Phase 0: Run the automated contract

Run these checks before launching the narrative audit:

```sh
cargo build --release
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
```

Fix structural failures at their canonical boundary. Do not copy the live MCP
schema, CLI inventory, or workflow order into another snapshot. A green result
proves marked public surfaces and built links agree with code; it does not prove
that descriptions, examples, risks, or workflows are correct.

## Scope

The documentation surfaces to audit are:

1. **Site docs** (`site/src/content/docs/`) — MCP tool reference pages, CLI
   docs, workflow guides, concept pages, getting-started guides
2. **Embedded SOPs** (`site/src/partials/sops/*.mdx`) — these are
   `include_str!`'d into the binary via `src/mcp/help.rs`
3. **Tool schemas** — `#[tool(description = "...")]` annotations and schemars
   `#[schemars(description = "...")]` on parameter structs in `src/mcp/`
4. **CLI help text** — clap `#[command]` and `#[arg]` annotations in
   `src/cli/`
5. **README.md** — project root

## Phase 1: Extract semantic ground truth from code

Launch subagents in parallel to extract the actual state from the codebase:

**Subagent A — MCP behavior:**
Trace handlers behind the documented tools. Focus on selection precedence,
side effects, persistence, defaults applied outside JSON Schema, validation,
failure behavior, and output meaning. The automated checker already owns the
marked name/type inventory.

**Subagent B — CLI behavior:**
Read `src/cli/` to verify workflow behavior, prompts, exit status, cache and
retry semantics, and operational advice beyond the machine-checked help
inventory.

**Subagent C — SOP semantics:**
Read each file in `site/src/partials/sops/` and check that sequence, expected
behavior, stated defaults, constraints, recovery guidance, and safety advice
match the handlers. Recognized tool calls and named top-level arguments are
already checked automatically.

**Subagent D — user journey:**
Walk the site docs as a new user. Check onboarding, discovery, prerequisites,
cross-links, terminology, examples, and whether the next safe action is clear.
Verify current external Rekordbox UI instructions separately.

## Phase 2: Cross-reference

With all four inventories in hand, systematically check for drift:

### MCP behavior and descriptions
- Descriptions should match actual handler behavior and output meaning.
- Defaults applied in handlers should agree with the docs even when the live
  schema cannot expose them.
- Selection precedence, cache semantics, side effects, and failure behavior
  should be explicit where they affect safe use.

### CLI behavior
- Prompts, confirmation bypasses, retries, progress, exit status, and partial
  completion should match the implementation.
- Examples should remain executable and operationally safe.

### SOP accuracy
- Behavior described in SOPs must match what the handler code does
- Stated constraints (e.g., "requires cache coverage first") must match
  actual runtime checks
- Risks, checkpoints, recovery paths, and handoff to the next workflow should
  be clear to a first-time user

### README accuracy
- Feature claims should reflect current capabilities
- Install instructions should match the release workflow
- Example commands should work

### Stale content
- References to removed tools, renamed parameters, or old defaults
- Tool descriptions that describe pre-refactor behavior
- Screenshots or examples using obsolete output formats

### Automated-gate extension review
- If workflow continuation fields exist, extend checks by consuming them from
  `site/src/data/workflows.mjs`; never repeat the values in the checker.
- If generated audience outputs exist, validate their built files and links
  through `check-doc-contract` without creating a hand-maintained output list.

## Phase 3: Report and fix

For each finding, categorize as:

- **Incorrect** — states something that contradicts the code. Must fix.
- **Stale** — refers to something that no longer exists. Must fix.
- **Incomplete** — code has capabilities not documented. Fix if the
  omission would mislead a user; otherwise note for follow-up.
- **Minor** — wording differences that don't cause confusion. Skip.

Present the categorized findings to the user and wait for confirmation. Then
implement fixes, prioritizing Incorrect and Stale items. Re-run the automated
contract and site build after changes; also rebuild the Rust binary when an
embedded SOP changed.

Draft a conventional commit message and present it for approval.
