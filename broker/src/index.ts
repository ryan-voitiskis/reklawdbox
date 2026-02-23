import {
  BERKELEY_MONO_FONT_DATA_URI,
  CALLBACK_LOGO_DATA_URI,
} from './branding'

interface Env {
  DB: D1Database
  DISCOGS_CONSUMER_KEY: string
  DISCOGS_CONSUMER_SECRET: string
  BROKER_PUBLIC_BASE_URL?: string
  BROKER_CLIENT_TOKEN?: string
  DEVICE_SESSION_TTL_SECONDS?: string
  SESSION_TOKEN_TTL_SECONDS?: string
  SEARCH_CACHE_TTL_SECONDS?: string
  DISCOGS_MIN_INTERVAL_MS?: string
}

type SessionStatus = 'pending' | 'authorized' | 'finalized' | 'expired'

interface DeviceSessionRow {
  device_id: string
  pending_token: string
  status: SessionStatus
  poll_interval_seconds: number
  created_at: number
  updated_at: number
  expires_at: number
  authorized_at: number | null
  oauth_access_token: string | null
  oauth_access_token_secret: string | null
  oauth_identity: string | null
  session_token_hash: string | null
  session_expires_at: number | null
  finalized_at: number | null
}

interface DiscogsSearchBody {
  artist: string
  title: string
  album?: string
}

interface DiscogsApiSearchResponse {
  results?: DiscogsApiSearchResult[]
}

interface DiscogsApiSearchResult {
  title?: string
  year?: string | number
  label?: string[]
  genre?: string[]
  style?: string[]
  uri?: string
}

interface DiscogsResult {
  title: string
  year: string
  label: string
  genres: string[]
  styles: string[]
  url: string
  fuzzy_match: boolean
}

interface NormalizedProxyPayload {
  result: DiscogsResult | null
  match_quality: 'exact' | 'fuzzy' | 'none'
  cache_hit: boolean
}

const DEFAULT_POLL_INTERVAL_SECONDS = 5
const DEFAULT_DEVICE_SESSION_TTL_SECONDS = 15 * 60
const DEFAULT_BROKER_SESSION_TTL_SECONDS = 30 * 24 * 60 * 60
const DEFAULT_CACHE_TTL_SECONDS = 7 * 24 * 60 * 60
const DEFAULT_DISCOGS_MIN_INTERVAL_MS = 1100
const DISCOGS_BASE_URL = 'https://api.discogs.com'

class BrokerHttpError extends Error {
  readonly error: string
  readonly status: number

  constructor(error: string, message: string, status: number,) {
    super(message,)
    this.name = 'BrokerHttpError'
    this.error = error
    this.status = status
  }
}

export default {
  async fetch(request: Request, env: Env,): Promise<Response> {
    try {
      const url = new URL(request.url,)
      const method = request.method.toUpperCase()

      if (url.pathname === '/v1/device/session/start' && method === 'POST') {
        return handleDeviceSessionStart(request, env, url,)
      }
      if (url.pathname === '/v1/device/session/status' && method === 'GET') {
        return handleDeviceSessionStatus(request, env, url,)
      }
      if (url.pathname === '/v1/device/session/finalize' && method === 'POST') {
        return handleDeviceSessionFinalize(request, env,)
      }
      if (url.pathname === '/v1/discogs/oauth/link' && method === 'GET') {
        return handleDiscogsOauthLink(env, url,)
      }
      if (url.pathname === '/v1/discogs/oauth/callback' && method === 'GET') {
        return handleDiscogsOauthCallback(env, url,)
      }
      if (url.pathname === '/v1/discogs/proxy/search' && method === 'POST') {
        return handleDiscogsProxySearch(request, env,)
      }

      return json(
        {
          error: 'not_found',
          message: `No route for ${method} ${url.pathname}`,
        },
        404,
      )
    } catch (err) {
      if (err instanceof BrokerHttpError) {
        return json(
          {
            error: err.error,
            message: err.message,
          },
          err.status,
        )
      }

      return json(
        {
          error: 'internal_error',
          message: asErrorMessage(err,),
        },
        500,
      )
    }
  },
}

