#!/usr/bin/env node

import { McpStdioClient } from './lib/mcp-stdio.mjs'

const args = process.argv.slice(2)
let bin = './target/release/reklawdbox'
let playlist = 'genre_verified'
let skipDb = false
let timeoutMs = 20_000

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i]
  if (arg === '--bin') {
    bin = args[++i]
  } else if (arg === '--playlist') {
    playlist = args[++i]
  } else if (arg === '--skip-db') {
    skipDb = true
  } else if (arg === '--timeout-ms') {
    timeoutMs = Number(args[++i])
  } else if (arg === '--help' || arg === '-h') {
    printUsage()
    process.exit(0)
  } else {
    fail(`unknown argument: ${arg}`)
  }
}

if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
  fail('--timeout-ms must be a positive number')
}

const env = { ...process.env }
delete env.RUST_LOG

const client = new McpStdioClient({
  bin,
  cwd: process.cwd(),
  env,
  timeoutMs,
})
const protocolViolations = client.protocolViolations

try {
  const summary = await runSmoke()
  console.log(JSON.stringify(summary, null, 2))
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exitCode = 1
} finally {
  await client.close()
}

async function runSmoke() {
  const initialized = await request('initialize', {
    protocolVersion: '2025-03-26',
    capabilities: {},
    clientInfo: {
      name: 'reklawdbox-mcp-smoke',
      version: '0.1.0',
    },
  })
  ensureNoTransportError(initialized, 'initialize')
  notify('notifications/initialized')

  const toolList = await client.listTools()
  ensureNoTransportError(toolList, 'tools/list')
  const tools = (toolList.tools ?? []).map((tool) => tool.name).sort()
  for (const name of ['help', 'search_tracks', 'calibration_coverage']) {
    if (!tools.includes(name)) {
      throw new Error(`tools/list did not include ${name}`)
    }
  }

  const help = await callTool('help', { topic: 'genre' })
  const helpText = toolText(help)
  if (!helpText.includes('calibration_coverage')) {
    throw new Error("help(topic='genre') did not mention calibration_coverage")
  }

  const auditHelp = await callTool('help', { topic: 'audit' })
  const auditHelpText = toolText(auditHelp)
  if (!auditHelpText.includes('Reload Tag')) {
    throw new Error("help(topic='audit') did not mention Reload Tag")
  }

  const albumHelp = await callTool('help', { topic: 'album' })
  const albumHelpPayload = parseToolJson(albumHelp, "help(topic='album')")
  if (albumHelpPayload.workflow !== 'Metadata Backfill') {
    throw new Error("help(topic='album') did not return Metadata Backfill")
  }
  if (!albumHelpPayload.sop?.includes('Step 1c')) {
    throw new Error(
      "help(topic='album') did not include the Step 1c label checkpoint",
    )
  }

  const taxonomy = await callTool('get_genre_taxonomy', {})
  const taxonomyPayload = parseToolJson(taxonomy, 'get_genre_taxonomy')
  if (
    !Array.isArray(taxonomyPayload.genres)
    || !taxonomyPayload.genres.includes('Italodance')
  ) {
    throw new Error(
      'get_genre_taxonomy did not return the canonical genre list',
    )
  }

  const playlistImportHelp = []
  for (const topic of ['set', 'pool', 'chapter']) {
    const result = await callTool('help', { topic })
    const payload = parseToolJson(result, `help(topic='${topic}')`)
    const sop = payload.sop ?? ''
    const normalized = sop.toLowerCase()

    for (
      const required of [
        'rekordbox xml',
        'playlists',
        'drag',
        'track count',
        'track order',
      ]
    ) {
      if (!normalized.includes(required)) {
        throw new Error(
          `help(topic='${topic}') did not include playlist import guidance containing '${required}'`,
        )
      }
    }
    if (sop.includes('import XmlPlaylistImportSteps')) {
      throw new Error(
        `help(topic='${topic}') exposed the playlist import component import`,
      )
    }
    if (sop.includes('<XmlPlaylistImportSteps />')) {
      throw new Error(
        `help(topic='${topic}') exposed an unresolved playlist import component tag`,
      )
    }

    playlistImportHelp.push({
      topic,
      bytes: Buffer.byteLength(sop),
    })
  }

  const summary = {
    binary: bin,
    server: initialized.serverInfo,
    protocolVersion: initialized.protocolVersion,
    toolCount: tools.length,
    help: {
      topic: 'genre',
      bytes: Buffer.byteLength(helpText),
    },
    auditHelp: {
      topic: 'audit',
      bytes: Buffer.byteLength(auditHelpText),
    },
    albumHelp: {
      topic: 'album',
      bytes: Buffer.byteLength(albumHelpPayload.sop ?? ''),
    },
    taxonomy: {
      genreCount: taxonomyPayload.genres.length,
    },
    playlistImportHelp,
    protocolViolations,
  }

  if (!skipDb) {
    const coverage = await callTool('calibration_coverage', { playlist })
    const coverageJson = parseToolJson(coverage, 'calibration_coverage')

    const missing = await request('tools/call', {
      name: 'calibration_coverage',
      arguments: { playlist: '__reklawdbox_smoke_missing_playlist__' },
    })

    summary.calibrationCoverage = {
      playlist: coverageJson.playlist,
      totalTracks: coverageJson.total_tracks,
      tracksWithAudioFeatures: coverageJson.tracks_with_audio_features,
      tracksWithStratumFeatures: coverageJson.tracks_with_stratum_features,
      tracksWithEssentiaFeatures: coverageJson.tracks_with_essentia_features,
      genreCount: Array.isArray(coverageJson.genres)
        ? coverageJson.genres.length
        : undefined,
      minTracksPerGenre: coverageJson.min_tracks_per_genre,
      storedProfilesNotInPlaylist: coverageJson.stored_profiles_not_in_playlist
        ?? [],
    }
    summary.missingPlaylistError = missing.transportError?.message ?? null

    if (!summary.missingPlaylistError?.includes('not found')) {
      throw new Error('missing-playlist call did not return the expected error')
    }
  }

  if (protocolViolations.length > 0) {
    throw new Error(
      `server wrote non-JSON data to stdout: ${
        protocolViolations[0].slice(0, 160)
      }`,
    )
  }

  return summary
}

