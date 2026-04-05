# Doc-Drift Workflow

Audits all documentation surfaces against the codebase to find and fix
places where docs describe something that doesn't match what the code
actually does.

## Usage

```
@docs/workflows/doc-drift/prompt.md
```

No additional input needed — the prompt covers all documentation surfaces
automatically. Run it periodically (e.g., after a batch of feature work)
or before a release.

## What it checks

- **MCP tool reference** — tool names, parameter names/types/defaults,
  descriptions
- **CLI docs** — subcommands, flags, arguments, defaults
- **Embedded SOPs** — tool names, parameter references, described behavior
- **README** — feature claims, install instructions, examples
- **Schema descriptions** — schemars/clap annotations vs. site docs

## When to run

- After adding or renaming tools/parameters
- After changing CLI subcommands or flags
- After modifying SOP content (remember SOPs are baked into the binary)
- Before a release, to catch accumulated drift