async function handleDeviceSessionStart(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  if (!isBrokerClientAuthorized(request, env,)) {
    return unauthorizedBrokerClientResponse()
  }

  assertDiscogsOAuthEnv(env,)

  const now = nowSeconds()
  const pollIntervalSeconds = DEFAULT_POLL_INTERVAL_SECONDS
  const ttlSeconds = envInt(
    env.DEVICE_SESSION_TTL_SECONDS,
    DEFAULT_DEVICE_SESSION_TTL_SECONDS,
  )
  const expiresAt = now + ttlSeconds

  const deviceId = randomToken(20,)
  const pendingToken = randomToken(24,)

  await env.DB.prepare(
    `INSERT INTO device_sessions (
      device_id,
      pending_token,
      status,
      poll_interval_seconds,
      created_at,
      updated_at,
      expires_at
    ) VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6)`,
  )
    .bind(deviceId, pendingToken, pollIntervalSeconds, now, now, expiresAt,)
    .run()

  const authBaseUrl = publicBaseUrl(env, url,)
  const authUrl = `${authBaseUrl}/v1/discogs/oauth/link?device_id=${
    encodeURIComponent(deviceId,)
  }&pending_token=${encodeURIComponent(pendingToken,)}`

  return json({
    device_id: deviceId,
    pending_token: pendingToken,
    auth_url: authUrl,
    poll_interval_seconds: pollIntervalSeconds,
    expires_at: expiresAt,
  },)
}

async function handleDeviceSessionStatus(
  request: Request,
  env: Env,
  url: URL,
): Promise<Response> {
  if (!isBrokerClientAuthorized(request, env,)) {
    return unauthorizedBrokerClientResponse()
  }

  const deviceId = url.searchParams.get('device_id',)?.trim()
  const pendingToken = url.searchParams.get('pending_token',)?.trim()
  if (!deviceId || !pendingToken) {
    return json(
      {
        error: 'invalid_params',
        message: 'device_id and pending_token are required',
      },
      400,
    )
  }

  let row = await getSessionByDeviceAndPending(env, deviceId, pendingToken,)
  if (!row) {
    const latest = await getSessionByDevice(env, deviceId,)
    if (!latest) {
      return json(
        {
          error: 'not_found',
          message: 'device session not found',
        },
        404,
      )
    }

    const replay = await finalizedReplayForPending(
      latest,
      deviceId,
      pendingToken,
    )
    if (!replay) {
      return json(
        {
          error: 'not_found',
          message: 'device session not found',
        },
        404,
      )
    }

    row = latest
  }

  const now = nowSeconds()
  let status = row.status
  if (now >= row.expires_at && status !== 'finalized') {
    status = 'expired'
    await markSessionStatus(env, row.device_id, 'expired', now,)
  }

  return json({
    status,
    expires_at: row.expires_at,
  },)
}

async function handleDeviceSessionFinalize(
  request: Request,
  env: Env,
): Promise<Response> {
  if (!isBrokerClientAuthorized(request, env,)) {
    return unauthorizedBrokerClientResponse()
  }

  const body = await parseJsonBody<
    { device_id?: string; pending_token?: string }
  >(request,)
  const deviceId = body.device_id?.trim()
  const pendingToken = body.pending_token?.trim()

  if (!deviceId || !pendingToken) {
    return json(
      {
        error: 'invalid_params',
        message: 'device_id and pending_token are required',
      },
      400,
    )
  }

  const row = await getSessionByDeviceAndPending(env, deviceId, pendingToken,)
  if (!row) {
    const latest = await getSessionByDevice(env, deviceId,)
    if (!latest) {
      return json(
        {
          error: 'not_found',
          message: 'device session not found',
        },
        404,
      )
    }

    const replay = await finalizedReplayForPending(
      latest,
      deviceId,
      pendingToken,
    )
    if (replay) {
      return json({
        session_token: replay.sessionToken,
        expires_at: replay.sessionExpiresAt,
      },)
    }

    return json(
      {
        error: 'not_found',
        message: 'device session not found',
      },
      404,
    )
  }

  const now = nowSeconds()
  if (now >= row.expires_at) {
    await markSessionStatus(env, row.device_id, 'expired', now,)
    return json(
      {
        error: 'expired',
        message: 'device session expired; restart auth',
      },
      410,
    )
  }

  if (row.status === 'finalized') {
    const replay = await finalizedReplayForPending(row, deviceId, pendingToken,)
    if (replay) {
      return json({
        session_token: replay.sessionToken,
        expires_at: replay.sessionExpiresAt,
      },)
    }

    return json(
      {
        error: 'already_finalized',
        message: 'device session has already been finalized',
      },
      409,
    )
  }

  if (row.status !== 'authorized') {
    return json(
      {
        error: 'not_ready',
        message: 'device session is not authorized yet',
        status: row.status,
      },
      409,
    )
  }

  if (!row.oauth_access_token || !row.oauth_access_token_secret) {
    return json(
      {
        error: 'not_ready',
        message: 'discogs token exchange has not completed',
      },
      409,
    )
  }

  const sessionToken = await deriveFinalizeSessionToken(row, deviceId, pendingToken,)
  if (!sessionToken) {
    return json(
      {
        error: 'not_ready',
        message: 'discogs token exchange has not completed',
      },
      409,
    )
  }
  const sessionTokenHash = await sha256Hex(sessionToken,)
  const sessionTtl = envInt(
    env.SESSION_TOKEN_TTL_SECONDS,
    DEFAULT_BROKER_SESSION_TTL_SECONDS,
  )
  const sessionExpiresAt = now + sessionTtl
  const nextPendingToken = randomToken(24,)

  const finalizeResult = await env.DB.prepare(
    `UPDATE device_sessions
     SET status = 'finalized',
         session_token_hash = ?1,
         session_expires_at = ?2,
         finalized_at = ?3,
         pending_token = ?4,
         updated_at = ?3
     WHERE device_id = ?5 AND pending_token = ?6 AND status = 'authorized'`,
  )
    .bind(
      sessionTokenHash,
      sessionExpiresAt,
      now,
      nextPendingToken,
      deviceId,
      pendingToken,
    )
    .run()

  const updated = Number(finalizeResult.meta.changes ?? 0,)
  if (updated !== 1) {
    const latest = await getSessionByDevice(env, deviceId,)
    if (!latest) {
      return json(
        {
          error: 'not_found',
          message: 'device session not found',
        },
        404,
      )
    }

    const currentNow = nowSeconds()
    if (latest.status === 'expired' || currentNow >= latest.expires_at) {
      if (latest.status !== 'expired') {
        await markSessionStatus(env, latest.device_id, 'expired', currentNow,)
      }
      return json(
        {
          error: 'expired',
          message: 'device session expired; restart auth',
        },
        410,
      )
    }

    if (latest.status === 'finalized') {
      const replay = await finalizedReplayForPending(
        latest,
        deviceId,
        pendingToken,
      )
      if (replay) {
        return json({
          session_token: replay.sessionToken,
          expires_at: replay.sessionExpiresAt,
        },)
      }

      return json(
        {
          error: 'already_finalized',
          message: 'device session has already been finalized',
        },
        409,
      )
    }

    return json(
      {
        error: 'not_ready',
        message: 'device session is not authorized yet',
        status: latest.status,
      },
      409,
    )
  }

  return json({
    session_token: sessionToken,
    expires_at: sessionExpiresAt,
  },)
}

