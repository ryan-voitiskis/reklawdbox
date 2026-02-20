# Discogs Broker Test-First Handover

## Purpose

This runbook is for shipping broker auth changes with a strict order:

1. Build and test first.
2. Deploy only after all checks are green.

## Scope

Covers:

- Rust MCP server checks (`reklawdbox`)
- Broker Worker checks (`broker/`)
- Local smoke validation
- Cloudflare deploy + post-deploy verification

## Preconditions

- Cloudflare account access and `wrangler` auth (`wrangler login` complete).
- `broker/wrangler.toml` has real D1 `database_id`.
- Broker secrets are set:
  - `DISCOGS_CONSUMER_KEY`
  - `DISCOGS_CONSUMER_SECRET`
- Optional broker vars are set when needed:
  - `BROKER_PUBLIC_BASE_URL`
  - `BROKER_CLIENT_TOKEN`

## Phase 1: Rust Build/Test Gate (Must Pass)

From repo root:

```bash
cargo fmt --all
cargo test
```

Go/No-Go:

- Go only if `cargo test` has zero failures.
- No deploy if any Rust test fails.

## Phase 2: Broker Build/Test Gate (Must Pass)

From repo root:

```bash
cd broker
npm install
npm run d1:migrate:local
npm run dev
```

In a second terminal, run local endpoint smoke tests (replace token/header as needed):

```bash
curl -sS -X POST "http://127.0.0.1:8787/v1/device/session/start" \
  -H "content-type: application/json" \
  -H "x-reklawdbox-broker-token: <BROKER_CLIENT_TOKEN_IF_ENABLED>"
```

Expected:

- JSON with `device_id`, `pending_token`, `auth_url`, `poll_interval_seconds`, `expires_at`.

Optional status check:

```bash
curl -sS "http://127.0.0.1:8787/v1/device/session/status?device_id=<device_id>&pending_token=<pending_token>" \
  -H "x-reklawdbox-broker-token: <BROKER_CLIENT_TOKEN_IF_ENABLED>"
```

Go/No-Go:

- Go only if local broker starts and endpoint contract matches.
- No deploy if startup, migration, or endpoint contract fails.

## Phase 3: Integration Gate (Client + Broker)

In MCP host env, set:

- `REKLAWDBOX_DISCOGS_BROKER_URL` to local broker URL (for local smoke) or deployed URL.
- `REKLAWDBOX_DISCOGS_BROKER_TOKEN` if broker enforces client token.

Run `lookup_discogs` from MCP client.

Expected flow:

1. First call returns actionable auth remediation including `auth_url`.
2. Open `auth_url` and complete Discogs approval.
3. Re-run `lookup_discogs`; expect Discogs-normalized payload and caching behavior.

Go/No-Go:

- Go only if auth-remediation and post-auth lookup both work.
- No deploy if session finalization or proxy lookup fails.

## Phase 4: Deploy (Only After All Gates Pass)

From repo root:

```bash
cd broker
npm run d1:migrate:remote
npm run deploy
```

Post-deploy checks:

1. `POST /v1/device/session/start` returns contract payload.
2. OAuth callback marks session authorized.
3. `POST /v1/discogs/proxy/search` works with bearer `session_token`.
4. `lookup_discogs` in MCP works against deployed broker.

## Rollback

If deploy is bad:

1. Re-deploy previous known-good Worker version.
2. If schema-related issue, stop traffic first; do not hot-edit data manually.
3. Clear local MCP broker session row (`broker_discogs_session`) and re-auth.

## Known Notes

- Legacy `REKLAWDBOX_DISCOGS_*` remains a deprecated fallback path; default path is broker-based.
- Local session token persistence is in internal SQLite table: `broker_discogs_session`.
- Reference docs:
  - `docs/discogs-broker-auth.md`
  - `docs/discogs-broker-auth-plan.md`

## Quick Operator Checklist

- `cargo test` passes from repo root.
- Local broker starts with `npm run dev` after local D1 migration.
- Local contract checks pass for `start -> status -> finalize -> proxy`.
- MCP `lookup_discogs` flow passes in both states:
  - unauthenticated remediation (`auth_url` shown)
  - post-auth lookup success (result/caching behavior)
