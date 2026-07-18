# Plan 036: Standardize the managed Essentia environment

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 4aad526..HEAD -- \
>   src/adapters/audio \
>   src/application/analysis \
>   src/cli/setup.rs \
>   src/mcp/analysis \
>   src/mcp/context.rs \
>   src/mcp/server.rs \
>   src/mcp/tests/analysis.rs \
>   AGENTS.md README.md \
>   site/src/content/docs/cli/index.mdx \
>   site/src/content/docs/concepts/architecture.mdx \
>   site/src/content/docs/getting-started/index.mdx \
>   site/src/content/docs/mcp-tools/enrichment-analysis.mdx \
>   site/src/content/docs/reference/environment-variables.md \
>   site/src/content/docs/troubleshooting/index.mdx
> ```
>
> If an in-scope source file changed, compare the live installer, probe,
> analyzer-version, and documentation contracts with the excerpts below. If
> installation is already centralized, the managed path changed, or the pinned
> package decision has been superseded, STOP and report the drift.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: 035
- **Category**: migration / DX / analysis reproducibility
- **Planned at**: commit `4aad526`, 2026-07-17

### Execution history

- **2026-07-18 — COMPLETED on `main`.** Commits `a05a6d4` and `3e34227`
  centralized CLI/MCP setup behind one transactional managed-environment
  workflow, pinned and validated the exact CPython 3.14.6 / Essentia
  2.1b6.dev1438 runtime manifest, and moved the analyzer identity to Essentia
  schema v3. The managed import and ignored real-audio round trip passed. The
  ignored `.mcp.json` override was removed and the broken repository-local
  environment was retired only after validation. Independent code and public-
  contract reviews approved the result; all workspace, release, MCP, site,
  documentation-contract, and semantic documentation gates passed.
- **2026-07-17 — BLOCKED after two executor revision rounds.** The isolated
  worktree `/Users/vz/.codex/worktrees/reklawdbox-036` contains an uncommitted,
  unapproved partial implementation. Exact-manifest discovery, a kernel
  `flock`, focused discovery/MCP tests, and parts of the transaction exist, but
  the executor did not implement the injected installer seam or mandatory
  offline failure/switch/rollback/pruning matrix. Full workspace, release,
  MCP, docs, real-audio, and operator-migration gates were not completed. Do
  not merge or build Plan 037 on that partial tree.

## Why this matters

Reklawdbox currently has one canonical managed Essentia path but two separate
installers and an older repository-local environment still referenced by the
local Claude Code configuration. The stale environment was created with
Homebrew Python 3.13.12; its interpreter symlink now points at a removed
Homebrew installation. A path can therefore appear in `.mcp.json` while failing
the actual Essentia import probe.

The CLI and MCP installers also independently run floating
`pip install --pre essentia`. Two machines can install different Essentia
builds while writing cache rows under the same `ESSENTIA_SCHEMA_VERSION = "2"`.
That is unsafe once Essentia becomes required evidence for full classification:
an upstream analyzer change must not silently reuse old cache rows or compatible
profile metadata.

The MCP's in-process lock does not serialize a concurrent CLI setup or another
MCP process, and rebuilding the canonical venv in place would turn a transient
pip/wheel failure into an outage. The shared installer therefore also needs an
interprocess lock and a validate-before-switch generation model with rollback.

This plan establishes one managed environment, one shared installer, and one
versioned analyzer contract. It preserves the expert
`CRATE_DIG_ESSENTIA_PYTHON` override, but standard setup and documentation must
not create or reference a venv inside the repository. It also preserves the
architectural boundary that Reklawdbox can start and provide non-classification
features without Python.

## Current state

### Discovery already has a canonical managed path

`src/adapters/audio/essentia.rs:166-196` probes an explicit override and then
the user-managed default:

```rust
pub(crate) fn probe_essentia_python_path() -> Option<String> {
    let env_override = std::env::var(ESSENTIA_PYTHON_ENV_VAR).ok();
    let default_candidate =
        dirs::home_dir().map(|home| home.join(".local/share/reklawdbox/essentia-venv/bin/python"));
    probe_essentia_python_from_sources(env_override.as_deref(), default_candidate)
}