async function handleDiscogsOauthLink(env: Env, url: URL,): Promise<Response> {
  assertDiscogsOAuthEnv(env,)

  const deviceId = url.searchParams.get('device_id',)?.trim()
  const pendingToken = url.searchParams.get('pending_token',)?.trim()
  if (!deviceId || !pendingToken) {
    return text('Missing device_id or pending_token', 400,)
  }

  let row = await getSessionByDeviceAndPending(env, deviceId, pendingToken,)
  if (!row) {
    const latest = await getSessionByDevice(env, deviceId,)
    if (!latest) {
      return text('Device session not found', 404,)
    }

    const replay = await finalizedReplayForPending(
      latest,
      deviceId,
      pendingToken,
    )
    if (!replay) {
      return text('Device session not found', 404,)
    }

    row = latest
  }

  const now = nowSeconds()
  if (now >= row.expires_at) {
    await markSessionStatus(env, row.device_id, 'expired', now,)
    return text('Device session expired. Restart auth from your client.', 410,)
  }

  if (row.status === 'finalized') {
    return html(
      oauthCallbackPage(
        'Already linked',
        'This device is already linked. Return to your client.',
      ),
      200,
    )
  }

  const callbackBase = publicBaseUrl(env, url,)
  const callbackUrl = `${callbackBase}/v1/discogs/oauth/callback?device_id=${
    encodeURIComponent(deviceId,)
  }&pending_token=${encodeURIComponent(pendingToken,)}`

  const requestToken = await requestDiscogsRequestToken(env, callbackUrl,)
  await env.DB.prepare(
    `INSERT INTO oauth_request_tokens (
      oauth_token,
      oauth_token_secret,
      device_id,
      pending_token,
      created_at,
      expires_at
    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT(oauth_token) DO UPDATE SET
      oauth_token_secret = excluded.oauth_token_secret,
      device_id = excluded.device_id,
      pending_token = excluded.pending_token,
      created_at = excluded.created_at,
      expires_at = excluded.expires_at`,
  )
    .bind(
      requestToken.oauthToken,
      requestToken.oauthTokenSecret,
      deviceId,
      pendingToken,
      now,
      now
        + envInt(
          env.DEVICE_SESSION_TTL_SECONDS,
          DEFAULT_DEVICE_SESSION_TTL_SECONDS,
        ),
    )
    .run()

  const authorizeUrl = `https://www.discogs.com/oauth/authorize?oauth_token=${
    encodeURIComponent(requestToken.oauthToken,)
  }`
  return Response.redirect(authorizeUrl, 302,)
}

