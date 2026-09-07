import { afterEach, expect, it, vi } from 'vitest'
import {
  DiscogsRateLimitError,
  withDiscogsRateLimitRecovery,
} from '../src/discogs-rate-limit'

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

it.each(['request_token', 'access_token'] as const)(
  'recovers once from a transient 429 at %s, after the requested delay',
  async (stage) => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const send = vi.fn()
      .mockResolvedValueOnce(
        new Response('', { status: 429, headers: { 'Retry-After': '2' } }),
      )
      .mockResolvedValueOnce(new Response('ok'))
    const pending = withDiscogsRateLimitRecovery(stage, send)
    await vi.advanceTimersByTimeAsync(1999)
    expect(send).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect((await pending).status).toBe(200)
    expect(send).toHaveBeenCalledTimes(2)
  },
)

it('stops after a second 429 and preserves its retry instruction', async () => {
  vi.useFakeTimers()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  const send = vi.fn()
    .mockResolvedValueOnce(
      new Response('', { status: 429, headers: { 'Retry-After': '0' } }),
    )
    .mockResolvedValueOnce(
      new Response('', { status: 429, headers: { 'Retry-After': '90' } }),
    )
  const failure = withDiscogsRateLimitRecovery('request_token', send).catch((
    error,
  ) => error)
  await vi.runAllTimersAsync()
  expect(await failure).toBeInstanceOf(DiscogsRateLimitError)
  expect((await failure).retryAfterSeconds).toBe(90)
  expect(send).toHaveBeenCalledTimes(2)
})

it('does not shorten an upstream wait beyond the automatic retry budget', async () => {
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  const send = vi.fn().mockResolvedValue(
    new Response('', {
      status: 429,
      headers: { 'Retry-After': '120' },
    }),
  )
  await expect(withDiscogsRateLimitRecovery('access_token', send)).rejects
    .toMatchObject({
      retryAfterSeconds: 120,
    })
  expect(send).toHaveBeenCalledTimes(1)
})

it('honors an HTTP-date Retry-After', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-09-07T07:00:00Z'))
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  const send = vi.fn()
    .mockResolvedValueOnce(
      new Response('', {
        status: 429,
        headers: { 'Retry-After': 'Mon, 07 Sep 2026 07:00:30 GMT' },
      }),
    )
    .mockResolvedValueOnce(new Response('ok'))
  const pending = withDiscogsRateLimitRecovery('request_token', send)
  await vi.advanceTimersByTimeAsync(29999)
  expect(send).toHaveBeenCalledTimes(1)
  await vi.advanceTimersByTimeAsync(1)
  expect((await pending).status).toBe(200)
})

it.each([null, '-1', '1oops', ''])(
  'uses a conservative fallback for Retry-After %s',
  async (value) => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const send = vi.fn()
      .mockResolvedValueOnce(
        new Response('', {
          status: 429,
          headers: value === null ? {} : { 'Retry-After': value },
        }),
      )
      .mockResolvedValueOnce(new Response('ok'))
    const pending = withDiscogsRateLimitRecovery('request_token', send)
    await vi.advanceTimersByTimeAsync(59999)
    expect(send).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(1)
    expect((await pending).status).toBe(200)
  },
)

it('does not retry an ambiguous network failure during token exchange', async () => {
  const error = new Error('connection lost')
  const send = vi.fn().mockRejectedValue(error)
  await expect(withDiscogsRateLimitRecovery('access_token', send)).rejects.toBe(
    error,
  )
  expect(send).toHaveBeenCalledTimes(1)
})

it.each([401, 403, 500])(
  'does not retry upstream status %s',
  async (status) => {
    const send = vi.fn().mockResolvedValue(new Response('', { status }))
    expect((await withDiscogsRateLimitRecovery('request_token', send)).status)
      .toBe(status)
    expect(send).toHaveBeenCalledTimes(1)
  },
)

it('logs only numeric quota information, never bodies or arbitrary header values', async () => {
  const log = vi.spyOn(console, 'warn').mockImplementation(() => {})
  const send = vi.fn().mockResolvedValue(
    new Response('secret response body', {
      status: 429,
      headers: {
        'Retry-After': '120',
        'X-Discogs-Ratelimit': '60',
        'X-Discogs-Ratelimit-Remaining': '0',
        'X-Discogs-Ratelimit-Used': 'secret header value',
      },
    }),
  )
  await expect(withDiscogsRateLimitRecovery('request_token', send)).rejects
    .toBeInstanceOf(DiscogsRateLimitError)
  expect(log).toHaveBeenCalledWith('discogs rate limit', {
    stage: 'request_token',
    retry_after_seconds: 120,
    limit: 60,
    remaining: 0,
    used: null,
  })
  expect(JSON.stringify(log.mock.calls)).not.toContain('secret')
})

it('returns search throttling immediately within the MCP client deadline', async () => {
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  const send = vi.fn().mockResolvedValue(
    new Response('', { status: 429, headers: { 'Retry-After': '30' } }),
  )
  await expect(withDiscogsRateLimitRecovery('search', send)).rejects
    .toMatchObject({ retryAfterSeconds: 30 })
  expect(send).toHaveBeenCalledTimes(1)
})
