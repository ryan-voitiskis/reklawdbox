import { env } from 'cloudflare:test'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import worker from '../src/index'

const COOLDOWN = 'discogs-api-cooldown'
const PACING = 'discogs-api-global'

beforeEach(async () => {
  for (
    const table of [
      'device_sessions',
      'oauth_request_tokens',
      'discogs_search_cache',
      'rate_limit_state',
    ]
  ) {
    await env.DB.prepare(`DELETE FROM ${table}`).run()
  }
  const now = Math.floor(Date.now() / 1000)
  const digest = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode('recovery-session'),
  )
  const hash = Array.from(
    new Uint8Array(digest),
    n => n.toString(16).padStart(2, '0'),
  ).join('')
  await env.DB.prepare(`INSERT INTO device_sessions
    (device_id, pending_token, status, poll_interval_seconds, created_at, updated_at,
      expires_at, oauth_access_token, oauth_access_token_secret, session_token_hash, session_expires_at)
    VALUES ('recovery-device', 'recovery-pending', 'finalized', 5, ?1, ?1, ?2,
      'synthetic-token', 'synthetic-secret', ?3, ?2)`)
    .bind(now, now + 3600, hash).run()
})

afterEach(() => vi.restoreAllMocks())

function request(
  path: string,
  init: RequestInit,
  overrides: Partial<typeof env> = {},
) {
  return worker.fetch(new Request(`https://broker.test${path}`, init), {
    ...env,
    DISCOGS_MIN_INTERVAL_MS: '1',
    ...overrides,
  })
}

function search(title: string, overrides: Partial<typeof env> = {}) {
  return request('/v1/discogs/proxy/search', {
    method: 'POST',
    headers: {
      Authorization: 'Bearer recovery-session',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ artist: 'Recovery Artist', title }),
  }, overrides)
}

async function setDeadline(bucket: string, deadlineMs: number) {
  await env.DB.prepare(
    `INSERT INTO rate_limit_state (bucket, last_request_at_ms)
    VALUES (?1, ?2) ON CONFLICT(bucket) DO UPDATE SET last_request_at_ms = excluded.last_request_at_ms`,
  )
    .bind(bucket, deadlineMs).run()
}

it('shares cooldowns across search and OAuth, serves cached results, and recovers after expiry', async () => {
  const upstream = vi.spyOn(globalThis, 'fetch')
    .mockResolvedValueOnce(
      new Response('', { status: 429, headers: { 'Retry-After': '180' } }),
    )
    .mockResolvedValueOnce(new Response(JSON.stringify({ results: [] })))
  const first = await search('First')
  expect(first.status).toBe(429)
  expect(first.headers.get('Retry-After')).toBe('180')
  const second = await search('Second')
  expect(second.status).toBe(429)
  expect(Number(second.headers.get('Retry-After'))).toBeGreaterThanOrEqual(179)
  expect(upstream).toHaveBeenCalledTimes(1)

  const start = await request('/v1/device/session/start', {
    method: 'POST',
    headers: { 'x-reklawdbox-broker-token': env.BROKER_CLIENT_TOKEN },
  })
  const started = await start.json<{ auth_url: string }>()
  const link = new URL(started.auth_url)
  expect((await request(link.pathname + link.search, {})).status).toBe(429)
  expect(upstream).toHaveBeenCalledTimes(1)

  const now = Math.floor(Date.now() / 1000)
  await env.DB.prepare(
    `INSERT INTO discogs_search_cache (cache_key, response_json, cached_at, expires_at)
    VALUES ('recovery artist|cached|', ?1, ?2, ?3)`,
  )
    .bind(
      JSON.stringify({ result: null, match_quality: 'none', cache_hit: false }),
      now,
      now + 3600,
    ).run()
  const cached = await search('Cached')
  expect(cached.status).toBe(200)
  expect(await cached.json()).toMatchObject({ result: null, cache_hit: true })
  expect(upstream).toHaveBeenCalledTimes(1)

  await setDeadline(COOLDOWN, Date.now() - 1)
  const recovered = await search('Second')
  expect(recovered.status).toBe(200)
  expect(await recovered.json()).toMatchObject({
    result: null,
    cache_hit: false,
  })
  expect(upstream).toHaveBeenCalledTimes(2)
})

it('rejects an excessive queue promptly without growing the backlog or calling Discogs', async () => {
  const reserved = Date.now() + 31000
  await setDeadline(PACING, reserved)
  const upstream = vi.spyOn(globalThis, 'fetch')
  const started = Date.now()
  const response = await search('Queued', { DISCOGS_MIN_INTERVAL_MS: '1100' })
  expect(response.status).toBe(503)
  expect(response.headers.get('Cache-Control')).toBe('no-store')
  expect(Number(response.headers.get('Retry-After'))).toBeGreaterThanOrEqual(31)
  expect(await response.json()).toMatchObject({ error: 'broker_busy' })
  expect(Date.now() - started).toBeLessThan(1000)
  expect(upstream).not.toHaveBeenCalled()
  const row = await env.DB.prepare(
    'SELECT last_request_at_ms FROM rate_limit_state WHERE bucket = ?1',
  )
    .bind(PACING).first<{ last_request_at_ms: number }>()
  expect(row?.last_request_at_ms).toBe(reserved)
})