async function handleDiscogsOauthCallback(
  env: Env,
  url: URL,
): Promise<Response> {
  assertDiscogsOAuthEnv(env,)

  const deviceId = url.searchParams.get('device_id',)?.trim()
  const pendingToken = url.searchParams.get('pending_token',)?.trim()
  const oauthToken = url.searchParams.get('oauth_token',)?.trim()
  const oauthVerifier = url.searchParams.get('oauth_verifier',)?.trim()

  if (!deviceId || !pendingToken || !oauthToken || !oauthVerifier) {
    return html(
      oauthCallbackPage(
        'Auth failed',
        'Missing required callback parameters. Restart auth from your client.',
      ),
      400,
    )
  }

  const session = await getSessionByDeviceAndPending(
    env,
    deviceId,
    pendingToken,
  )
  if (!session) {
    return html(
      oauthCallbackPage('Auth failed', 'Device session not found.',),
      404,
    )
  }

  const now = nowSeconds()
  if (now >= session.expires_at) {
    await markSessionStatus(env, session.device_id, 'expired', now,)
    return html(
      oauthCallbackPage(
        'Auth expired',
        'The device session expired. Restart auth from your client.',
      ),
      410,
    )
  }

  const temp = await env.DB.prepare(
    `SELECT oauth_token_secret
     FROM oauth_request_tokens
     WHERE oauth_token = ?1 AND device_id = ?2 AND pending_token = ?3 AND expires_at > ?4`,
  )
    .bind(oauthToken, deviceId, pendingToken, now,)
    .first<{ oauth_token_secret: string }>()

  if (!temp) {
    return html(
      oauthCallbackPage(
        'Auth failed',
        'OAuth request token was not found or expired. Restart auth.',
      ),
      400,
    )
  }

  const access = await requestDiscogsAccessToken(
    env,
    oauthToken,
    temp.oauth_token_secret,
    oauthVerifier,
  )

  await env.DB.prepare(
    `UPDATE device_sessions
     SET status = 'authorized',
         oauth_access_token = ?1,
         oauth_access_token_secret = ?2,
         oauth_identity = ?3,
         authorized_at = ?4,
         updated_at = ?4
     WHERE device_id = ?5 AND pending_token = ?6`,
  )
    .bind(
      access.oauthToken,
      access.oauthTokenSecret,
      access.username ?? access.userId ?? null,
      now,
      deviceId,
      pendingToken,
    )
    .run()

  await env.DB.prepare(
    'DELETE FROM oauth_request_tokens WHERE oauth_token = ?1',
  )
    .bind(oauthToken,)
    .run()

  return html(
    oauthCallbackPage(
      'Discogs linked',
      'You can close this tab and return to your client.',
    ),
    200,
  )
}

async function handleDiscogsProxySearch(
  request: Request,
  env: Env,
): Promise<Response> {
  assertDiscogsOAuthEnv(env,)

  const sessionToken = bearerToken(request,)
  if (!sessionToken) {
    return json(
      {
        error: 'unauthorized',
        message: 'missing bearer session token',
      },
      401,
    )
  }

  const sessionTokenHash = await sha256Hex(sessionToken,)
  const now = nowSeconds()
  const session = await env.DB.prepare(
    `SELECT *
     FROM device_sessions
     WHERE session_token_hash = ?1
       AND session_expires_at > ?2
       AND status = 'finalized'
     LIMIT 1`,
  )
    .bind(sessionTokenHash, now,)
    .first<DeviceSessionRow>()

  if (
    !session || !session.oauth_access_token
    || !session.oauth_access_token_secret
  ) {
    return json(
      {
        error: 'unauthorized',
        message: 'invalid or expired broker session',
      },
      401,
    )
  }

  const body = await parseJsonBody<DiscogsSearchBody>(request,)
  const artist = body.artist?.trim()
  const title = body.title?.trim()
  const album = body.album?.trim()

  if (!artist || !title) {
    return json(
      {
        error: 'invalid_params',
        message: 'artist and title are required',
      },
      400,
    )
  }

  const cacheKey = `${normalize(artist,)}|${normalize(title,)}|${
    normalize(album ?? '',)
  }`
  const cached = await env.DB.prepare(
    `SELECT response_json
     FROM discogs_search_cache
     WHERE cache_key = ?1 AND expires_at > ?2`,
  )
    .bind(cacheKey, now,)
    .first<{ response_json: string }>()

  if (cached?.response_json) {
    const parsed = safeJsonParse<NormalizedProxyPayload>(cached.response_json,)
    if (parsed) {
      return json({
        ...parsed,
        cache_hit: true,
      },)
    }
  }

  const payload = await lookupDiscogsViaApi(env, {
    artist,
    title,
    album,
    oauthToken: session.oauth_access_token,
    oauthTokenSecret: session.oauth_access_token_secret,
  },)

  const cacheTtlSeconds = envInt(
    env.SEARCH_CACHE_TTL_SECONDS,
    DEFAULT_CACHE_TTL_SECONDS,
  )
  await env.DB.prepare(
    `INSERT INTO discogs_search_cache (cache_key, response_json, cached_at, expires_at)
     VALUES (?1, ?2, ?3, ?4)
     ON CONFLICT(cache_key) DO UPDATE SET
       response_json = excluded.response_json,
       cached_at = excluded.cached_at,
       expires_at = excluded.expires_at`,
  )
    .bind(cacheKey, JSON.stringify(payload,), now, now + cacheTtlSeconds,)
    .run()

  return json(payload,)
}