pub(crate) const ESSENTIA_VENV_RELPATH: &str =
    ".local/share/reklawdbox/essentia-venv";
```

The override remains useful for development and unsupported installations.
The managed path must remain the zero-configuration default and the only
user-facing interpreter path; validated generation storage beneath the same
Reklawdbox-owned data directory remains an internal installer detail.

### Validation accepts any version-looking output

`src/adapters/audio/essentia.rs:95-164` executes the import check and accepts
any non-empty line containing a digit:

```rust
let version_line = stdout.lines().map(str::trim).find(|line| !line.is_empty());
matches!(
    version_line,
    Some(line) if line.chars().any(|ch| ch.is_ascii_digit())
)
```

This proves that some Essentia imports, not that it matches the analyzer
runtime Reklawdbox calibrated and cached against. Replace the boolean-only
internal contract with typed runtime information containing the interpreter
path, Python version, exact package manifest, and a stable analyzer-contract
identifier. Keep a small boolean wrapper only where an existing caller
genuinely needs it.

### CLI and MCP duplicate installation policy

`src/cli/setup.rs:9-16,49-97` owns one Python candidate list and runs:

```rust
run_cmd(
    venv_pip.to_string_lossy().as_ref(),
    &["install", "--pre", "essentia"],
    "pip install essentia",
)?;
```

`src/mcp/analysis/handlers.rs:1086-1224` owns a second candidate list, creates
the same venv, independently invokes `pip install --pre essentia`, validates
the import, and then sets a process-local override. The MCP lock at
`handlers.rs:1051-1052` correctly serializes concurrent setup calls and must be
preserved.

Reusable setup behavior belongs in `application/analysis`; subprocess and
filesystem primitives belong in `adapters/audio`. CLI and MCP should translate
one shared result into their own presentation rather than maintain separate
installation algorithms.

### Cache compatibility does not pin the upstream analyzer

`src/adapters/audio/mod.rs:41-44` currently declares:

```rust
pub(crate) const STRATUM_SCHEMA_VERSION: &str = "21";
pub(crate) const ESSENTIA_SCHEMA_VERSION: &str = "2";
```

`src/adapters/audio/essentia_analysis.py:9` records
`essentia.__version__` inside the payload, but cache freshness checks only the
Rust schema constant. A newly pinned upstream analyzer is a semantic cache
change even when the JSON field names stay identical.

### Local machine migration to perform after code verification

At planning time on 2026-07-17:

- `.venvs/essentia/bin/python` is a broken symlink chain ending at the missing
  `/opt/homebrew/opt/python@3.13/bin/python3.13`;
- `~/.local/share/reklawdbox/essentia-venv/bin/python` does not exist;
- `/opt/homebrew/bin/python3` is CPython 3.14.6; and
- the ignored local `.mcp.json` explicitly sets
  `CRATE_DIG_ESSENTIA_PYTHON` to the repository-local broken path.

Both `.mcp.json` and `.venvs/` are ignored by `.gitignore`; they are local
operator state, not files to commit. The source change and the local cleanup
must be verified separately.

### Upstream package contract selected for this plan

As of 2026-07-17, PyPI publishes `essentia 2.1b6.dev1438` as the current
pre-release. Its published wheels include CPython 3.14 for macOS 15+ ARM64,
macOS 15+ x86_64, and manylinux x86_64. That matches this repository's stated
primary target (macOS on Apple Silicon) and the local machine. Its package
metadata leaves `numpy>=1.25`, `pyyaml`, and `six` unpinned, so pinning only the
top-level wheel would not make the environment reproducible. Verify the
current metadata and file list at
<https://pypi.org/pypi/essentia/2.1b6.dev1438/json> before implementation; the
comparison build is documented at
<https://pypi.org/project/essentia/2.1b6.dev1389/>.

Pin this plan to:

```text
essentia==2.1b6.dev1438
numpy==2.5.1
PyYAML==6.0.3
six==1.17.0
CPython 3.14
binary wheels only
```

The selected dependency versions publish compatible CPython 3.14/macOS ARM64
wheels as of the planning date. Treat the four package versions as one runtime
manifest. Before implementation, verify that the exact set still resolves
wheel-only and passes the repository's real-audio test; if not, STOP rather
than selecting a different dependency version silently.

Do not silently substitute `2.1b6.dev1389`: that older build has broader
CPython 3.9-3.13 wheels but is a different analyzer. Do not silently broaden
platform support through an unreviewed source build. If the maintainer wants a
multi-version compatibility matrix instead of one reproducible analyzer, STOP
and redesign cache identity before implementing.

Essentia is published under AGPL-3.0-only while Reklawdbox is MIT. Preserve the
existing separate-process, user-installed boundary: do not vendor the wheel,
link Essentia into the Rust release, or redistribute it inside the release
archive. Public setup documentation should disclose the external package and
license without making unsupported legal claims.

## Commands you will need

| Purpose                   | Command                                                                                                                                                                                             | Expected on success                                                             |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Focused environment tests | `cargo test -p reklawdbox essentia_environment -- --nocapture`                                                                                                                                      | exit 0; path, version, candidate, and failure-policy tests pass without network |
| MCP setup tests           | `cargo test -p reklawdbox setup_essentia -- --nocapture`                                                                                                                                            | exit 0; shared setup result is presented and activated correctly                |
| Audio cache tests         | `cargo test -p reklawdbox adapters::state -- --nocapture`                                                                                                                                           | exit 0; old Essentia cache/profile metadata is stale but preserved              |
| Duplicate-policy check    | `! rg -n -e PYTHON_CANDIDATES -e python_candidates -e 'pip install.*essentia' -e 'install.*--pre.*essentia' src/cli src/mcp`                                                                        | exit 0; transports contain no installer policy                                  |
| Repo-local path check     | `! rg -n "\.venvs/essentia" AGENTS.md README.md CONTRIBUTING.md src site scripts`                                                                                                                   | exit 0; committed surfaces do not prescribe the retired venv                    |
| Format                    | `cargo fmt --check && dprint check`                                                                                                                                                                 | exit 0                                                                          |
| Lint                      | `cargo clippy --workspace --all-targets -- -D warnings`                                                                                                                                             | exit 0                                                                          |
| Tests                     | `cargo test --workspace --no-fail-fast`                                                                                                                                                             | exit 0                                                                          |
| Release and smoke         | `cargo build --release && ./target/release/reklawdbox --version && ./target/release/reklawdbox --help && node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000` | exit 0; no MCP protocol violations                                              |
| Public docs               | commands in `docs/workflows/doc-drift/README.md`                                                                                                                                                    | parser tests, site build, live contract, and semantic review pass               |

## Scope

**In scope — committed source and documentation**:

- `src/adapters/audio/essentia.rs`
- `src/adapters/audio/essentia_environment.rs` (new; low-level filesystem and
  subprocess adapter)
- `src/adapters/audio/essentia_analysis.py`
- `src/adapters/audio/mod.rs`
- `src/adapters/audio/tests.rs`
- `src/application/analysis/setup.rs` (new; reusable setup policy/workflow)
- `src/application/analysis/mod.rs`
- `src/cli/setup.rs`
- `src/mcp/analysis/handlers.rs`
- `src/mcp/context.rs`
- `src/mcp/server.rs`
- `src/mcp/tests/analysis.rs`
- `AGENTS.md`
- `README.md`
- `site/src/content/docs/cli/index.mdx`
- `site/src/content/docs/concepts/architecture.mdx`
- `site/src/content/docs/getting-started/index.mdx`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `site/src/content/docs/reference/environment-variables.md`
- `site/src/content/docs/troubleshooting/index.mdx`
- `plans/README.md` for the status row only

**In scope — ignored local operator state, never stage or commit**:

- `.mcp.json` — remove the repository-local
  `CRATE_DIG_ESSENTIA_PYTHON` override after managed setup validates
- `.venvs/essentia/` — delete only after the managed environment passes import
  and real-audio smoke checks

**Out of scope**:

- Making the MCP server or unrelated library/metadata/export tools refuse to
  start without Essentia; Plan 037 owns classification semantics.
- Changing genre taxonomy, classifier weights, profile scoring, transition
  scoring, or pool scoring.
- Bundling Python, Essentia, or an Essentia wheel in the release tarball.
- Supporting an upstream Essentia source build, Windows, Linux ARM, or a
  multi-version analyzer matrix without a separate reviewed design.
- Deleting old cache rows or profile rows. Cache/profile version checks must
  make them inert while preserving rollback and diagnostics.
- Modifying Rekordbox `master.db`, audio tags, or audio files.
- Deploying the Homebrew binary, pushing, opening a PR, or releasing.

## Git workflow

- Branch: `codex/036-managed-essentia-environment`
- Use Conventional Commits; preferred final message:
  `refactor(analysis): standardize managed Essentia runtime`.
- Stage only committed source/docs. Never stage `.mcp.json`, `.venvs/`, cache
  databases, or private audio.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Characterize managed discovery and exact runtime validation

Before refactoring, add deterministic tests around the adapter boundary. Use
temporary directories and fake executable scripts, following
`src/mcp/tests/analysis.rs:184-262`; never invoke pip or the network in a
mandatory test.

Cover these cases:

1. a managed interpreter returning the exact Python and four-package manifest
   is accepted;
2. an otherwise valid interpreter with Essentia `2.1b6.dev1389`, or with one
   mismatched transitive dependency, is reported as unsupported rather than
   current;
3. a broken explicit override falls through to a valid managed interpreter;
4. a valid explicit override remains available as an expert escape hatch but
   its version must still match the supported analyzer contract;
5. missing, non-numeric, timed-out, and non-zero import probes fail closed; and
6. the default path is always under
   `~/.local/share/reklawdbox/essentia-venv`, never the repository.

Introduce a typed internal value such as `EssentiaRuntime` containing at least
`python_path`, `python_version`, `essentia_version`, `numpy_version`,
`pyyaml_version`, `six_version`, and `analyzer_contract`. Do not expose a
repository path or private home directory in committed fixtures.

**Verify**:

```bash
cargo test -p reklawdbox essentia_environment -- --nocapture
```

Expected: all new tests pass without network access or changes outside their
temporary directories.

### Step 2: Build one wheel-only managed installer behind the application boundary

Create low-level environment/process operations in
`src/adapters/audio/essentia_environment.rs`. Create the reusable orchestration
in `src/application/analysis/setup.rs`; CLI and MCP must call this workflow.
Match the dependency rule in `src/README.md`:

```text
CLI / MCP -> application::analysis -> adapters::audio
```

The shared workflow must:

1. return `AlreadyInstalled` when the managed environment imports the exact
   pinned runtime manifest;
2. find CPython 3.14 by trying `python3.14`, then `python3` only when a parsed
   `sys.version_info` is exactly major 3/minor 14;
3. acquire a bounded OS advisory lock under
   `~/.local/share/reklawdbox` before changing environment state, then re-probe
   after acquiring it so concurrent CLI/MCP processes cannot install over each
   other;
4. create a unique final generation directory under a Reklawdbox-owned sibling
   such as `essentia-envs/`, using `python -m venv --copies`; create the venv at
   its final generation path because Python venv scripts contain absolute paths
   and must not be made "transactional" by moving a completed venv;
5. install with the generation's interpreter, not a separately discovered
   `pip`:

   ```text
   <generation-python> -m pip install --only-binary=:all: \
     essentia==2.1b6.dev1438 numpy==2.5.1 PyYAML==6.0.3 six==1.17.0
   ```

6. validate the exact imported Python/package manifest through the generation
   path, then
   atomically switch the stable user-facing
   `~/.local/share/reklawdbox/essentia-venv` entrypoint to that generation with
   a same-directory relative symlink and validate again through the stable
   path;
7. preserve the prior working target until the stable-path validation passes;
   if creation, installation, switching, or final validation fails, remove only
   the incomplete new generation and preserve/restore the prior target;
8. migrate a legacy canonical directory safely under the same lock rather than
   deleting it before the replacement validates, and prune superseded managed
   generations only after success;
9. return structured diagnostics for lock timeout, candidate-not-found, venv
   creation, wheel unavailability, pip failure, import failure, and manifest
   mismatch;
10. never fall back to an unpinned pre-release, source compilation, or a
    repository-local venv; and
11. allow an injected command runner/filesystem root in tests so mandatory
    tests stay deterministic and offline.

Keep the complete package manifest, supported Python version, analyzer-contract
identifier, stable managed relative path, generation root, and import check in
one adapter-owned contract. Do not duplicate their string values in CLI or
MCP. The stable managed path remains the only public/configured entrypoint;
generation paths are internal state and must not appear in host configuration
or public instructions.

Use an explicit test seam rather than testing transaction helpers piecemeal:

1. define an internal `EnvironmentOps` (or equivalently named) trait that owns
   command execution and runtime inspection;
2. return a small owned command result containing success, stdout, and stderr
   instead of exposing `std::process::Output` to fake implementations;
3. make the production implementation call the real subprocess/probe code;
4. make `install_managed_essentia()` derive the real managed paths and delegate
   to an internal `install_managed_essentia_at(paths, ops, lock_config)`
   transaction;
5. let tests supply a temp managed root, short lock timeout/poll values, and a
   scripted fake that records calls and creates `generation/bin/python` when
   venv creation succeeds; and
6. keep filesystem mutations in the transaction itself so tests exercise the
   same symlink, rename, cleanup, and pruning code as production.

Do not accept direct unit tests of isolated helpers as a substitute: the
mandatory tests must drive the complete install transaction without reading
the real `HOME`/`PATH`, invoking pip, or using the network.

**Verify**:

```bash
cargo test -p reklawdbox essentia_environment -- --nocapture
! rg -n "PYTHON_CANDIDATES|python_candidates|pip install.*essentia" src/cli src/mcp
```

Expected: tests prove command order, interprocess exclusion, atomic stable-path
switching, rollback/failure behavior, and the grep finds no transport-owned
installation policy.

The focused transaction suite must prove all of the following:

- exact `python3.14`, then `python3`, candidate order and rejection of a
  non-CPython/non-3.14 candidate;
- exact `-m venv --copies` and
  `<generation-python> -m pip install --only-binary=:all: --no-deps` arguments
  for all four pinned packages;
- venv failure, pip failure, and direct generation-probe mismatch each remove
  only the new generation and preserve the previous stable runtime;
- successful activation writes a relative stable symlink, probes through that
  stable path, returns the stable Python path, and only then prunes the prior
  managed generation or preserved legacy directory;
- stable-path probe failure restores both a previous relative symlink and a
  previous legacy directory in separate cases, removes the new generation,
  and leaves no `.switch`, `.failed`, or incomplete-generation artifacts; and
- a held kernel advisory lock times out within the injected bound and a new
  acquisition succeeds after the first lock is dropped.

### Step 3: Wire CLI and MCP to the shared workflow

In `src/cli/setup.rs`, remove `PYTHON_CANDIDATES`, `find_python`, the local
installer, and direct pip/venv calls. Keep only CLI presentation, MCP host
configuration, optional broker configuration, and database verification.
Present the managed path, Python version, analyzer-contract identifier, pinned
package manifest, and whether the environment was reused or installed.

In `src/mcp/analysis/handlers.rs`, preserve the existing Tokio setup lock and
run the shared blocking workflow through `spawn_blocking`. Translate its typed
result into the existing additive JSON response. On success, set the
process-local override to the validated managed interpreter so the current MCP
process can use it immediately without restart.

Update `src/mcp/context.rs` and `src/mcp/server.rs` only as required to carry
typed runtime data safely. Preserve the current behavior where a successful
`setup_essentia` call can recover from a memoized initial miss. Do not let an
unsupported explicit override shadow a valid managed environment.

Add tests for:

- CLI and MCP both calling the same shared setup policy;
- `already_installed` and `installed` response shapes;
- concurrent MCP calls remaining serialized;
- an unsupported/broken override falling through to managed setup;
- immediate activation after a successful setup; and
- structured, bounded failure when no compatible wheel/runtime exists.

No mandatory test may make a real network call or write to the real home
directory.

**Verify**:

```bash
cargo test -p reklawdbox setup_essentia -- --nocapture
cargo test -p reklawdbox essentia_python -- --nocapture
```

Expected: every setup/probe regression passes; no test writes outside a
temporary directory.

### Step 4: Version the pinned analyzer in cache and profile compatibility

Change `ESSENTIA_SCHEMA_VERSION` from `"2"` to `"3"`. The accepted upstream
analyzer changes from an uncontrolled floating package and dependencies to the
locked CPython/four-package manifest. Treat that semantic change as
cache-incompatible.

Keep the exact Essentia package version in the payload's existing
`analyzer_version` and add a structured runtime manifest containing the Python,
NumPy, PyYAML, and six versions plus the stable analyzer-contract identifier.
Do not remove the existing field. Add tests proving:

- old v2 cache rows remain stored but are not returned as fresh v3 rows;
- current v3 writes round-trip;
- v3 payloads report the complete manifest and the supported contract
  identifier;
- profile metadata carrying Essentia schema v2 loads as `incompatible`; and
- no migration deletes or rewrites old cache/profile data.

Do not bump `STRATUM_SCHEMA_VERSION`. Do not bump the store schema merely to
change an analyzer freshness constant. Plan 037 will separately bump the
classifier profile schema when it changes the training sample contract.

**Verify**:

```bash
cargo test -p reklawdbox adapters::state -- --nocapture
rg -n 'ESSENTIA_SCHEMA_VERSION: &str = "3"' src/adapters/audio/mod.rs
```

Expected: old rows are preserved/inert, current rows are fresh, and the exact
constant appears once.

### Step 5: Align committed setup and environment documentation

Update the in-scope docs so they consistently say:

- Reklawdbox creates and probes the managed environment at
  `~/.local/share/reklawdbox/essentia-venv`;
- the stable path may point to a Reklawdbox-owned validated generation, but
  generation paths are internal and never configured directly;
- no environment variable is needed for standard use;
- `CRATE_DIG_ESSENTIA_PYTHON` is an expert override, not the normal setup path;
- the supported analyzer contract is CPython 3.14 plus the pinned binary
  package manifest for supported platforms;
- changing Python, Essentia, or any pinned runtime dependency requires an
  Essentia cache-version review;
- Essentia is installed as an external AGPL-3.0-only package and is not bundled
  in the MIT release; and
- the core server can still run without it, while Plan 037 will define the
  degraded/full classification boundary.

Update `AGENTS.md` so its MCP guidance no longer claims the ignored local
`.mcp.json` points at `.venvs/essentia/bin/python`. Keep its warning that a repo
build does not refresh the Homebrew host binary.

Do not remove truthful documentation for explicit Stratum-only analysis or
graceful transition/pool scoring. Plan 037 owns the classification-specific
wording and MCP result contract.

**Verify**:

```bash
! rg -n "\.venvs/essentia" AGENTS.md README.md CONTRIBUTING.md src site scripts
rg -n "essentia-venv|2\.1b6\.dev1438|AGPL" README.md site/src/content/docs
```

Expected: the repository-local path has no committed references and the
managed/version/license contract is documented on setup and reference
surfaces.

### Step 6: Migrate this checkout's ignored local environment safely

Perform this step only after Steps 1-5 pass and while operating in the original
checkout, not an isolated executor worktree. These are local machine changes;
never stage them.

1. Build the release binary.
2. Run `./target/release/reklawdbox setup` or call the built binary's
   `setup_essentia` tool.
3. Validate the managed interpreter directly:

   ```bash
   "$HOME/.local/share/reklawdbox/essentia-venv/bin/python" -c \
     'import essentia,numpy,yaml,six,sys; expected=((3,14),"2.1b6.dev1438","2.5.1","6.0.3","1.17.0"); actual=(sys.version_info[:2],essentia.__version__,numpy.__version__,yaml.__version__,six.__version__); assert actual == expected, actual; print(actual)'
   ```

4. Run the existing ignored temporary-store real-audio round trip with the
   managed interpreter explicitly selected:

   ```bash
   CRATE_DIG_ESSENTIA_PYTHON="$HOME/.local/share/reklawdbox/essentia-venv/bin/python" \
     cargo test -p reklawdbox \
       analyze_track_audio_essentia_cache_round_trip_real_track \
       -- --ignored --nocapture
   ```

5. Edit the ignored repository `.mcp.json` to remove only the
   `CRATE_DIG_ESSENTIA_PYTHON` entry. If that leaves an empty `env` object,
   remove the empty object. Preserve the configured command and arguments.
6. Restart/reconnect the relevant MCP host so it discovers the managed path.
7. Re-run one happy audio path and one missing-track/error path against the
   current checkout.
8. Only after all prior checks pass, delete exactly
   `$REPO_ROOT/.venvs/essentia`. Resolve `REPO_ROOT` with
   `git rev-parse --show-toplevel`, assert the target begins with
   `$REPO_ROOT/.venvs/`, and never construct the deletion from an empty
   variable.

If the ignored `.mcp.json` contains unrelated local edits, preserve them. If
the managed environment or real-audio test fails, keep the stale directory for
forensics and STOP instead of deleting it.

**Verify**:

```bash
managed="$HOME/.local/share/reklawdbox/essentia-venv/bin/python"
test -x "$managed"
"$managed" -c 'import essentia,numpy,yaml,six,sys; assert (sys.version_info[:2],essentia.__version__,numpy.__version__,yaml.__version__,six.__version__) == ((3,14),"2.1b6.dev1438","2.5.1","6.0.3","1.17.0")'
! rg -n "\.venvs/essentia|CRATE_DIG_ESSENTIA_PYTHON" .mcp.json
test ! -e "$(git rev-parse --show-toplevel)/.venvs/essentia"
git status --short
```

Expected: managed import exits 0; ignored local configuration no longer pins
the repo venv; the stale directory is absent; `git status --short` contains
only the intended committed-source plan diff.

### Step 7: Run public-contract and full workspace gates

Because setup descriptions, MCP help, cache compatibility, and public docs
change, run the complete gate plus the doc-drift workflow:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
git diff --check
git status --short
```

