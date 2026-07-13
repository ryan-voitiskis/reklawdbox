# Source map

`src/` is organized by responsibility so each concept has one canonical home.
Start here, then follow the capability directory that matches the change.

```text
src/
├── main.rs           Process entry point and top-level package declarations
├── bootstrap/        Startup mode detection and environment initialization
├── domain/           Pure models, policies, calculations, and invariants
│   ├── classification/
│   ├── library/
│   ├── metadata/
│   └── planning/
├── application/      Transport-independent workflows that coordinate domain logic and adapters
│   ├── analysis/
│   ├── audit/
│   ├── classification/
│   ├── enrichment/
│   ├── metadata/
│   └── planning/
├── adapters/         Integrations with databases, files, analyzers, providers, and the platform
│   ├── audio/
│   ├── platform/
│   ├── providers/
│   ├── rekordbox/
│   └── state/
├── mcp/              MCP tool schemas, handlers, presentation, and capability tests
├── cli/              CLI parsing, dispatch, prompts, progress, and terminal presentation
└── README.md         This navigation and boundary guide
```

`main.rs` wires the process together but does not own product behavior.
`bootstrap/` decides how the process starts and prepares its environment.
`domain/` owns rules and data that do not require I/O.
`application/` owns use cases shared by MCP and CLI.
`adapters/` owns all communication with external systems and writable local
infrastructure. `mcp/` and `cli/` translate their respective transports into
application calls and present the results.

## Dependency rule

```text
CLI / MCP  ->  application  ->  domain + adapters
                                 adapters -> domain (allowed)
domain must never depend on application, adapters, CLI, or MCP
```

Keep transport types at the edge. Application workflows may accept domain
models and adapter-neutral inputs, while domain code must remain usable without
a database, filesystem, network, MCP host, or terminal.

## Safety boundaries

- **Rekordbox library access is read-only.** Code under
  `adapters/rekordbox/` opens Rekordbox `master.db` read-only; never add a
  direct database write path.
- **Local state is writable.** `adapters/state/` may write Reklawdbox-owned
  SQLite data for caches, analysis, audit, calibration, presets, and broker
  sessions. These writes must not be confused with Rekordbox library edits.
- **Rekordbox metadata changes are staged and exported.** User-visible
  metadata goes through `ChangeManager`, then `write_xml` produces an XML file
  for manual import into Rekordbox. Staging does not mutate `master.db`.
- **Audio-file writes are direct and explicit.** Tag and cover-art operations
  in `adapters/audio/tags.rs` can modify the selected audio files. They are a
  separate boundary from both staged Rekordbox metadata and local cache writes,
  and their MCP/CLI surfaces must preserve preview and confirmation behavior.

## Where to add something

| Adding               | Canonical home                                                                                                                               |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| MCP tool             | Register its `#[tool]` surface in `mcp/server.rs`, then put transport types and handling in the matching `mcp/<capability>/` directory.      |
| CLI command          | Add parsing and dispatch under `cli/`; place reusable work in `application/` rather than the command handler.                                |
| Application workflow | Add it to the capability under `application/`, coordinating domain rules and adapters without MCP or CLI types.                              |
| Domain rule or model | Add it to the matching `domain/` capability and keep it free of I/O and infrastructure dependencies.                                         |
| SQL query            | Put read-only Rekordbox queries in `adapters/rekordbox/` and Reklawdbox-owned writable queries in `adapters/state/`.                         |
| Metadata provider    | Add the external client under `adapters/providers/`; keep provider-independent sequencing and resolution in `application/enrichment/`.       |
| Audio analyzer       | Add decoding or analyzer integration under `adapters/audio/`; keep shared analysis workflow and identity policy in `application/analysis/`.  |
| Platform integration | Add operating-system configuration, credentials, or process integration under `adapters/platform/`; reserve `bootstrap/` for startup wiring. |

Files named `mod.rs` are navigation surfaces only: declarations, focused
re-exports, and short module-level documentation belong there. Put models,
rules, workflows, I/O, and tests in named files or submodules so directory
exploration reveals where behavior lives.