async function lookupDiscogsViaApi(
  env: Env,
  params: {
    artist: string
    title: string
    album?: string
    oauthToken: string
    oauthTokenSecret: string
  },
): Promise<NormalizedProxyPayload> {
  const query = new URLSearchParams()
  query.set('artist', params.artist,)
  query.set('track', params.title,)
  query.set('type', 'release',)
  query.set('per_page', '15',)
  if (params.album) {
    query.set('release_title', params.album,)
  }

  const doRequest = async (): Promise<Response> => {
    await enforceDiscogsRateLimit(env,)

    const oauthParams: Record<string, string> = {
      oauth_consumer_key: env.DISCOGS_CONSUMER_KEY,
      oauth_nonce: randomToken(16,),
      oauth_signature_method: 'PLAINTEXT',
      oauth_timestamp: `${Math.floor(Date.now() / 1000,)}`,
      oauth_token: params.oauthToken,
      oauth_version: '1.0',
      oauth_signature:
        `${env.DISCOGS_CONSUMER_SECRET}&${params.oauthTokenSecret}`,
    }

    return fetch(`${DISCOGS_BASE_URL}/database/search?${query.toString()}`, {
      method: 'GET',
      headers: {
        Authorization: oauthHeader(oauthParams,),
        'User-Agent': 'reklawdbox-broker/0.1',
      },
    },)
  }

  let response = await doRequest()
  if (response.status === 429) {
    const retryAfterSeconds = parseRetryAfterSeconds(
      response.headers.get('Retry-After',),
    )
    await delay((retryAfterSeconds ?? 30) * 1000,)
    response = await doRequest()
  }

  if (!response.ok) {
    throw new Error(`Discogs search failed: HTTP ${response.status}`,)
  }

  const data = (await response.json()) as DiscogsApiSearchResponse
  const results = data.results ?? []
  if (results.length === 0) {
    return {
      result: null,
      match_quality: 'none',
      cache_hit: false,
    }
  }

  const normArtist = normalize(params.artist,)
  const matched = results.find((entry,) => {
    const resultTitle = (entry.title ?? '').toLowerCase()
    return resultTitle.includes(normArtist,) || normArtist.length < 3
  },)

  if (matched) {
    return {
      result: toDiscogsResult(matched, false,),
      match_quality: 'exact',
      cache_hit: false,
    }
  }

  return {
    result: toDiscogsResult(results[0], true,),
    match_quality: 'fuzzy',
    cache_hit: false,
  }
}

function toDiscogsResult(
  entry: DiscogsApiSearchResult,
  fuzzy: boolean,
): DiscogsResult {
  const uri = entry.uri ?? ''
  const url = uri ? `https://www.discogs.com${uri}` : ''
  return {
    title: entry.title ?? '',
    year: `${entry.year ?? ''}`,
    label: Array.isArray(entry.label,) && entry.label.length > 0
      ? entry.label[0]
      : '',
    genres: Array.isArray(entry.genre,) ? entry.genre : [],
    styles: Array.isArray(entry.style,) ? entry.style : [],
    url,
    fuzzy_match: fuzzy,
  }
}

