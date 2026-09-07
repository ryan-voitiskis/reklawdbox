const MAX_AUTOMATIC_WAIT_SECONDS = 60
const DEFAULT_RETRY_AFTER_SECONDS = 60

type DiscogsStage = 'request_token' | 'access_token' | 'search'

export class DiscogsRateLimitError extends Error {
  constructor(readonly retryAfterSeconds: number) {
    super(
      `Discogs is rate limiting requests from this broker. Wait ${retryAfterSeconds} seconds before retrying.`,
    )
    this.name = 'DiscogsRateLimitError'
  }
}

// Retry only an explicit rejection, never a timeout or an ambiguous token exchange.
export async function withDiscogsRateLimitRecovery(
  stage: DiscogsStage,
  send: () => Promise<Response>,
): Promise<Response> {
  for (let attempt = 0;; attempt++) {
    const response = await send()
    if (response.status !== 429) return response

    const retryAfterSeconds =
      parseRetryAfterSeconds(response.headers.get('Retry-After'))
        ?? DEFAULT_RETRY_AFTER_SECONDS
    console.warn('discogs rate limit', {
      stage,
      retry_after_seconds: retryAfterSeconds,
      limit: numericHeader(response.headers.get('X-Discogs-Ratelimit')),
      remaining: numericHeader(
        response.headers.get('X-Discogs-Ratelimit-Remaining'),
      ),
      used: numericHeader(response.headers.get('X-Discogs-Ratelimit-Used')),
    })
    await response.body?.cancel()

    // MCP clients have a 30-second total request deadline. Return the retry
    // instruction immediately for searches instead of hiding it behind a wait.
    if (
      stage === 'search' || attempt > 0
      || retryAfterSeconds > MAX_AUTOMATIC_WAIT_SECONDS
    ) {
      throw new DiscogsRateLimitError(retryAfterSeconds)
    }
    await new Promise((resolve) =>
      setTimeout(resolve, retryAfterSeconds * 1000)
    )
  }
}

function numericHeader(value: string | null): number | null {
  if (value === null || !/^\d+$/.test(value.trim())) return null
  const parsed = Number(value)
  return Number.isSafeInteger(parsed) ? parsed : null
}

function parseRetryAfterSeconds(value: string | null): number | null {
  if (!value?.trim()) return null
  const seconds = numericHeader(value)
  if (seconds !== null) return seconds
  // Numeric-looking malformed values must not be interpreted as dates.
  if (!/^[A-Za-z]{3},/.test(value.trim())) return null
  const deadline = Date.parse(value)
  return Number.isFinite(deadline)
    ? Math.max(0, Math.ceil((deadline - Date.now()) / 1000))
    : null
}
