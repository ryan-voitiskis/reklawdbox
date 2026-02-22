# Discogs Broker Auth Plan (Issue #6)

## Decisions (Locked)

- Broker is hosted by the project maintainer.
- Broker handles Discogs OAuth handshake and proxies Discogs API requests.
- No email provider in v1.
- Single-machine-first in v1 (no new-machine pairing flow).
- Local client stores only broker session token in local SQLite.
- Existing `REKLAWDBOX_DISCOGS_*` client-secret flow is deprecated from default path.

## Goal

Implement broker-based Discogs OAuth and proxy flow so clients never need a shared Discogs secret.

## Scope Checklist

- [x] Add `broker/` Cloudflare Worker project in-repo with D1 migrations and local dev scripts.
- [x] Implement broker device session start endpoint: `POST /v1/device/session/start`.
- [x] Implement broker device session status endpoint: `GET /v1/device/session/status`.
- [x] Implement broker device session finalize endpoint: `POST /v1/device/session/finalize`.
- [x] Implement Discogs OAuth link start and callback handling in broker.
- [x] Persist Discogs OAuth tokens server-side only (D1).
- [x] Implement Discogs search proxy endpoint in broker: `POST /v1/discogs/proxy/search`.
- [x] Add broker-side rate limiting, 429 retry policy, and lookup caching.
- [x] Add internal SQLite migration in Rust for local broker session state.
- [x] Replace client-side Discogs signing path with broker client calls.
- [x] Update `lookup_discogs` to fail with actionable auth remediation when missing auth.
- [x] Update Discogs path in `enrich_tracks` to use broker and preserve partial-results behavior.
- [x] Add config vars for broker URL/token settings.
- [x] Deprecate `REKLAWDBOX_DISCOGS_*` from default docs/config.
- [x] Document first-run auth flow, common failure modes, and reset/re-auth steps.
- [x] Add tests for broker session lifecycle, auth-required tool behavior, and proxy result mapping.
- [x] Ensure `cargo test` passes.

## Out of Scope

- Multi-machine pairing flow.
- Email/magic-link identity provider.
- Beatport auth changes.

## API Contract v1

- `POST /v1/device/session/start` -> `{ device_id, pending_token, auth_url, poll_interval_seconds, expires_at }`
- `GET /v1/device/session/status?device_id=...&pending_token=...` -> `{ status, expires_at }`
- `POST /v1/device/session/finalize` -> `{ session_token, expires_at }`
- `POST /v1/discogs/proxy/search` with bearer `session_token` -> Discogs-normalized search result payload.

## Conventional Commit Sequence

1. `feat(broker): scaffold cloudflare worker and d1 schema for device sessions`
2. `feat(broker): add device session start status and finalize endpoints`
3. `feat(broker): implement discogs oauth link flow and callback persistence`
4. `feat(broker): add authenticated discogs search proxy with rate limits and cache`
5. `feat(store): add internal sqlite migration for broker session state`
6. `refactor(discogs): replace local oauth signing with broker client integration`
7. `feat(tools): add auth-remediation flow for lookup_discogs and enrich_tracks`
8. `docs(auth): document broker setup sign-in flow and failure recovery`
9. `chore(config): deprecate local discogs secret env vars in templates and guides`
10. `test(auth): cover broker session persistence and discogs auth-required behavior`

## PR Cut Plan

1. PR 1: Broker scaffold + session endpoints.
2. PR 2: Broker Discogs OAuth + proxy + caching/rate limit.
3. PR 3: Rust migration + broker client + tool rewiring.
4. PR 4: Docs/config deprecation + tests + acceptance pass.