- Remote D1 migration is applied before deploy.
- Post-deploy contract checks pass against deployed URL.

## Local Contract Smoke (Full Flow)

From `broker/` in one terminal:

```bash
npm install
npm run d1:migrate:local
npm run dev
```

In a second terminal (`jq` required for variable extraction):

```bash
BROKER_URL="http://127.0.0.1:8787"
BROKER_TOKEN="<BROKER_CLIENT_TOKEN_IF_ENABLED>"

START_JSON=$(curl -sS -X POST "${BROKER_URL}/v1/device/session/start" \
  -H "x-reklawdbox-broker-token: ${BROKER_TOKEN}")
echo "$START_JSON"

DEVICE_ID=$(echo "$START_JSON" | jq -r '.device_id')
PENDING_TOKEN=$(echo "$START_JSON" | jq -r '.pending_token')
AUTH_URL=$(echo "$START_JSON" | jq -r '.auth_url')

echo "Open auth URL: $AUTH_URL"
```

After approving in browser, run:

```bash
STATUS_JSON=$(curl -sS "${BROKER_URL}/v1/device/session/status?device_id=${DEVICE_ID}&pending_token=${PENDING_TOKEN}" \
  -H "x-reklawdbox-broker-token: ${BROKER_TOKEN}")
echo "$STATUS_JSON"
```

If `status` is `authorized`, finalize and call proxy:

```bash
FINALIZE_JSON=$(curl -sS -X POST "${BROKER_URL}/v1/device/session/finalize" \
  -H "content-type: application/json" \
  -H "x-reklawdbox-broker-token: ${BROKER_TOKEN}" \
  -d "{\"device_id\":\"${DEVICE_ID}\",\"pending_token\":\"${PENDING_TOKEN}\"}")
echo "$FINALIZE_JSON"

SESSION_TOKEN=$(echo "$FINALIZE_JSON" | jq -r '.session_token')

curl -sS -X POST "${BROKER_URL}/v1/discogs/proxy/search" \
  -H "content-type: application/json" \
  -H "authorization: Bearer ${SESSION_TOKEN}" \
  -d '{"artist":"Daft Punk","title":"One More Time"}'
```

Expected proxy contract:

- JSON shape includes `result`, `match_quality`, `cache_hit`.
- `match_quality` is one of `exact`, `fuzzy`, `none`.

## Failure Triage (Fast Map)

| Surface | Signal | Meaning | Action |
|---|---|---|---|
| `start/status/finalize` | `401 invalid broker client token` | `BROKER_CLIENT_TOKEN` mismatch | Fix token in broker env and MCP env/header. |
| `status/finalize` | `404 device session not found` | bad/expired `device_id` + `pending_token` pair | Restart at `POST /v1/device/session/start`. |
| `finalize` | `409 device session is not authorized yet` | browser OAuth not completed | Complete auth URL flow, then retry finalize. |
| `finalize` | `410 device session expired; restart auth` | session TTL elapsed | Start a new device session and re-auth. |
| `proxy/search` | `401 missing bearer session token` | no bearer token sent | Send `authorization: Bearer <session_token>`. |
| `proxy/search` | `401 invalid or expired broker session` | stale/invalid `session_token` | Re-run auth flow to obtain fresh session. |
| `proxy/search` | `400 artist and title are required` | invalid request body | Send JSON with non-empty `artist` and `title`. |
| MCP `lookup_discogs` | `Discogs auth is not configured` | missing broker URL env | Set `REKLAWDBOX_DISCOGS_BROKER_URL` and retry. |
| MCP `lookup_discogs` | `Discogs sign-in is still pending` | OAuth not yet approved | Open `auth_url`, approve, retry lookup. |
| MCP `lookup_discogs` | `Discogs broker session is missing or expired` | local token invalid/expired | Re-run lookup to trigger fresh auth flow. |

## Release Evidence Checklist

Capture these artifacts in PR comment or release notes:

- Rust test gate output (`cargo test`) showing pass.
- Local broker smoke payloads for `start`, `status`, `finalize`, `proxy/search`.
- One successful MCP `lookup_discogs` output after browser auth.
- Deployed broker URL and timestamp of post-deploy checks.
- Rollback reference to previous known-good worker version.