Then run the semantic review prompt in
`docs/workflows/doc-drift/prompt.md` against the changed setup, environment,
architecture, and troubleshooting surfaces.

Expected: every command exits 0; MCP smoke reports no protocol violations;
docs match the current binary; only in-scope committed files and the plan-index
status change are modified.

## Test plan

- Adapter tests use fake executables and temporary homes to prove exact runtime
  manifest validation, timeout handling, managed-path discovery, and override
  fallback.
- Shared workflow tests inject command execution and assert exact venv/pip
  arguments, candidate order, wheel-only behavior, bounded interprocess
  locking, validate-before-switch behavior, rollback, and structured failures.
- CLI/MCP tests prove both transports use the same workflow; MCP setup remains
  serialized and activates the managed interpreter without restart.
- State tests prove the Essentia v2-to-v3 freshness transition preserves old
  rows and suppresses incompatible profile metadata.
- Mandatory tests perform no network access and require no private audio.
- The ignored real-audio test is an operator smoke in a temporary store and is
  not added to CI.
- Documentation contract tests plus the semantic review cover the changed
  public setup and runtime claims.

## Done criteria

- [x] One shared setup workflow owns Python selection, managed venv creation,
      package installation, import validation, and typed diagnostics.
- [x] CLI and MCP contain no duplicate Python candidate list or pip policy.
- [x] Standard setup creates only
      `~/.local/share/reklawdbox/essentia-venv` as its stable public entrypoint;
      internal generations are never exposed in configuration or docs.