async function requestDiscogsRequestToken(
  env: Env,
  callbackUrl: string,
): Promise<{ oauthToken: string; oauthTokenSecret: string }> {
  const oauthParams: Record<string, string> = {
    oauth_callback: callbackUrl,
    oauth_consumer_key: env.DISCOGS_CONSUMER_KEY,
    oauth_nonce: randomToken(16,),
    oauth_signature_method: 'PLAINTEXT',
    oauth_timestamp: `${Math.floor(Date.now() / 1000,)}`,
    oauth_version: '1.0',
    oauth_signature: `${env.DISCOGS_CONSUMER_SECRET}&`,
  }

  const response = await fetch(`${DISCOGS_BASE_URL}/oauth/request_token`, {
    method: 'POST',
    headers: {
      Authorization: oauthHeader(oauthParams,),
      'User-Agent': 'reklawdbox-broker/0.1',
    },
  },)

  const body = await response.text()
  if (!response.ok) {
    throw new Error(`Discogs request_token failed: HTTP ${response.status}`,)
  }

  const params = new URLSearchParams(body,)
  const oauthToken = params.get('oauth_token',) ?? ''
  const oauthTokenSecret = params.get('oauth_token_secret',) ?? ''
  if (!oauthToken || !oauthTokenSecret) {
    throw new Error(
      'Discogs request_token response missing oauth_token fields',
    )
  }

  return { oauthToken, oauthTokenSecret, }
}

async function requestDiscogsAccessToken(
  env: Env,
  oauthToken: string,
  oauthTokenSecret: string,
  oauthVerifier: string,
): Promise<
  {
    oauthToken: string
    oauthTokenSecret: string
    username?: string
    userId?: string
  }
> {
  const oauthParams: Record<string, string> = {
    oauth_consumer_key: env.DISCOGS_CONSUMER_KEY,
    oauth_nonce: randomToken(16,),
    oauth_signature_method: 'PLAINTEXT',
    oauth_timestamp: `${Math.floor(Date.now() / 1000,)}`,
    oauth_token: oauthToken,
    oauth_verifier: oauthVerifier,
    oauth_version: '1.0',
    oauth_signature: `${env.DISCOGS_CONSUMER_SECRET}&${oauthTokenSecret}`,
  }

  const response = await fetch(`${DISCOGS_BASE_URL}/oauth/access_token`, {
    method: 'POST',
    headers: {
      Authorization: oauthHeader(oauthParams,),
      'User-Agent': 'reklawdbox-broker/0.1',
    },
  },)

  const body = await response.text()
  if (!response.ok) {
    throw new Error(`Discogs access_token failed: HTTP ${response.status}`,)
  }

  const params = new URLSearchParams(body,)
  const accessToken = params.get('oauth_token',) ?? ''
  const accessTokenSecret = params.get('oauth_token_secret',) ?? ''
  const username = params.get('username',) ?? undefined
  const userId = params.get('user_id',) ?? undefined

  if (!accessToken || !accessTokenSecret) {
    throw new Error('Discogs access_token response missing oauth_token fields',)
  }

  return {
    oauthToken: accessToken,
    oauthTokenSecret: accessTokenSecret,
    username,
    userId,
  }
}

async function finalizedReplayForPending(
  row: DeviceSessionRow,
  deviceId: string,
  pendingToken: string,
): Promise<{ sessionToken: string; sessionExpiresAt: number } | null> {
  if (
    row.status !== 'finalized'
    || !row.session_token_hash
    || !row.session_expires_at
  ) {
    return null
  }

  const sessionToken = await deriveFinalizeSessionToken(row, deviceId, pendingToken,)
  if (!sessionToken) {
    return null
  }

  const tokenHash = await sha256Hex(sessionToken,)
  if (tokenHash !== row.session_token_hash) {
    return null
  }

  return {
    sessionToken,
    sessionExpiresAt: row.session_expires_at,
  }
}

async function getSessionByDeviceAndPending(
  env: Env,
  deviceId: string,
  pendingToken: string,
): Promise<DeviceSessionRow | null> {
  return env.DB.prepare(
    `SELECT *
     FROM device_sessions
     WHERE device_id = ?1 AND pending_token = ?2
     LIMIT 1`,
  )
    .bind(deviceId, pendingToken,)
    .first<DeviceSessionRow>()
}

async function getSessionByDevice(
  env: Env,
  deviceId: string,
): Promise<DeviceSessionRow | null> {
  return env.DB.prepare(
    `SELECT *
     FROM device_sessions
     WHERE device_id = ?1
     LIMIT 1`,
  )
    .bind(deviceId,)
    .first<DeviceSessionRow>()
}

async function markSessionStatus(
  env: Env,
  deviceId: string,
  status: SessionStatus,
  updatedAt: number,
): Promise<void> {
  await env.DB.prepare(
    `UPDATE device_sessions
     SET status = ?1,
         updated_at = ?2
     WHERE device_id = ?3`,
  )
    .bind(status, updatedAt, deviceId,)
    .run()
}

