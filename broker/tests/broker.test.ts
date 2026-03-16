import {
  createExecutionContext,
  env,
  waitOnExecutionContext,
} from 'cloudflare:test'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import worker from '../src/index'

const BASE_URL = 'https://broker.test'
const TOKEN_HEADER = 'x-reklawdbox-broker-token'
const ENCRYPTED_SECRET_PREFIX = 'enc:v1:'
const DISCOGS_AUTH_NOT_CONFIGURED_MESSAGE = 'discogs auth is not configured'
const DISCOGS_UPSTREAM_FAILURE_MESSAGE = 'discogs upstream request failed'

beforeEach(async () => {
  await env.DB.prepare('DELETE FROM device_sessions').run()
  await env.DB.prepare('DELETE FROM oauth_request_tokens').run()
  await env.DB.prepare('DELETE FROM discogs_search_cache').run()
  await env.DB.prepare('DELETE FROM rate_limit_state').run()
})

describe('broker test runner baseline', () => {
  it('rejects start requests without broker client token header', async () => {
    const response = await request('/v1/device/session/start', {
      method: 'POST',
    })

    expect(response.status).toBe(401)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('unauthorized')
    expect(body.message).toBe('invalid broker client token')
  })

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
  })

  it('creates a device session when broker token is valid', async () => {
    const response = await request('/v1/device/session/start', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
      },
    })

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
    expect(body.expires_at).toBeGreaterThan(Math.floor(Date.now() / 1000))

    const row = await env.DB.prepare(
      `SELECT status
       FROM device_sessions
       WHERE device_id = ?1`,
    )
      .bind(body.device_id)
      .first<{ status: string }>()
    expect(row?.status).toBe('pending')
  })

  it('returns invalid_json for malformed finalize payloads', async () => {
    const response = await request('/v1/device/session/finalize', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
        'content-type': 'application/json',
      },
      body: '{"device_id":"abc"',
    })

    expect(response.status).toBe(400)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('invalid_json')
    expect(body.message).toBe('request body must be valid JSON')
  })

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
    const expectedSessionTokenHash = await sha256Hex(expectedSessionToken)
    const now = Math.floor(Date.now() / 1000)
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
      }),
    })

    expect(response.status).toBe(200)
    const body = await response.json<{
      session_token: string
      expires_at: number
    }>()
    expect(body.session_token).toBe(expectedSessionToken)
    expect(body.expires_at).toBe(sessionExpiresAt)
  })

  it('stores oauth request token secrets encrypted at rest', async () => {
    const start = await request('/v1/device/session/start', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
      },
    })
    const startBody = await start.json<{
      device_id: string
      pending_token: string
      auth_url: string
    }>()
    const authPath = new URL(startBody.auth_url).pathname
      + new URL(startBody.auth_url).search

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response('oauth_token=req-token&oauth_token_secret=req-secret', {
        status: 200,
        headers: {
          'content-type': 'text/plain; charset=utf-8',
        },
      }),
    )

    try {
      const response = await request(authPath, {
        method: 'GET',
      })
      expect(response.status).toBe(302)

      const row = await env.DB.prepare(
        `SELECT oauth_token_secret
         FROM oauth_request_tokens
         WHERE oauth_token = ?1`,
      )
        .bind('req-token')
        .first<{ oauth_token_secret: string }>()

      expect(row?.oauth_token_secret).toBeTruthy()
      expect(row?.oauth_token_secret).not.toBe('req-secret')
      expect(row?.oauth_token_secret).toMatch(
        new RegExp(`^${ENCRYPTED_SECRET_PREFIX}`),
      )
    } finally {
      fetchSpy.mockRestore()
    }
  })

  it('stores access token secrets encrypted and still finalizes sessions', async () => {
    const start = await request('/v1/device/session/start', {
      method: 'POST',
      headers: {
        [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
      },
    })
    const startBody = await start.json<{
      device_id: string
      pending_token: string
      auth_url: string
    }>()
    const authUrl = new URL(startBody.auth_url)

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (url === 'https://api.discogs.com/oauth/request_token') {
          return new Response(
            'oauth_token=req-token&oauth_token_secret=req-secret',
            {
              status: 200,
              headers: {
                'content-type': 'text/plain; charset=utf-8',
              },
            },
          )
        }
        if (url === 'https://api.discogs.com/oauth/access_token') {
          return new Response(
            'oauth_token=access-token&oauth_token_secret=access-secret&username=tester',
            {
              status: 200,
              headers: {
                'content-type': 'text/plain; charset=utf-8',
              },
            },
          )
        }
        throw new Error(`unexpected fetch URL: ${url}`)
      },
    )

    try {
      const linkResponse = await request(
        authUrl.pathname + authUrl.search,
        { method: 'GET' },
      )
      expect(linkResponse.status).toBe(302)

      const callbackResponse = await request(
        `/v1/discogs/oauth/callback?device_id=${
          encodeURIComponent(startBody.device_id)
        }&pending_token=${
          encodeURIComponent(startBody.pending_token)
        }&oauth_token=req-token&oauth_verifier=test-verifier`,
        { method: 'GET' },
      )
      expect(callbackResponse.status).toBe(200)

      const authorized = await env.DB.prepare(
        `SELECT status, oauth_access_token_secret
         FROM device_sessions
         WHERE device_id = ?1`,
      )
        .bind(startBody.device_id)
        .first<{ status: string; oauth_access_token_secret: string }>()

      expect(authorized?.status).toBe('authorized')
      expect(authorized?.oauth_access_token_secret).toBeTruthy()
      expect(authorized?.oauth_access_token_secret).not.toBe('access-secret')
      expect(authorized?.oauth_access_token_secret).toMatch(
        new RegExp(`^${ENCRYPTED_SECRET_PREFIX}`),
      )

      const finalizeResponse = await request('/v1/device/session/finalize', {
        method: 'POST',
        headers: {
          [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          device_id: startBody.device_id,
          pending_token: startBody.pending_token,
        }),
      })

      expect(finalizeResponse.status).toBe(200)
      const finalizeBody = await finalizeResponse.json<{
        session_token: string
      }>()
      expect(finalizeBody.session_token).toMatch(/^[0-9a-f]{96}$/)
    } finally {
      fetchSpy.mockRestore()
    }
  })

  it('migrates legacy plaintext access token secrets on read', async () => {
    const sessionToken = 'legacy-migration-token'
    const sessionTokenHash = await sha256Hex(sessionToken)
    const now = Math.floor(Date.now() / 1000)

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
        ?1, ?2, 'finalized', 5, ?3, ?3, ?4, ?3, ?5, ?6, 'tester', ?7, ?4, ?3
      )`,
    )
      .bind(
        'device-legacy-migration',
        'pending-legacy-migration',
        now,
        now + 3600,
        'legacy-access-token',
        'legacy-access-secret',
        sessionTokenHash,
      )
      .run()

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify({
          results: [
            {
              title: 'Legacy Artist - Legacy Track',
              year: 2025,
              label: ['Test Label'],
              genre: ['Electronic'],
              style: ['House'],
              uri: '/release/456',
            },
          ],
        }),
        {
          status: 200,
          headers: {
            'content-type': 'application/json',
          },
        },
      ),
    )

    try {
      const response = await request('/v1/discogs/proxy/search', {
        method: 'POST',
        headers: {
          authorization: `Bearer ${sessionToken}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          artist: 'Legacy Artist',
          title: 'Legacy Track',
        }),
      })

      expect(response.status).toBe(200)

      const row = await env.DB.prepare(
        `SELECT oauth_access_token_secret
         FROM device_sessions
         WHERE device_id = ?1`,
      )
        .bind('device-legacy-migration')
        .first<{ oauth_access_token_secret: string }>()

      expect(row?.oauth_access_token_secret).toBeTruthy()
      expect(row?.oauth_access_token_secret).not.toBe('legacy-access-secret')
      expect(row?.oauth_access_token_secret).toMatch(
        new RegExp(`^${ENCRYPTED_SECRET_PREFIX}`),
      )
    } finally {
      fetchSpy.mockRestore()
    }
  })

  it('reports auth posture on /v1/health', async () => {
    const okResponse = await request('/v1/health', {
      method: 'GET',
    })

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
      { method: 'GET' },
      { ALLOW_UNAUTHENTICATED_BROKER: 'true' },
    )
    const devOverrideBody = await devOverrideResponse.json<{
      status: string
      broker_client_auth: {
        mode: string
        warning?: string
      }
    }>()
    expect(devOverrideBody.status).toBe('warning')
    expect(devOverrideBody.broker_client_auth.mode).toBe(
      'unauthenticated_dev_override',
    )
    expect(devOverrideBody.broker_client_auth.warning).toContain(
      'local development',
    )

    const misconfiguredResponse = await request(
      '/v1/health',
      { method: 'GET' },
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
    expect(misconfiguredBody.broker_client_auth.mode).toBe(
      'misconfigured_no_token',
    )
    expect(misconfiguredBody.broker_client_auth.warning).toContain(
      'BROKER_CLIENT_TOKEN',
    )
  })

  it('prunes expired rows when scheduled handler runs', async () => {
    const now = Math.floor(Date.now() / 1000)

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
      .bind('expired-pending-session', now - 120, now - 1)
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
      .bind('expired-oauth-token', now - 100, now - 1)
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
      .bind('active-oauth-token', now - 100, now + 3600)
      .run()

    await env.DB.prepare(
      `INSERT INTO discogs_search_cache (
        cache_key,
        response_json,
        cached_at,
        expires_at
      ) VALUES (?1, ?2, ?3, ?4)`,
    )
      .bind('expired-cache', '{"result":null}', now - 100, now - 1)
      .run()

    await env.DB.prepare(
      `INSERT INTO discogs_search_cache (
        cache_key,
        response_json,
        cached_at,
        expires_at
      ) VALUES (?1, ?2, ?3, ?4)`,
    )
      .bind('active-cache', '{"result":null}', now - 100, now + 3600)
      .run()

    const ctx = createExecutionContext()
    const controller = {
      cron: '0 * * * *',
      scheduledTime: Date.now(),
      noRetry() {},
    } as ScheduledController

    await worker.scheduled(controller, env, ctx)
    await waitOnExecutionContext(ctx)

    const remainingDeviceRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM device_sessions`,
    )
      .first<{ count: number }>()
    expect(Number(remainingDeviceRows?.count ?? 0)).toBe(1)

    const remainingOauthRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM oauth_request_tokens`,
    )
      .first<{ count: number }>()
    expect(Number(remainingOauthRows?.count ?? 0)).toBe(1)

    const remainingCacheRows = await env.DB.prepare(
      `SELECT COUNT(*) AS count
       FROM discogs_search_cache`,
    )
      .first<{ count: number }>()
    expect(Number(remainingCacheRows?.count ?? 0)).toBe(1)
  })

  it('sanitizes missing Discogs auth configuration errors', async () => {
    const response = await request(
      '/v1/device/session/start',
      {
        method: 'POST',
        headers: {
          [TOKEN_HEADER]: env.BROKER_CLIENT_TOKEN,
        },
      },
      {
        BROKER_STATE_ENCRYPTION_KEY: '',
      },
    )

    expect(response.status).toBe(503)
    const body = await response.json<{
      error: string
      message: string
    }>()
    expect(body.error).toBe('service_unavailable')
    expect(body.message).toBe(DISCOGS_AUTH_NOT_CONFIGURED_MESSAGE)
    expect(body.message).not.toContain('BROKER_STATE_ENCRYPTION_KEY')
    expect(body.message).not.toContain('DISCOGS_CONSUMER_SECRET')
  })

  it('sanitizes upstream fetch failures for proxy callers', async () => {
    const sessionToken = 'session-upstream-failure-token'
    const sessionTokenHash = await sha256Hex(sessionToken)
    const now = Math.floor(Date.now() / 1000)

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
        ?1, ?2, 'finalized', 5, ?3, ?3, ?4, ?3, ?5, ?6, 'tester', ?7, ?4, ?3
      )`,
    )
      .bind(
        'device-upstream-failure',
        'pending-upstream-failure',
        now,
        now + 3600,
        'access-token',
        'access-secret',
        sessionTokenHash,
      )
      .run()

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockRejectedValue(
      new Error('super secret upstream detail'),
    )

    try {
      const response = await request('/v1/discogs/proxy/search', {
        method: 'POST',
        headers: {
          authorization: `Bearer ${sessionToken}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          artist: 'Failure Artist',
          title: 'Failure Track',
        }),
      })

      expect(response.status).toBe(502)
      const body = await response.json<{
        error: string
        message: string
      }>()
      expect(body.error).toBe('discogs_unavailable')
      expect(body.message).toBe(DISCOGS_UPSTREAM_FAILURE_MESSAGE)
      expect(body.message).not.toContain('super secret upstream detail')
    } finally {
      fetchSpy.mockRestore()
    }
  })
})