- [x] Setup serializes concurrent processes and a failed rebuild leaves the
      previously valid managed runtime usable.
- [x] `CRATE_DIG_ESSENTIA_PYTHON` remains an expert override but cannot make an
      unsupported analyzer look current.
- [x] Installation is pinned to the documented binary four-package manifest
      under CPython 3.14; no floating top-level or transitive dependency remains.
- [x] `ESSENTIA_SCHEMA_VERSION` is `3`; v2 cache/profile state is preserved but
      incompatible.
- [x] The MIT release does not bundle Essentia; docs disclose the external
      AGPL package boundary.
- [x] Committed files contain no `.venvs/essentia` path.
- [x] This checkout's ignored `.mcp.json` no longer points to the repo venv,
      the managed import succeeds, and the stale repo venv is removed only
      after the real-audio smoke passes.
- [x] Mandatory tests are offline/private-data-free; the opt-in managed
      real-audio round trip passes locally.
- [x] Full workspace, release, MCP smoke, site, documentation-contract, and
      semantic doc-drift gates pass.
- [x] No cache rows, profile rows, audio files, Rekordbox data, or unrelated
      local configuration were deleted or modified.
- [x] `plans/README.md` status row is updated.

## STOP conditions

Stop and report rather than improvising if:

- PyPI no longer provides any selected manifest package as a compatible binary
  wheel for the target platform and CPython 3.14;
