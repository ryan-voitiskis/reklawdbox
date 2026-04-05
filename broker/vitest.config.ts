import {
  cloudflareTest,
  readD1Migrations,
} from '@cloudflare/vitest-pool-workers'
import { defineConfig } from 'vitest/config'

const migrationsPath = new URL('./migrations', import.meta.url).pathname
const migrations = await readD1Migrations(migrationsPath)

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: {
        configPath: './wrangler.toml',
      },
      miniflare: {
        d1Databases: {
          DB: 'broker-test-db',
        },
        bindings: {
          TEST_MIGRATIONS: migrations,
          DISCOGS_CONSUMER_KEY: 'test-consumer-key',
          DISCOGS_CONSUMER_SECRET: 'test-consumer-secret',
          BROKER_STATE_ENCRYPTION_KEY: 'test-state-encryption-key',
          BROKER_CLIENT_TOKEN: 'test-broker-token',
          BROKER_PUBLIC_BASE_URL: 'https://broker.test',
        },
      },
    }),
  ],
  test: {
    include: ['tests/**/*.test.ts'],
    setupFiles: ['./tests/setup.ts'],
  },
})