async function enforceDiscogsRateLimit(env: Env,): Promise<void> {
  const bucket = 'discogs-api-global'
  const minIntervalMs = envInt(
    env.DISCOGS_MIN_INTERVAL_MS,
    DEFAULT_DISCOGS_MIN_INTERVAL_MS,
  )

  for (;;) {
    const nowMs = Date.now()
    const row = await env.DB.prepare(
      `SELECT last_request_at_ms
       FROM rate_limit_state
       WHERE bucket = ?1`,
    )
      .bind(bucket,)
      .first<{ last_request_at_ms: number }>()

    if (!row) {
      const insertResult = await env.DB.prepare(
        `INSERT INTO rate_limit_state (bucket, last_request_at_ms)
         VALUES (?1, ?2)
         ON CONFLICT(bucket) DO NOTHING`,
      )
        .bind(bucket, nowMs,)
        .run()

      if ((insertResult.meta.changes ?? 0) === 1) {
        return
      }
      continue
    }

    const lastRequestAtMs = Number(row.last_request_at_ms,)
    const reservedAtMs = Math.max(lastRequestAtMs + minIntervalMs, nowMs,)
    const updateResult = await env.DB.prepare(
      `UPDATE rate_limit_state
       SET last_request_at_ms = ?1
       WHERE bucket = ?2
         AND last_request_at_ms = ?3`,
    )
      .bind(reservedAtMs, bucket, lastRequestAtMs,)
      .run()

    if ((updateResult.meta.changes ?? 0) === 1) {
      const waitMs = reservedAtMs - nowMs
      if (waitMs > 0) {
        await delay(waitMs,)
      }
      return
    }
  }
}

function assertDiscogsOAuthEnv(env: Env,): void {
  if (!env.DISCOGS_CONSUMER_KEY || !env.DISCOGS_CONSUMER_SECRET) {
    throw new Error(
      'DISCOGS_CONSUMER_KEY and DISCOGS_CONSUMER_SECRET must be set',
    )
  }
}

function publicBaseUrl(env: Env, requestUrl: URL,): string {
  return (env.BROKER_PUBLIC_BASE_URL
    ?? `${requestUrl.protocol}//${requestUrl.host}`).replace(/\/$/, '',)
}

function isBrokerClientAuthorized(request: Request, env: Env,): boolean {
  const expected = env.BROKER_CLIENT_TOKEN?.trim()
  if (!expected) {
    return true
  }

  const provided = request.headers.get('x-reklawdbox-broker-token',)?.trim()
  return provided === expected
}

function unauthorizedBrokerClientResponse(): Response {
  return json(
    {
      error: 'unauthorized',
      message: 'invalid broker client token',
    },
    401,
  )
}

function oauthHeader(params: Record<string, string>,): string {
  const pairs = Object.entries(params,)
    .map(([key, value,],) =>
      `${percentEncode(key,)}="${percentEncode(value,)}"`
    )
    .join(', ',)
  return `OAuth ${pairs}`
}

