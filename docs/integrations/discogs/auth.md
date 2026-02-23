# Discogs Broker Auth

## v1 Model

- Clients do not hold Discogs consumer/token secrets.
- Broker performs OAuth handshake and Discogs API proxy calls.
- Client stores only broker `session_token` metadata in local internal SQLite.

## Client Configuration

Set these in your MCP host env:

- `REKLAWDBOX_DISCOGS_BROKER_URL` (required for broker path)
- `REKLAWDBOX_DISCOGS_BROKER_TOKEN` (required by default; optional only if broker explicitly sets `ALLOW_UNAUTHENTICATED_BROKER=true` for local development)

Legacy `REKLAWDBOX_DISCOGS_*` auth vars still exist as a fallback path, but are deprecated from default docs and config.

## First-Run Auth

1. Call `lookup_discogs`.
2. If unauthenticated, the tool returns an actionable remediation payload/message with `auth_url`.
3. Open `auth_url` in a browser and approve Discogs OAuth.
4. Run `lookup_discogs` again.
5. Client finalizes the broker device session and stores broker `session_token` in internal SQLite.

## Common Failure Modes

- `Discogs auth is not configured`: missing `REKLAWDBOX_DISCOGS_BROKER_URL` and no legacy fallback vars.
- `Discogs sign-in is still pending`: complete browser auth at `auth_url`, then retry.
- `Discogs broker session is missing or expired`: session token expired or rejected; retry to start auth again.
- `broker ... HTTP 401`: broker token mismatch (`REKLAWDBOX_DISCOGS_BROKER_TOKEN`) or invalid broker session.
- `broker ... HTTP 404/410`: pending device flow expired; restart auth.

## Reset / Re-Auth

### Soft reset

- Retry `lookup_discogs` after completing browser flow. If session is invalid, client clears local session and starts a new auth flow.

### Hard reset (local)

1. Close MCP host process.
2. Remove broker session row from internal SQLite (`broker_discogs_session`) or delete the store DB file.
3. Restart host.
4. Run `lookup_discogs` to start auth again.

Default internal store location resolves to platform data dir + `reklawdbox/internal.sqlite3` (for example on macOS: `~/Library/Application Support/reklawdbox/internal.sqlite3`).

Deployment runbook: `broker/README.md` (`Release Runbook (Test-First)` section).
