# Discogs Broker (Cloudflare Worker)

Maintainer-hosted broker for Discogs OAuth and proxied search.

## Endpoints

- `POST /v1/device/session/start`
- `GET /v1/device/session/status?device_id=...&pending_token=...`
- `POST /v1/device/session/finalize`
- `GET /v1/discogs/oauth/link`
- `GET /v1/discogs/oauth/callback`
- `POST /v1/discogs/proxy/search`

## Required secrets

Set with `wrangler secret put`:

- `DISCOGS_CONSUMER_KEY`
- `DISCOGS_CONSUMER_SECRET`

## Optional vars

Set in `wrangler.toml` `[vars]`:

- `BROKER_PUBLIC_BASE_URL`
- `BROKER_CLIENT_TOKEN`
- `DEVICE_SESSION_TTL_SECONDS` (default `900`)
- `SESSION_TOKEN_TTL_SECONDS` (default `2592000`)
- `SEARCH_CACHE_TTL_SECONDS` (default `604800`)
- `DISCOGS_MIN_INTERVAL_MS` (default `1100`)

## Local dev

```bash
cd broker
npm install
npm run d1:migrate:local
npm run dev
```

Or run:

```bash
cd broker
./scripts/dev.sh
```

## Release Runbook (Test-First)

Ship broker auth changes in strict order:

1. Build and test first.
2. Deploy only after all checks are green.

### Preconditions

- `wrangler login` completed for the target Cloudflare account.
- `broker/wrangler.toml` points to the real D1 `database_id`.
- Required secrets are set (`DISCOGS_CONSUMER_KEY`, `DISCOGS_CONSUMER_SECRET`).
- Optional vars are configured as needed (`BROKER_PUBLIC_BASE_URL`, `BROKER_CLIENT_TOKEN`).

### Gate 1: Rust Build/Test

From repo root:

```bash
cargo fmt --all
cargo test
```

Go only if all tests pass.

### Gate 2: Broker Build/Test

From repo root:

```bash
cd broker
npm install
npm run d1:migrate:local
npm run dev
```

In another terminal:

```bash
curl -sS -X POST "http://127.0.0.1:8787/v1/device/session/start" \
  -H "content-type: application/json" \
  -H "x-reklawdbox-broker-token: <BROKER_CLIENT_TOKEN_IF_ENABLED>"
```

Expected payload includes:
`device_id`, `pending_token`, `auth_url`, `poll_interval_seconds`, `expires_at`.

### Gate 3: Client + Broker Integration

Set MCP host env:

- `REKLAWDBOX_DISCOGS_BROKER_URL` (local or deployed broker URL)
- `REKLAWDBOX_DISCOGS_BROKER_TOKEN` (if broker enforces client token)

Run `lookup_discogs` from MCP client:

1. First call should return auth remediation containing `auth_url`.
2. Complete browser OAuth.
3. Re-run `lookup_discogs`; expect normalized result and cache behavior.

### Gate 4: Deploy

From repo root:

```bash
cd broker
npm run d1:migrate:remote
npm run deploy
```

Post-deploy checks:

1. `POST /v1/device/session/start` works.
2. OAuth callback marks session authorized.
3. `POST /v1/discogs/proxy/search` works with bearer `session_token`.
4. MCP `lookup_discogs` succeeds against deployed broker.

## Local Contract Smoke (Full Flow)

In terminal 1:

```bash
cd broker
npm install
npm run d1:migrate:local
npm run dev
```

In terminal 2 (`jq` required):

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

After browser approval:

```bash
STATUS_JSON=$(curl -sS "${BROKER_URL}/v1/device/session/status?device_id=${DEVICE_ID}&pending_token=${PENDING_TOKEN}" \
  -H "x-reklawdbox-broker-token: ${BROKER_TOKEN}")
echo "$STATUS_JSON"
```

If status is `authorized`:

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

Expected proxy response includes `result`, `match_quality`, and `cache_hit`.

## Failure Triage

| Surface | Signal | Meaning | Action |
|---|---|---|---|
| `start/status/finalize` | `401 invalid broker client token` | `BROKER_CLIENT_TOKEN` mismatch | Fix token in broker env and MCP env/header. |
| `status/finalize` | `404 device session not found` | bad/expired `device_id` + `pending_token` pair | Restart from `POST /v1/device/session/start`. |
| `finalize` | `409 device session is not authorized yet` | browser OAuth not completed | Complete auth URL flow, retry finalize. |
| `finalize` | `410 device session expired; restart auth` | session TTL elapsed | Start a new device session and re-auth. |
| `proxy/search` | `401 missing bearer session token` | no bearer token sent | Send `authorization: Bearer <session_token>`. |
| `proxy/search` | `401 invalid or expired broker session` | stale/invalid `session_token` | Re-run auth flow to obtain fresh session. |
| `proxy/search` | `400 artist and title are required` | invalid request body | Send JSON with non-empty `artist` and `title`. |

## Rollback

If deploy is bad:

1. Re-deploy the previous known-good Worker version.
2. If schema-related issue, stop traffic first; do not hot-edit data manually.
3. Clear local MCP broker session row (`broker_discogs_session`) and re-auth.
