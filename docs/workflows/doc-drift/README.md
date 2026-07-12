# Doc-Drift Workflow

Run the automated documentation contract first, then audit the semantics that
code cannot prove. A green contract is evidence that public inventories and
links agree; it is not proof that the guidance is complete or easy to follow.

## Usage

```sh
cargo build --release
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
```

After that gate passes, use `@docs/workflows/doc-drift/prompt.md` for the
semantic review. Run both after a batch of public-surface changes or before a
release.

## What the automated gate proves

- The live DB-free MCP inventory matches the small tool-to-page mapping.
- Explicitly marked MCP tables match live schema names, types, requiredness,
  exposed enums/defaults, nested objects, and declared empty surfaces.
- Recognized calls in embedded SOPs use live tools and top-level arguments.
- Marked CLI commands, arguments, flags, short forms, and exposed defaults
  match successful application `--help` output.
- The canonical 11-page workflow catalog, 9-entry runtime menu, and 7-step
  recommendation remain distinct and ordered.
- Internal links and runtime-help URLs resolve in the built site.

The checker is DB-free and does not call providers, read credentials, inspect a
Rekordbox library, or use the network.

## What still needs semantic review

- Whether workflow intent and handler behavior are described accurately.
- Whether external Rekordbox UI instructions still match the current product.
- Whether risks, prerequisites, recovery advice, and failure handling are
  clear enough for a first-time user.
- Whether examples, README claims, conceptual explanations, and descriptions
  are useful rather than merely structurally valid.

## Extending the gate

Add assertions at the existing parser and canonical-data boundaries; do not
copy live schemas or workflow records into a snapshot. New tool and CLI tables
need explicit contract markers, and new tools need one mapping entry.

When workflow continuation fields are added, derive their checks directly from
`site/src/data/workflows.mjs`. When generated audience outputs are introduced,
validate the built outputs and their links through `check-doc-contract` rather
than maintaining a second hand-written inventory.

## When to run

- After adding or renaming tools/parameters
- After changing CLI subcommands or flags
- After modifying SOP content (remember SOPs are baked into the binary)
- Before a release, to catch accumulated drift