- the exact dependency manifest does not pass the existing real-audio test, or
  implementing it would require silently selecting a different package version;
- the maintainer wants Python 3.9-3.13, Windows, Linux ARM, multiple upstream
  analyzer versions, or source builds supported by the same cache contract;
- selecting a different Essentia version would occur without an analyzer
  cache-version review and a classification benchmark plan;
- implementation would vendor, statically link, or redistribute Essentia
  inside the MIT release rather than installing it separately;
- the shared setup workflow cannot be placed behind the documented
  application/adapter ownership boundary without a wider architecture change;
- the platform cannot provide a bounded interprocess lock and atomic
  same-filesystem stable-path switch without introducing an unreviewed global
  setup race;
- a mandatory test needs real pip/network access, the real home directory,
  private audio, or Rekordbox data;
- old cache/profile rows would need destructive deletion or rewriting;
- the managed import or ignored real-audio smoke fails — do not delete the
  repo-local environment in this case;
- the resolved deletion target is not exactly beneath the current repository's
  `.venvs/` directory;
- files outside Scope require semantic edits; or
- two reasonable attempts cannot make a required verification command pass.

## Maintenance notes

The pinned Python and package manifest is part of the analysis model, not a
disposable setup detail. Any future manifest change requires reviewing
`ESSENTIA_SCHEMA_VERSION`, profile compatibility, real-audio output, and the
classifier benchmark before release. Keep package/version/Python policy in one
source module.

`--copies` reduces the specific Homebrew interpreter-symlink failure observed
here but does not make a venv permanently independent of the underlying
runtime. The Reklawdbox-owned stable-path symlink is different: it selects a
validated immutable generation, while each generation's interpreter is created
with `--copies` at its final path. Setup must remain idempotent and able to
rebuild a broken managed environment without destroying the last working
generation first.

Keep the environment override for expert and test scenarios, but never make a
repo-local absolute path part of standard onboarding. If a future release
bundles or directly links Essentia, revisit licensing and distribution
separately; this plan authorizes only a user-installed subprocess dependency.

Plan 037 depends on this plan because it treats fresh Essentia cache rows as a
classification capability contract. Do not execute Plan 037 against a floating
or ambiguous Essentia runtime.
