import {
  defineWorkersConfig,
  readD1Migrations,
} from '@cloudflare/vitest-pool-workers/config'
import { resolve } from 'node:path'

export default defineWorkersConfig(async () => {
  const migrations = await readD1Migrations(
    resolve(__dirname, 'migrations'),
  )

  return {
    test: {
      pool: '@cloudflare/vitest-pool-workers',
      include: ['tests/**/*.test.ts'],
      setupFiles: ['./tests/setup.ts'],
      poolOptions: {
        workers: {
          isolatedStorage: true,
          singleWorker: true,
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
              BROKER_CLIENT_TOKEN: 'test-broker-token',
              BROKER_PUBLIC_BASE_URL: 'https://broker.test',
            },
          },
        },
      },
    },
  }
})
