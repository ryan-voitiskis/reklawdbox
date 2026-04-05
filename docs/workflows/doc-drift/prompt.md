You are auditing documentation for drift against the actual codebase. Your
goal is to find every place where documentation describes something that
doesn't match what the code actually does, and fix it.

## Scope

The documentation surfaces to audit are:

1. **Site docs** (`site/src/content/docs/`) — MCP tool reference pages, CLI
   docs, workflow guides, concept pages, getting-started guides
2. **Embedded SOPs** (`site/src/partials/sops/*.mdx`) — these are
   `include_str!`'d into the binary via `src/tools/help_handler.rs`
3. **Tool schemas** — `#[tool(description = "...")]` annotations and schemars
   `#[schemars(description = "...")]` on parameter structs in `src/tools/`
4. **CLI help text** — clap `#[command]` and `#[arg]` annotations in
   `src/cli/`
5. **README.md** — project root

## Phase 1: Extract ground truth from code

Launch subagents in parallel to extract the actual state from the codebase:

**Subagent A — MCP tool inventory:**
Read `src/tools/mod.rs` to get the full list of `#[tool(...)]` declarations.
For each tool, extract: tool name, description, and the handler function it
delegates to. Then read each handler's parameter struct to get parameter
names, types, defaults, and schemars descriptions. Output a structured list.

**Subagent B — CLI inventory:**
Read `src/cli/` to extract all subcommands, their arguments, flags,
defaults, and help text from clap derive annotations.

**Subagent C — SOP inventory:**
Read each file in `site/src/partials/sops/` and extract: tool names
referenced, parameter names referenced, expected tool behavior described,
and any stated defaults or constraints.

**Subagent D — Site docs inventory:**
Read each file in `site/src/content/docs/` and extract the same: tool names,
parameter names, described behavior, stated defaults, CLI commands referenced.

## Phase 2: Cross-reference

With all four inventories in hand, systematically check for drift:

### Tools coverage
- Every MCP tool in the code should appear in the site docs tool reference
- Every tool's documented parameters should match the actual parameter struct
- Parameter descriptions in site docs should match schemars descriptions
- Documented defaults should match actual defaults in code

### CLI coverage
- Every CLI subcommand should appear in the CLI docs page
- Documented flags and arguments should match clap annotations
- Documented defaults should match actual defaults

### SOP accuracy
- Tool names referenced in SOPs must exist
- Parameter names referenced in SOPs must exist on the stated tool
- Behavior described in SOPs must match what the handler code does
- Stated constraints (e.g., "requires cache coverage first") must match
  actual runtime checks

### README accuracy
- Feature claims should reflect current capabilities
- Install instructions should match the release workflow
- Example commands should work

### Stale content
- References to removed tools, renamed parameters, or old defaults
- Tool descriptions that describe pre-refactor behavior
- Screenshots or examples using obsolete output formats

## Phase 3: Report and fix

For each finding, categorize as:

- **Incorrect** — states something that contradicts the code. Must fix.
- **Stale** — refers to something that no longer exists. Must fix.
- **Incomplete** — code has capabilities not documented. Fix if the
  omission would mislead a user; otherwise note for follow-up.
- **Minor** — wording differences that don't cause confusion. Skip.

Present the categorized findings to the user and wait for confirmation.
Then implement fixes, prioritizing Incorrect and Stale items. Build the
site (`cd site && npm run build`) to verify no broken links or build
errors after changes.

Draft a conventional commit message and present it for approval.
