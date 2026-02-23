import {
  createExecutionContext,
  env,
  waitOnExecutionContext,
} from 'cloudflare:test'
import { beforeEach, describe, expect, it } from 'vitest'
import worker from '../src/index'

const BASE_URL = 'https://broker.test'
const TOKEN_HEADER = 'x-reklawdbox-broker-token'

beforeEach(async () => {
  await env.DB.prepare('DELETE FROM device_sessions',).run()
  await env.DB.prepare('DELETE FROM oauth_request_tokens',).run()
  await env.DB.prepare('DELETE FROM discogs_search_cache',).run()
  await env.DB.prepare('DELETE FROM rate_limit_state',).run()
},)

describe('broker test runner baseline', () => {
  it('rejects start requests without broker client token header', async () => {
    const response = await request('/v1/device/session/start', {
      method: 'POST',
    },)

    expect(response.status).toBe(401)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('unauthorized')
    expect(body.message).toBe('invalid broker client token')
  },)

  it('returns guidance when broker token is not configured', async () => {
    const response = await request(
      '/v1/device/session/start',
      {
        method: 'POST',
      },
      {
        BROKER_CLIENT_TOKEN: '',
        ALLOW_UNAUTHENTICATED_BROKER: undefined,
      },
    )

    expect(response.status).toBe(401)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('unauthorized')
    expect(body.message).toContain('BROKER_CLIENT_TOKEN')
  },)

  it('creates a device session when broker token is valid', async () => {
    const response = await request('/v1/device/session/start', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
      },
    },)

    expect(response.status).toBe(200)
    const body = await response.json<{
      device_id: string
      pending_token: string
      auth_url: string
      poll_interval_seconds: number
      expires_at: number
    }>()

    expect(body.device_id).toMatch(/^[0-9a-f]{40}$/)
    expect(body.pending_token).toMatch(/^[0-9a-f]{48}$/)
    expect(body.auth_url).toContain('/v1/discogs/oauth/link?device_id=')
    expect(body.poll_interval_seconds).toBe(5)
    expect(body.expires_at).toBeGreaterThan(Math.floor(Date.now() / 1000,))

    const row = await env.DB.prepare(
      `SELECT status
       FROM device_sessions
       WHERE device_id = ?1`,
    )
      .bind(body.device_id,)
      .first<{ status: string }>()
    expect(row?.status).toBe('pending')
  },)

  it('returns invalid_json for malformed finalize payloads', async () => {
    const response = await request('/v1/device/session/finalize', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
        'content-type': 'application/json',
      },
      body: '{"device_id":"abc"',
    },)

    expect(response.status).toBe(400)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('invalid_json')
    expect(body.message).toBe('request body must be valid JSON')
  },)

  it('replays the same finalize token for an already-finalized session', async () => {
    const deviceId = 'device-finalized'
    const pendingToken = 'pending-finalized'
    const oauthToken = 'oauth-access-token'
    const oauthSecret = 'oauth-access-secret'
    const expectedSessionToken = await deriveFinalizeSessionToken(
      deviceId,
      pendingToken,
      oauthToken,
      oauthSecret,
    )
    const expectedSessionTokenHash = await sha256Hex(expectedSessionToken,)
    const now = Math.floor(Date.now() / 1000,)
    const sessionExpiresAt = now + 3600

    await env.DB.prepare(
      `INSERT INTO device_sessions (
        device_id,
        pending_token,
        status,
        poll_interval_seconds,
        created_at,
        updated_at,
        expires_at,
        authorized_at,
        oauth_access_token,
        oauth_access_token_secret,
        oauth_identity,
        session_token_hash,
        session_expires_at,
        finalized_at
      ) VALUES (
        ?1, ?2, 'finalized', 5, ?3, ?3, ?4, ?3, ?5, ?6, 'tester', ?7, ?8, ?3
      )`,
    )
      .bind(
        deviceId,
        pendingToken,
        now,
        now + 600,
        oauthToken,
        oauthSecret,
        expectedSessionTokenHash,
        sessionExpiresAt,
      )
      .run()

    const response = await request('/v1/device/session/finalize', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        device_id: deviceId,
        pending_token: pendingToken,
      },),
    },)

    expect(response.status).toBe(200)
    const body = await response.json<{
      session_token: string
      expires_at: number
    }>()
    expect(body.session_token).toBe(expectedSessionToken)
    expect(body.expires_at).toBe(sessionExpiresAt)
  },)

  it('reports auth posture on /v1/health', async () => {
    const okResponse = await request('/v1/health', {
      method: 'GET',
    },)

    expect(okResponse.status).toBe(200)
    const okBody = await okResponse.json<{
      status: string
      broker_client_auth: {
        mode: string
        token_configured: boolean
        allow_unauthenticated_broker: boolean
      }
    }>()
    expect(okBody.status).toBe('ok')
    expect(okBody.broker_client_auth.mode).toBe('token_required')
    expect(okBody.broker_client_auth.token_configured).toBe(true)
    expect(okBody.broker_client_auth.allow_unauthenticated_broker).toBe(false)

    const devOverrideResponse = await request(
      '/v1/health',
      { method: 'GET', },
      { ALLOW_UNAUTHENTICATED_BROKER: 'true', },
    )
    const devOverrideBody = await devOverrideResponse.json<{
      status: string
      broker_client_auth: {
        mode: string
        warning?: string
      }
    }>()
    expect(devOverrideBody.status).toBe('warning')
    expect(devOverrideBody.broker_client_auth.mode).toBe('unauthenticated_dev_override')
    expect(devOverrideBody.broker_client_auth.warning).toContain('local development')

    const misconfiguredResponse = await request(
      '/v1/health',
      { method: 'GET', },
      {
        BROKER_CLIENT_TOKEN: '',
        ALLOW_UNAUTHENTICATED_BROKER: undefined,
      },
    )
    const misconfiguredBody = await misconfiguredResponse.json<{
      status: string
      broker_client_auth: {
        mode: string
        warning?: string
      }
    }>()
    expect(misconfiguredBody.status).toBe('warning')
    expect(misconfiguredBody.broker_client_auth.mode).toBe('misconfigured_no_token')
    expect(misconfiguredBody.broker_client_auth.warning).toContain('BROKER_CLIENT_TOKEN')
  },)

  it('prunes expired rows when scheduled handler runs', async () => {
    const now = Math.floor(Date.now() / 1000,)

    await env.DB.prepare(
      `INSERT INTO device_sessions (
        device_id,
        pending_token,
        status,
        poll_interval_seconds,
        created_at,
        updated_at,
        expires_at
      ) VALUES (?1, 'pending-a', 'pending', 5, ?2, ?2, ?3)`,
    )
      .bind('expired-pending-session', now - 120, now - 1,)
      .run()

    await env.DB.prepare(
      `INSERT INTO device_sessions (
        device_id,
        pending_token,
        status,
        poll_interval_seconds,
        created_at,
        updated_at,
        expires_at,
        authorized_at,
        oauth_access_token,
        oauth_access_token_secret,
        oauth_identity,
        session_token_hash,
        session_expires_at,
        finalized_at
      ) VALUES (
        ?1, 'pending-b', 'finalized', 5, ?2, ?2, ?3, ?2, 'token', 'secret', 'tester', 'hash', ?4, ?2
      )`,
    )
      .bind(
        'expired-finalized-session',
        now - 120,
        now + 3600,
        now - 1,
      )
      .run()

    await env.DB.prepare(
      `INSERT INTO device_sessions (
        device_id,
        pending_token,
        status,
        poll_interval_seconds,
        created_at,
        updated_at,
        expires_at,
        authorized_at,
        oauth_access_token,
        oauth_access_token_secret,
        oauth_identity,
        session_token_hash,
        session_expires_at,
        finalized_at
      ) VALUES (
        ?1, 'pending-c', 'finalized', 5, ?2, ?2, ?3, ?2, 'token', 'secret', 'tester', 'hash', ?4, ?2
      )`,
    )
      .bind(
        'active-finalized-session',
        now - 120,
        now + 3600,
        now + 3600,
      )
      .run()

    await env.DB.prepare(
      `INSERT INTO oauth_request_tokens (
        oauth_token,
        oauth_token_secret,
        device_id,
        pending_token,
        created_at,
        expires_at
      ) VALUES (?1, 's', 'd', 'p', ?2, ?3)`,
    )
      .bind('expired-oauth-token', now - 100, now - 1,)
      .run()

    await env.DB.prepare(
      `INSERT INTO oauth_request_tokens (
        oauth_token,
        oauth_token_secret,
        device_id,
        pending_token,
        created_at,
        expires_at
      ) VALUES (?1, 's', 'd', 'p', ?2, ?3)`,
    )
      .bind('active-oauth-token', now - 100, now + 3600,)
      .run()

    await env.DB.prepare(
      `INSERT INTO discogs_search_cache (
        cache_key,
        response_json,
        cached_at,
        expires_at
      ) VALUES (?1, ?2, ?3, ?4)`,
    )
      .bind('expired-cache', '{"result":null}', now - 100, now - 1,)
      .run()

    await env.DB.prepare(
      `INSERT INTO discogs_search_cache (
        cache_key,
        response_json,
        cached_at,
        expires_at
      ) VALUES (?1, ?2, ?3, ?4)`,
    )
      .bind('active-cache', '{"result":null}', now - 100, now + 3600,)
      .run()

    const ctx = createExecutionContext()
    const controller = {
      cron: '0 * * * *',
      scheduledTime: Date.now(),
      noRetry() {},
    } as ScheduledController

    await worker.scheduled(controller, env, ctx,)
    await waitOnExecutionContext(ctx,)

    const remainingDeviceRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM device_sessions`,
    )
      .first<{ count: number }>()
    expect(Number(remainingDeviceRows?.count ?? 0,)).toBe(1)

    const remainingOauthRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM oauth_request_tokens`,
    )
      .first<{ count: number }>()
    expect(Number(remainingOauthRows?.count ?? 0,)).toBe(1)

    const remainingCacheRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM discogs_search_cache`,
    )
      .first<{ count: number }>()
    expect(Number(remainingCacheRows?.count ?? 0,)).toBe(1)
  },)
},)

function request(
  path: string,
  init: RequestInit,
  overrides?: Partial<typeof env>,
): Promise<Response> {
  const req = new Request(`${BASE_URL}${path}`, init,)
  const runtimeEnv = overrides ? ({ ...env, ...overrides, } as typeof env) : env
  return worker.fetch(req, runtimeEnv,)
}

async function deriveFinalizeSessionToken(
  deviceId: string,
  pendingToken: string,
  oauthToken: string,
  oauthTokenSecret: string,
): Promise<string> {
  const message = `${deviceId}:${pendingToken}:${oauthToken}`
  const partA = await hmacSha256Hex(
    oauthTokenSecret,
    `broker-session:v1:${message}`,
  )
  const partB = await hmacSha256Hex(
    oauthTokenSecret,
    `broker-session:v1:extra:${message}`,
  )
  return `${partA}${partB.slice(0, 32)}`
}

async function hmacSha256Hex(key: string, input: string,): Promise<string> {
  const encoder = new TextEncoder()
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    encoder.encode(key,),
    { name: 'HMAC', hash: 'SHA-256', },
    false,
    ['sign',],
  )
  const digest = await crypto.subtle.sign(
    'HMAC',
    cryptoKey,
    encoder.encode(input,),
  )
  const bytes = new Uint8Array(digest,)
  return Array.from(bytes,).map((b,) => b.toString(16,).padStart(2, '0',)).join('',)
}

async function sha256Hex(input: string,): Promise<string> {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(input,),
  )
  const bytes = new Uint8Array(digest,)
  return Array.from(bytes,).map((b,) => b.toString(16,).padStart(2, '0',)).join('',)
}