async function callTool(name, toolArgs) {
  const result = await request('tools/call', {
    name,
    arguments: toolArgs,
  })
  ensureNoTransportError(result, `tools/call ${name}`)
  if (result.isError) {
    throw new Error(`${name} returned tool error: ${toolText(result)}`)
  }
  return result
}

function request(method, params) {
  return client.request(method, params)
}

function notify(method, params) {
  client.notify(method, params)
}

function toolText(result) {
  return result?.content?.[0]?.text ?? ''
}

function parseToolJson(result, name) {
  try {
    return JSON.parse(toolText(result))
  } catch (error) {
    throw new Error(`${name} returned non-JSON text: ${error.message}`)
  }
}

function ensureNoTransportError(result, label) {
  if (result?.transportError) {
    throw new Error(`${label} failed: ${JSON.stringify(result.transportError)}`)
  }
  if (result?.timeout) {
    throw new Error(`${label} timed out. stderr tail:\n${result.stderr}`)
  }
  if (result?.childExit !== undefined) {
    throw new Error(
      `${label} failed because server exited: ${result.childExit}`,
    )
  }
}

function printUsage() {
  console.log(`Usage: node scripts/mcp-smoke.mjs [options]

Options:
  --bin <path>         MCP server binary (default: ./target/release/reklawdbox)
  --playlist <name>    verified playlist for calibration_coverage (default: genre_verified)
  --skip-db            only run handshake, tools/list, and help()
  --timeout-ms <ms>    JSON-RPC request timeout (default: 20000)
`)
}

function fail(message) {
  console.error(message)
  printUsage()
  process.exit(2)
}