describe('discogs proxy search matching', () => {
  async function setupSessionAndSearch(
    sessionToken: string,
    searchBody: { artist: string; title: string; album?: string },
    discogsResults: Array<{
      title?: string
      year?: string | number
      label?: string[]
      genre?: string[]
      style?: string[]
      format?: string[]
      uri?: string
    }>,
  ): Promise<Response> {
    const sessionTokenHash = await sha256Hex(sessionToken)
    const now = Math.floor(Date.now() / 1000)

    await env.DB.prepare(
      `INSERT INTO device_sessions (
        device_id, pending_token, status, poll_interval_seconds,
        created_at, updated_at, expires_at, authorized_at,
        oauth_access_token, oauth_access_token_secret, oauth_identity,
        session_token_hash, session_expires_at, finalized_at
      ) VALUES (
        ?1, ?2, 'finalized', 5, ?3, ?3, ?4, ?3, ?5, ?6, 'tester', ?7, ?4, ?3
      )`,
    )
      .bind(
        `device-match-${sessionToken}`,
        `pending-match-${sessionToken}`,
        now,
        now + 3600,
        'oauth-token',
        'oauth-secret',
        sessionTokenHash,
      )
      .run()

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify({ results: discogsResults }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    )

    try {
      return await request('/v1/discogs/proxy/search', {
        method: 'POST',
        headers: {
          authorization: `Bearer ${sessionToken}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify(searchBody),
      })
    } finally {
      fetchSpy.mockRestore()
    }
  }

  it('returns exact match when artist and title both match', async () => {
    const response = await setupSessionAndSearch(
      'match-exact-both',
      { artist: 'Legowelt', title: 'Disco Rout' },
      [
        {
          title: 'Other Artist - Other Track',
          year: 2020,
          label: ['Label A'],
          genre: ['Electronic'],
          style: ['Techno'],
          uri: '/release/1',
        },
        {
          title: 'Legowelt - Disco Rout',
          year: 2021,
          label: ['Clone'],
          genre: ['Electronic'],
          style: ['Electro'],
          uri: '/release/2',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { title: string; fuzzy_match: boolean }; match_quality: string }
    >()
    expect(body.match_quality).toBe('exact')
    expect(body.result.title).toBe('Legowelt - Disco Rout')
    expect(body.result.fuzzy_match).toBe(false)
  })

  it('returns fuzzy match when artist matches but title does not', async () => {
    const response = await setupSessionAndSearch(
      'match-artist-only',
      { artist: 'Legowelt', title: 'Nonexistent Track' },
      [
        {
          title: 'Legowelt - Disco Rout',
          year: 2021,
          label: ['Clone'],
          genre: ['Electronic'],
          style: ['Electro'],
          uri: '/release/1',
        },
        {
          title: 'Legowelt - Other EP',
          year: 2019,
          label: ['Clone'],
          genre: ['Electronic'],
          style: ['Electro'],
          uri: '/release/2',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { title: string; fuzzy_match: boolean }; match_quality: string }
    >()
    expect(body.match_quality).toBe('fuzzy')
    expect(body.result.title).toBe('Legowelt - Disco Rout')
    expect(body.result.fuzzy_match).toBe(true)
  })

  it('prefers artist+title match over artist-only match', async () => {
    const response = await setupSessionAndSearch(
      'match-prefer-title',
      { artist: 'Shed', title: 'Silent Witness' },
      [
        {
          title: 'Shed - The Killer',
          year: 2008,
          label: ['Ostgut'],
          genre: ['Electronic'],
          style: ['Techno'],
          uri: '/release/1',
        },
        {
          title: 'Shed - Silent Witness',
          year: 2010,
          label: ['Ostgut'],
          genre: ['Electronic'],
          style: ['Dub Techno'],
          uri: '/release/2',
        },
        {
          title: 'Shed - Shedding The Past',
          year: 2012,
          label: ['Ostgut'],
          genre: ['Electronic'],
          style: ['Techno'],
          uri: '/release/3',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      {
        result: { title: string; fuzzy_match: boolean; url: string }
        match_quality: string
      }
    >()
    expect(body.match_quality).toBe('exact')
    expect(body.result.title).toBe('Shed - Silent Witness')
    expect(body.result.fuzzy_match).toBe(false)
  })

  it('returns fuzzy when no artist matches but title matches', async () => {
    const response = await setupSessionAndSearch(
      'match-title-only',
      { artist: 'Misspelled Artst', title: 'Correct Title' },
      [
        {
          title: 'Real Artist - Correct Title',
          year: 2023,
          label: ['Label'],
          genre: ['Electronic'],
          style: ['House'],
          uri: '/release/1',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { title: string; fuzzy_match: boolean }; match_quality: string }
    >()
    expect(body.match_quality).toBe('fuzzy')
    expect(body.result.fuzzy_match).toBe(true)
    expect(body.result.title).toBe('Real Artist - Correct Title')
  })

  it('returns fuzzy first result when nothing matches', async () => {
    const response = await setupSessionAndSearch(
      'match-nothing',
      { artist: 'Unknown', title: 'Nope' },
      [
        {
          title: 'Completely Different - Something Else',
          year: 2020,
          label: ['Label'],
          genre: ['Rock'],
          style: ['Indie'],
          uri: '/release/1',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { fuzzy_match: boolean }; match_quality: string }
    >()
    expect(body.match_quality).toBe('fuzzy')
    expect(body.result.fuzzy_match).toBe(true)
  })

  it('prefers original single over compilation when both match artist+title', async () => {
    const response = await setupSessionAndSearch(
      'score-original-over-comp',
      { artist: 'Aphex Twin', title: 'Xtal' },
      [
        {
          title: 'Aphex Twin - Selected Ambient Works 85-92',
          year: 2006,
          label: ['Apollo'],
          genre: ['Electronic'],
          style: ['Ambient'],
          format: ['CD', 'Album', 'Compilation'],
          uri: '/release/comp',
        },
        {
          title: 'Aphex Twin - Selected Ambient Works 85-92',
          year: 1992,
          label: ['R & S Records'],
          genre: ['Electronic'],
          style: ['Ambient'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string; url: string }; match_quality: string }
    >()
    expect(body.result.label).toBe('R & S Records')
    expect(body.result.url).toContain('/release/original')
  })

  it('prefers earlier year release among identical formats', async () => {
    const response = await setupSessionAndSearch(
      'score-earlier-year',
      { artist: 'Fantastic Man', title: 'Antiboudi' },
      [
        {
          title: 'Fantastic Man - Solar Surfing',
          year: 2020,
          label: ['Mule Musiq'],
          genre: ['Electronic'],
          style: ['House'],
          format: ['Vinyl', 'LP'],
          uri: '/release/reissue',
        },
        {
          title: 'Fantastic Man - Solar Surfing',
          year: 2016,
          label: ['Superconscious Records'],
          genre: ['Electronic'],
          style: ['House'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string }; match_quality: string }
    >()
    expect(body.result.label).toBe('Superconscious Records')
  })

  it('penalizes Various Artists compilations', async () => {
    const response = await setupSessionAndSearch(
      'score-various-artists',
      { artist: 'Moodymann', title: 'The Third Track' },
      [
        {
          title: 'Various - Planet E Compilation',
          year: 2005,
          label: ['Planet E'],
          genre: ['Electronic'],
          style: ['Deep House'],
          format: ['CD', 'Compilation'],
          uri: '/release/various',
        },
        {
          title: 'Moodymann - Silentintroduction',
          year: 1997,
          label: ['KDJ'],
          genre: ['Electronic'],
          style: ['Deep House'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string }; match_quality: string }
    >()
    expect(body.result.label).toBe('KDJ')
  })

  it('prefers EP format over album format when both match', async () => {
    const response = await setupSessionAndSearch(
      'score-ep-over-album',
      { artist: 'Vril', title: 'Sine Fine' },
      [
        {
          title: 'Vril - Portal',
          year: 2018,
          label: ['Delsin'],
          genre: ['Electronic'],
          style: ['Techno'],
          format: ['Vinyl', 'LP', 'Album'],
          uri: '/release/album',
        },
        {
          title: 'Vril - Anima Mundi',
          year: 2015,
          label: ['Giegling'],
          genre: ['Electronic'],
          style: ['Techno'],
          format: ['Vinyl', '12"', 'EP'],
          uri: '/release/ep',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string }; match_quality: string }
    >()
    expect(body.result.label).toBe('Giegling')
  })

  it('prefers original pressing in fuzzy artist-only fallback', async () => {
    const response = await setupSessionAndSearch(
      'score-fuzzy-artist-only',
      { artist: 'Rick Wade', title: 'Nonexistent Track' },
      [
        {
          title: 'Rick Wade - Deep In The Pocket',
          year: 2010,
          label: ['Unknown Season'],
          genre: ['Electronic'],
          style: ['Deep House'],
          format: ['CD', 'Compilation'],
          uri: '/release/comp',
        },
        {
          title: 'Rick Wade - Harmonie Park',
          year: 2001,
          label: ['Harmonie Park'],
          genre: ['Electronic'],
          style: ['Deep House'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string }; match_quality: string }
    >()
    expect(body.match_quality).toBe('fuzzy')
    expect(body.result.label).toBe('Harmonie Park')
  })

  it('breaks ties by array position (first wins)', async () => {
    const response = await setupSessionAndSearch(
      'score-tie-break',
      { artist: 'Shed', title: 'Silent Witness' },
      [
        {
          title: 'Shed - Silent Witness',
          year: 2010,
          label: ['Ostgut Ton'],
          genre: ['Electronic'],
          style: ['Techno'],
          format: ['Vinyl', '12"'],
          uri: '/release/first',
        },
        {
          title: 'Shed - Silent Witness',
          year: 2010,
          label: ['Ostgut Ton Repress'],
          genre: ['Electronic'],
          style: ['Techno'],
          format: ['Vinyl', '12"'],
          uri: '/release/second',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { label: string; url: string } }
    >()
    expect(body.result.label).toBe('Ostgut Ton')
    expect(body.result.url).toContain('/release/first')
  })

  it('compilation penalty dominates even with preferred format present', async () => {
    const response = await setupSessionAndSearch(
      'score-comp-with-preferred',
      { artist: 'Aphex Twin', title: 'Xtal' },
      [
        {
          title: 'Aphex Twin - Selected Ambient Works 85-92',
          year: 1993,
          label: ['Apollo'],
          genre: ['Electronic'],
          style: ['Ambient'],
          format: ['12"', 'Compilation'],
          uri: '/release/comp-vinyl',
        },
        {
          title: 'Aphex Twin - Selected Ambient Works 85-92',
          year: 2006,
          label: ['R & S Records'],
          genre: ['Electronic'],
          style: ['Ambient'],
          format: ['CD', 'Album'],
          uri: '/release/cd-reissue',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<{ result: { label: string } }>()
    expect(body.result.label).toBe('R & S Records')
  })

  it('prefers earlier year when formats are identical', async () => {
    const response = await setupSessionAndSearch(
      'score-year-only',
      { artist: 'Legowelt', title: 'Disco Rout' },
      [
        {
          title: 'Legowelt - Disco Rout',
          year: 2018,
          label: ['Reissue Label'],
          genre: ['Electronic'],
          style: ['Electro'],
          format: ['Vinyl', '12"'],
          uri: '/release/reissue',
        },
        {
          title: 'Legowelt - Disco Rout',
          year: 2005,
          label: ['Clone'],
          genre: ['Electronic'],
          style: ['Electro'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<{ result: { label: string } }>()
    expect(body.result.label).toBe('Clone')
  })

  it('handles year as string correctly', async () => {
    const response = await setupSessionAndSearch(
      'score-string-year',
      { artist: 'Legowelt', title: 'Disco Rout' },
      [
        {
          title: 'Legowelt - Disco Rout',
          year: '2018',
          label: ['Reissue Label'],
          genre: ['Electronic'],
          style: ['Electro'],
          format: ['Vinyl', '12"'],
          uri: '/release/reissue',
        },
        {
          title: 'Legowelt - Disco Rout',
          year: '1995',
          label: ['Clone'],
          genre: ['Electronic'],
          style: ['Electro'],
          format: ['Vinyl', '12"'],
          uri: '/release/original',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<{ result: { label: string } }>()
    expect(body.result.label).toBe('Clone')
  })

  it('scores correctly when nothing matches and multiple results present', async () => {
    const response = await setupSessionAndSearch(
      'score-nothing-matched',
      { artist: 'Unknown', title: 'Nope' },
      [
        {
          title: 'Other - Compilation Hits',
          year: 2020,
          label: ['Comp Label'],
          genre: ['Electronic'],
          style: ['House'],
          format: ['CD', 'Compilation'],
          uri: '/release/comp',
        },
        {
          title: 'Another - Original Single',
          year: 2005,
          label: ['Good Label'],
          genre: ['Electronic'],
          style: ['House'],
          format: ['Vinyl', '12"'],
          uri: '/release/single',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<{ result: { label: string } }>()
    expect(body.result.label).toBe('Good Label')
  })

  it('still picks correct result when no format data is present', async () => {
    const response = await setupSessionAndSearch(
      'score-no-format',
      { artist: 'Shed', title: 'Silent Witness' },
      [
        {
          title: 'Shed - The Killer',
          year: 2008,
          label: ['Ostgut'],
          genre: ['Electronic'],
          style: ['Techno'],
          uri: '/release/1',
        },
        {
          title: 'Shed - Silent Witness',
          year: 2010,
          label: ['Ostgut'],
          genre: ['Electronic'],
          style: ['Dub Techno'],
          uri: '/release/2',
        },
      ],
    )

    expect(response.status).toBe(200)
    const body = await response.json<
      { result: { title: string }; match_quality: string }
    >()
    expect(body.match_quality).toBe('exact')
    expect(body.result.title).toBe('Shed - Silent Witness')
  })
})

describe('discogs proxy rate limiting', () => {
  it('serializes concurrent proxy searches through the global rate-limit gate', async () => {
    const minIntervalMs = 120
    const sessionToken = 'session-rate-limit-token'
    const sessionTokenHash = await sha256Hex(sessionToken)
    const now = Math.floor(Date.now() / 1000)

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
        ?1, ?2, 'finalized', 5, ?3, ?3, ?4, ?3, ?5, ?6, 'tester', ?7, ?4, ?3
      )`,
    )
      .bind(
        'device-rate-limit',
        'pending-rate-limit',
        now,
        now + 3600,
        'oauth-access-token',
        'oauth-access-secret',
        sessionTokenHash,
      )
      .run()

    const discogsCallTimes: number[] = []
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockImplementation(
      async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input.toString()
        if (!url.startsWith('https://api.discogs.com/database/search?')) {
          throw new Error(`unexpected fetch URL: ${url}`)
        }
        discogsCallTimes.push(Date.now())
        return new Response(
          JSON.stringify({
            results: [
              {
                title: 'Test Artist - Test Track',
                year: 2025,
                label: ['Test Label'],
                genre: ['Electronic'],
                style: ['House'],
                uri: '/release/123',
              },
            ],
          }),
          {
            status: 200,
            headers: {
              'content-type': 'application/json',
            },
          },
        )
      },
    )

    try {
      const titles = ['Track A', 'Track B', 'Track C']
      const startedAt = Date.now()
      const responses = await Promise.all(
        titles.map((title) =>
          request(
            '/v1/discogs/proxy/search',
            {
              method: 'POST',
              headers: {
                authorization: `Bearer ${sessionToken}`,
                'content-type': 'application/json',
              },
              body: JSON.stringify({
                artist: 'Test Artist',
                title,
              }),
            },
            {
              DISCOGS_MIN_INTERVAL_MS: `${minIntervalMs}`,
            },
          )
        ),
      )
      const elapsedMs = Date.now() - startedAt

      expect(responses).toHaveLength(3)
      for (const response of responses) {
        expect(response.status).toBe(200)
      }
      expect(discogsCallTimes).toHaveLength(3)

      const gapOneMs = discogsCallTimes[1] - discogsCallTimes[0]
      const gapTwoMs = discogsCallTimes[2] - discogsCallTimes[1]
      expect(gapOneMs).toBeGreaterThanOrEqual(minIntervalMs - 25)
      expect(gapTwoMs).toBeGreaterThanOrEqual(minIntervalMs - 25)
      expect(elapsedMs).toBeGreaterThanOrEqual((minIntervalMs * 2) - 25)
    } finally {
      fetchSpy.mockRestore()
    }
  })
})

function request(
  path: string,
  init: RequestInit,
  overrides?: Partial<typeof env>,
): Promise<Response> {
  const req = new Request(`${BASE_URL}${path}`, init)
  const runtimeEnv = overrides ? ({ ...env, ...overrides } as typeof env) : env
  return worker.fetch(req, runtimeEnv)
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

async function hmacSha256Hex(key: string, input: string): Promise<string> {
  const encoder = new TextEncoder()
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    encoder.encode(key),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  )
  const digest = await crypto.subtle.sign(
    'HMAC',
    cryptoKey,
    encoder.encode(input),
  )
  const bytes = new Uint8Array(digest)
  return Array.from(bytes).map((b) => b.toString(16).padStart(2, '0')).join(
    '',
  )
}

async function sha256Hex(input: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(input),
  )
  const bytes = new Uint8Array(digest)
  return Array.from(bytes).map((b) => b.toString(16).padStart(2, '0')).join(
    '',
  )
}
