import { applyD1Migrations, env } from 'cloudflare:test'
import { beforeEach } from 'vitest'

declare module 'cloudflare:test' {
  interface ProvidedEnv {
    DB: D1Database
    TEST_MIGRATIONS: unknown
    DISCOGS_CONSUMER_KEY: string
    DISCOGS_CONSUMER_SECRET: string
    BROKER_CLIENT_TOKEN: string
    BROKER_PUBLIC_BASE_URL: string
    ALLOW_UNAUTHENTICATED_BROKER?: string
  }
}

beforeEach(async () => {
  await applyD1Migrations(env.DB, env.TEST_MIGRATIONS as any,)
},)
