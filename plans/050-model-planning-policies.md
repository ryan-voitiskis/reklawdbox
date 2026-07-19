# Plan 050: Model planning policies explicitly

> **Executor instructions**: Replace long parameter lists only with cohesive
> planning concepts that already exist in the behavior. Preserve scoring,
> ordering, tie-breakers, defaults, floating-point operations, MCP schemas, and
> JSON. Do not use a grab-bag context or broad rewrite. Update the tracker only
> after independent review and complete verification.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/domain/planning \
>   src/application/planning \
>   src/mcp/planning \
>   src/mcp/tests/planning.rs
> ```
>
> STOP if transition/pool formulas, public parameter defaults, preset
> resolution, beam/greedy ordering, or timbral normalization changed after the
> planning commit. A behavior change must be planned separately with numerical
> evidence.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: domain modeling / parameter pressure / test navigation
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

The planning subsystem contains seven production
`#[allow(clippy::too_many_arguments)]` sites across domain and application:

- `score_transition_profiles` mixes two profiles with phases, weights, tempo
  policy, harmonic style, run context, and optional play BPMs;
- greedy and beam sequence builders repeat almost the same ten arguments;
- pool discovery mixes scoring policy with graph/output bounds; and
- application rank, expansion, and discovery functions repeat those
  primitives beside store and track inputs.

These are not independent scalar knobs. Call sites repeatedly reconstruct the
same combinations, making it hard to see which values define a mixing policy,
which describe one transition moment, and which bound a search. The large
planning tests then repeat positional calls hundreds of lines apart.

The code already contains one good precedent: `BuildSetOptions` gives a public
workflow request a meaningful home. This plan extends that discipline down to
domain policy without creating a universal planning context.

## Target types

### Transition policy and moment

Add two small domain values:

- `TransitionMixingPolicy<'a>` — `weights`, `master_tempo`, and optional
  `harmonic_style`; and
- `TransitionMoment` — `from_phase`, `to_phase`, `genre_run_length`, and
  optional `(from_play_bpm, to_play_bpm)`.

Then use:

```rust
score_transition_profiles(from, to, mixing_policy, moment)
```

`TransitionMoment` is a description of one edge in a set, not a generic
context. Keep defaults only where they are currently meaningful; call sites
must still be explicit about master tempo and harmonic style.

### Sequence search policy

Add `SequencePolicy<'a>` for the values shared by greedy and beam search:

- target track count;
- energy phases;
- transition mixing policy;
- BPM drift percentage; and
- optional target BPM trajectory.

Keep `variation_index` and `beam_width` as algorithm-specific values rather
than optional fields in one struct. Greedy and beam functions remain separate
implementations; this plan does not invent a search trait.

### Pool scoring and discovery bounds

Separate:

- `PoolScoringPolicy<'a>` — master-tempo mode, reference BPM, weights, and
  optional timbral normalization; and
- `PoolDiscoveryBounds` — threshold, minimum size, maximum size, and maximum
  result count.

Construct `PoolDiscoveryBounds` only after the MCP edge has applied its current
clamps. The type may enforce relationships already guaranteed by those clamps,
but it must not add a new rejection path or alter existing user-visible error
text. Do not convert planning `Result<_, String>` values in this plan unless a
caller already branches on a failure category.

### Application request types

Use capability-specific options for ranking and pool discovery, leaving
tracks and `rusqlite::Connection` as explicit ownership parameters. Prefer:

- `RankTransitionOptions` for phase/mixing/target/limit; and
- `ExpandPoolOptions` for addition count, cross-genre policy, and scoring; and
- `DiscoverPoolsOptions` for scoring and bounds.

Do not put tracks, store connections, output buffers, or transport params into
an omnibus application context.

## Scope

**In scope**:

- `src/domain/planning/model.rs`
- `src/domain/planning/transition.rs`
- `src/domain/planning/sequence.rs`
- `src/domain/planning/pool.rs`
- `src/domain/planning/mod.rs`
- `src/domain/planning/tests/evaluation.rs` (split below)
- `src/domain/planning/tests/mod.rs`
- new `src/domain/planning/tests/{support,transitions,sequence,pools}.rs`
- `src/application/planning/sets.rs`
- `src/application/planning/transitions.rs`
- `src/application/planning/pools.rs`
- `src/application/planning/mod.rs`
- `src/application/planning/tests/**` only for affected application behavior
- `src/mcp/planning/sequencing.rs`
- `src/mcp/planning/pools.rs`
- `src/mcp/tests/planning.rs` (replaced by the directory below)
- new `src/mcp/tests/planning/{mod,support,contracts,transitions,sets,pools,presets}.rs`
- `src/mcp/tests/mod.rs`
- `plans/README.md` status row only during execution

**Out of scope**:

- Any scoring constant, formula, weight, preset value, rounding, tie-breaker,
  phase curve, beam width, candidate ordering, or fallback.
- MCP parameter/result schema, defaults, descriptions, error categories, JSON,
  or tool count.
- Genre taxonomy, aliases, classification, analyzer/cache schemas, provider
  evidence, or timbral calibration semantics.
- Replacing greedy/beam search, adding traits, genericizing all evaluators, or
  moving transport types inward.
- Bulk conversion of planning string errors whose callers do not branch on
  category.
- Merely wrapping every old positional argument in one `Context` struct.

## Steps

### Step 1: Freeze numerical and public behavior

Before changing signatures, add table-driven characterization covering:

1. transition score components, composite value, adjustments, effective key,
   and pitch shift with/without master tempo and target BPM;
2. genre-run phase/context penalties;
3. greedy and beam ordered IDs, tie-breakers, variation behavior, and BPM drift
   penalties;