it('rechecks cooldowns after an already queued request wakes', async () => {
  await setDeadline(PACING, Date.now() + 300)
  const upstream = vi.spyOn(globalThis, 'fetch')
  const pending = search('Already queued')
  await new Promise(resolve => setTimeout(resolve, 75))
  await setDeadline(COOLDOWN, Date.now() + 60000)
  const response = await pending
  expect(response.status).toBe(429)
  expect(upstream).not.toHaveBeenCalled()
})

it('keeps the longest cooldown when in-flight upstream responses finish out of order', async () => {
  const replies: Array<(response: Response) => void> = []
  const upstream = vi.spyOn(globalThis, 'fetch').mockImplementation(() =>
    new Promise<Response>(resolve => replies.push(resolve))
  )
  const first = search('Concurrent A')
  const second = search('Concurrent B')
  await vi.waitFor(() => expect(upstream).toHaveBeenCalledTimes(2))
  replies[0](
    new Response('', { status: 429, headers: { 'Retry-After': '180' } }),
  )
  await vi.waitFor(async () => {
    const row = await env.DB.prepare(
      'SELECT last_request_at_ms FROM rate_limit_state WHERE bucket = ?1',
    )
      .bind(COOLDOWN).first<{ last_request_at_ms: number }>()
    expect(Number(row?.last_request_at_ms)).toBeGreaterThan(Date.now() + 179000)
  })
  replies[1](
    new Response('', { status: 429, headers: { 'Retry-After': '60' } }),
  )
  expect((await first).status).toBe(429)
  const secondResponse = await second
  expect(secondResponse.status).toBe(429)
  expect(Number(secondResponse.headers.get('Retry-After')))
    .toBeGreaterThanOrEqual(179)
  const row = await env.DB.prepare(
    'SELECT last_request_at_ms FROM rate_limit_state WHERE bucket = ?1',
  )
    .bind(COOLDOWN).first<{ last_request_at_ms: number }>()
  expect(Number(row?.last_request_at_ms)).toBeGreaterThan(Date.now() + 179000)
})

it('routes the complete OAuth and search flow through the configured egress binding', async () => {
  const direct = vi.spyOn(globalThis, 'fetch')
  const routed = vi.fn()
    .mockResolvedValueOnce(
      new Response('oauth_token=req-token&oauth_token_secret=req-secret'),
    )
    .mockResolvedValueOnce(
      new Response('oauth_token=access-token&oauth_token_secret=access-secret'),
    )
    .mockResolvedValueOnce(new Response(JSON.stringify({ results: [] })))
  const overrides = { DISCOGS_EGRESS: { fetch: routed } as unknown as Fetcher }
  const start = await request('/v1/device/session/start', {
    method: 'POST',
    headers: { 'x-reklawdbox-broker-token': env.BROKER_CLIENT_TOKEN },
  }, overrides)
  const started = await start.json<
    { auth_url: string; device_id: string; pending_token: string }
  >()
  const link = new URL(started.auth_url)
  expect((await request(link.pathname + link.search, {}, overrides)).status)
    .toBe(302)
  const callback = '/v1/discogs/oauth/callback' + link.search
    + '&oauth_token=req-token&oauth_verifier=verifier'
  expect((await request(callback, {}, overrides)).status).toBe(200)
  const finalized = await request('/v1/device/session/finalize', {
    method: 'POST',
    headers: {
      'x-reklawdbox-broker-token': env.BROKER_CLIENT_TOKEN,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      device_id: started.device_id,
      pending_token: started.pending_token,
    }),
  }, overrides)
  expect(finalized.status).toBe(200)
  const session = await finalized.json<{ session_token: string }>()
  const result = await request('/v1/discogs/proxy/search', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${session.session_token}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ artist: 'Gateway Artist', title: 'Gateway Track' }),
  }, overrides)
  expect(result.status).toBe(200)
  expect(routed.mock.calls.map(([url]) => new URL(url).pathname)).toEqual([
    '/oauth/request_token',
    '/oauth/access_token',
    '/database/search',
  ])
  expect(direct).not.toHaveBeenCalled()
  const health = await request('/v1/health', {}, overrides)
  expect(await health.json()).toMatchObject({
    status: 'ok',
    discogs_egress: 'gateway',
  })
})

it('does not fall back to direct Worker egress when the configured route fails', async () => {
  const direct = vi.spyOn(globalThis, 'fetch')
  const routed = vi.fn().mockRejectedValue(
    new Error('synthetic egress failure'),
  )
  const response = await search('Failure', {
    DISCOGS_EGRESS: { fetch: routed } as unknown as Fetcher,
  })
  expect(response.status).toBe(502)
  expect(await response.json()).toMatchObject({ error: 'discogs_unavailable' })
  expect(routed).toHaveBeenCalledTimes(1)
  expect(direct).not.toHaveBeenCalled()
})