function percentEncode(value: string,): string {
  return encodeURIComponent(value,)
    .replace(/!/g, '%21',)
    .replace(/\*/g, '%2A',)
    .replace(/\(/g, '%28',)
    .replace(/\)/g, '%29',)
    .replace(/'/g, '%27',)
}

function normalize(input: string,): string {
  return input
    .toLowerCase()
    .split('',)
    .filter((ch,) => /[\p{L}\p{N} ]/u.test(ch,))
    .join('',)
    .trim()
}

async function deriveFinalizeSessionToken(
  row: Pick<DeviceSessionRow, 'oauth_access_token' | 'oauth_access_token_secret'>,
  deviceId: string,
  pendingToken: string,
): Promise<string | null> {
  if (!row.oauth_access_token || !row.oauth_access_token_secret) {
    return null
  }

  const message = `${deviceId}:${pendingToken}:${row.oauth_access_token}`
  const partA = await hmacSha256Hex(
    row.oauth_access_token_secret,
    `broker-session:v1:${message}`,
  )
  const partB = await hmacSha256Hex(
    row.oauth_access_token_secret,
    `broker-session:v1:extra:${message}`,
  )
  return `${partA}${partB.slice(0, 32)}`
}

function randomToken(bytes: number,): string {
  const arr = new Uint8Array(bytes,)
  crypto.getRandomValues(arr,)
  return Array.from(arr,)
    .map((b,) => b.toString(16,).padStart(2, '0',))
    .join('',)
}

async function sha256Hex(input: string,): Promise<string> {
  const encoded = new TextEncoder().encode(input,)
  const digest = await crypto.subtle.digest('SHA-256', encoded,)
  const arr = Array.from(new Uint8Array(digest,),)
  return arr.map((b,) => b.toString(16,).padStart(2, '0',)).join('',)
}

async function hmacSha256Hex(key: string, input: string,): Promise<string> {
  const encoder = new TextEncoder()
  const cryptoKey = await crypto.subtle.importKey(
    'raw',
    encoder.encode(key,),
    { name: 'HMAC', hash: 'SHA-256', },
    false,
    ['sign'],
  )
  const digest = await crypto.subtle.sign('HMAC', cryptoKey, encoder.encode(input,),)
  const arr = Array.from(new Uint8Array(digest,),)
  return arr.map((b,) => b.toString(16,).padStart(2, '0',)).join('',)
}

async function parseJsonBody<T,>(request: Request,): Promise<T> {
  try {
    return (await request.json()) as T
  } catch {
    throw new BrokerHttpError(
      'invalid_json',
      'request body must be valid JSON',
      400,
    )
  }
}

function bearerToken(request: Request,): string | null {
  const auth = request.headers.get('authorization',) ?? ''
  const [scheme, token,] = auth.split(/\s+/, 2,)
  if (!scheme || !token || scheme.toLowerCase() !== 'bearer') {
    return null
  }
  return token.trim()
}

function nowSeconds(): number {
  return Math.floor(Date.now() / 1000,)
}

function envInt(input: string | undefined, fallback: number,): number {
  if (!input) {
    return fallback
  }
  const parsed = Number.parseInt(input, 10,)
  return Number.isFinite(parsed,) && parsed > 0 ? parsed : fallback
}

function parseRetryAfterSeconds(header: string | null,): number | null {
  if (!header) {
    return null
  }
  const parsed = Number.parseInt(header, 10,)
  if (Number.isFinite(parsed,) && parsed > 0) {
    return parsed
  }
  return null
}

function safeJsonParse<T,>(input: string,): T | null {
  try {
    return JSON.parse(input,) as T
  } catch {
    return null
  }
}

function asErrorMessage(err: unknown,): string {
  if (err instanceof Error) {
    return err.message
  }
  return String(err,)
}

function escapeHtml(input: string,): string {
  return input
    .replaceAll('&', '&amp;',)
    .replaceAll('<', '&lt;',)
    .replaceAll('>', '&gt;',)
    .replaceAll('"', '&quot;',)
    .replaceAll("'", '&#39;',)
}

function oauthCallbackPage(title: string, message: string,): string {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${escapeHtml(title,)} | reklawdbox</title>
  <style>
    @font-face {
      font-family: 'BerkeleyMono';
      font-style: normal;
      font-weight: 100 150;
      font-display: swap;
      src: url('${BERKELEY_MONO_FONT_DATA_URI}') format('woff2-variations');
    }
    :root {
      color-scheme: dark;
    }
    * {
      box-sizing: border-box;
      margin: 0;
      padding: 0;
    }
    body {
      min-height: 100vh;
      display: grid;
      place-items: center;
      padding: 24px;
      font-family: 'BerkeleyMono', monospace;
      font-weight: 110;
      background: #0c0c0c;
      color: #e0e0e0;
    }
    .card {
      width: min(520px, 100%);
      border-radius: 16px;
      border: 1px solid rgba(255, 255, 255, 0.06);
      background: rgba(255, 255, 255, 0.04);
      box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
      padding: 40px 32px;
      text-align: center;
    }
    .logo {
      width: 192px;
      height: 192px;
      object-fit: contain;
    }
    h1 {
      margin: 0 0 10px;
      font-size: 22px;
      font-weight: 130;
      line-height: 1.2;
      letter-spacing: -0.01em;
    }
    p {
      margin: 0 auto;
      color: rgba(255, 255, 255, 0.45);
      font-size: 13px;
      line-height: 1.5;
    }
    .brand {
      margin-top: 20px;
      font-size: 11px;
      color: rgba(255, 255, 255, 0.2);
      letter-spacing: 0.03em;
    }
  </style>
</head>
<body>
  <main class="card">
    <img class="logo" src="${CALLBACK_LOGO_DATA_URI}" alt="reklawdbox" />
    <h1>${escapeHtml(title,)}</h1>
    <p>${message.split(/(?<=\.) /,).map(escapeHtml,).join('<br />',)}</p>
    <div class="brand">reklawdbox</div>
  </main>
</body>
</html>`
}

function json(payload: unknown, status = 200,): Response {
  return new Response(JSON.stringify(payload,), {
    status,
    headers: {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
    },
  },)
}

function text(payload: string, status = 200,): Response {
  return new Response(payload, {
    status,
    headers: {
      'content-type': 'text/plain; charset=utf-8',
      'cache-control': 'no-store',
    },
  },)
}

function html(payload: string, status = 200,): Response {
  return new Response(payload, {
    status,
    headers: {
      'content-type': 'text/html; charset=utf-8',
      'cache-control': 'no-store',
    },
  },)
}

async function delay(ms: number,): Promise<void> {
  if (ms <= 0) {
    return
  }
  await new Promise((resolve,) => setTimeout(resolve, ms,))
}