4. pool compatibility matrix, clique membership/order, bridges, threshold,
   min/max sizes, and max-pool truncation;
5. application profile skip/error handling; and
6. representative MCP JSON for score, rank, build-set, cohesion, and discovery.

Compare structured values and bounded floating-point deltas, not debug strings.
Record the exact current fixture outputs before signature edits.

Focused baseline:

```bash
cargo test -p reklawdbox planning_transition_ -- --nocapture
cargo test -p reklawdbox planning_sequence_ -- --nocapture
cargo test -p reklawdbox planning_pool_ -- --nocapture
cargo test -p reklawdbox mcp_planning_contract_ -- --nocapture
```

Rename existing tests only to add truthful filter prefixes; keep assertions.

### Step 2: Introduce transition policy values

Add `TransitionMixingPolicy` and `TransitionMoment`, migrate
`score_transition_profiles`, and update call sites without moving formula code.
Use constructors/builders only when they encode a real default or invariant;
avoid fluent boilerplate for tests.

Run transition tests and inspect the function body diff. Aside from reading
fields from the new types, the arithmetic and branch order should be
substantively unchanged.

### Step 3: Reuse one sequence policy in greedy and beam search

Add `SequencePolicy`, migrate both builders, and update `BuildSetOptions`
orchestration. Keep greedy `variation_index` and beam `beam_width` explicit.
Do not combine the algorithms or allocate/cloned-own profiles just to satisfy
the type.

Add a parity helper that calls both old-characterized paths through the new
API and compares ordered IDs plus every transition score. Remove the two
sequence `too_many_arguments` suppressions.

### Step 4: Separate pool scoring from discovery bounds

Add `PoolScoringPolicy` and validated `PoolDiscoveryBounds`, then migrate
domain and application discovery. Keep graph construction and Bron-Kerbosch
logic in `pool.rs`; this plan only clarifies inputs.

If MCP already clamps all public values, invalid-bound unit tests should target
the domain constructor and MCP schema/default tests should remain unchanged.
Remove the domain pool and affected application pool suppressions.

### Step 5: Add application option types and preserve edge mapping

Migrate `rank_transition_candidates`, `expand_pool`, and
`discover_track_pools` to focused options. MCP handlers should visibly
translate transport params into domain or application policy values at the
edge. Do not make application types derive `JsonSchema` merely to avoid that
mapping.

Remove the affected application suppressions and assert that no new
`too_many_arguments` suppression was added elsewhere.

### Step 6: Split tests by planning capability

After production APIs are stable, split the 2,055-line domain evaluation file
and 2,342-line MCP planning file into the scoped modules above. Keep only
small canonical profile/store builders in `support.rs`; capability-specific
builders stay with their tests. Remove long test-helper suppressions where a
semantic fixture value makes the call clearer.

`mod.rs` files remain declarations. Do not introduce a 1,000-line shared
support module or re-export private production internals solely for tests.

### Step 7: Verify suppressions and contracts

Run:

```bash
! rg -n '#\[allow\(clippy::too_many_arguments\)\]' \
  src/domain/planning/transition.rs \
  src/domain/planning/sequence.rs \
  src/domain/planning/pool.rs \
  src/application/planning/transitions.rs \
  src/application/planning/pools.rs
cargo test -p reklawdbox planning_ -- --nocapture
cargo test -p reklawdbox mcp_planning_ -- --nocapture
cargo test -p reklawdbox --test source_boundaries
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
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
git diff --check
```

Inspect generated MCP schemas and representative JSON before/after. This plan
does not authorize documentation changes because the public contract is
supposed to be identical.

Require independent domain/API and test-quality reviews. The first reviewer
must compare arithmetic/branch order, call-site mappings, defaults, and
serialized contracts; the second must verify every moved assertion and that
support modules remain narrow. Remediate concrete findings and repeat the
focused numerical tests before approval.

## Machine-checkable done criteria

- [ ] Seven affected production `too_many_arguments` suppressions are removed.
- [ ] Transition policy, transition moment, sequence policy, pool scoring, and
      pool bounds each have one canonical type with a cohesive invariant.
- [ ] Ranking, expansion, and discovery each accept a capability-specific
      application option rather than reconstructing positional policy.
- [ ] No omnibus context, generic search trait, or wrapper-only facade was
      introduced.
- [ ] Numerical characterization is unchanged within existing tolerances;
      ordered IDs, tie-breakers, and adjustment ordering are exact.
- [ ] MCP schemas, defaults, errors, tool count, JSON, and serialization are
      unchanged.
- [ ] Domain and MCP tests are split by capability with small explicit support
      modules and navigation-only `mod.rs` files.
- [ ] No taxonomy, weight, profile, analyzer, cache, or database behavior
      changed.
- [ ] Architecture, workspace, release, MCP, docs-contract, site, and diff
      gates pass.

## STOP conditions

Stop and report if:

- preserving the current numeric outputs would require changing a formula or
  floating-point operation order;
- a new type has no invariant beyond “these arguments happened to be nearby”;
- public transport structs would need to move into domain/application;
- a scoring/taxonomy/profile/cache change appears necessary;
- test splitting requires exposing internal APIs to production; or
- the diff becomes a broad planning rewrite instead of input-modeling work.

## Complexity accounting

Success replaces positional coupling with five named domain concepts and makes
tests addressable by capability. If the same arguments remain hidden inside a
single context, or files are split while every call still reconstructs ad hoc
tuples, complexity was moved rather than removed.

## Git workflow

- Branch: `codex/050-model-planning-policies`
- Preferred commit: `refactor(planning): model scoring policies`
- Do not push, merge, release, or deploy.
