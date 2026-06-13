#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'

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

const child = spawn(bin, ['mcp'], {
  cwd: process.cwd(),
  env,
  stdio: ['pipe', 'pipe', 'pipe'],
})

const pending = new Map()
const protocolViolations = []
let nextId = 1
let stderr = ''
let exited = false

const rl = createInterface({ input: child.stdout })
rl.on('line', (line) => {
  if (!line.trim()) return

  let message
  try {
    message = JSON.parse(line)
  } catch {
    protocolViolations.push(line)
    return
  }

  if (message.id != null && pending.has(message.id)) {
    const { resolve } = pending.get(message.id)
    pending.delete(message.id)
    resolve(message.error ? { transportError: message.error } : message.result)
  }
})

child.stderr.on('data', (chunk) => {
  stderr += chunk.toString()
})

child.on('exit', (code, signal) => {
  exited = true
  for (const { resolve } of pending.values()) {
    resolve({ childExit: code ?? signal })
  }
  pending.clear()
})

try {
  const summary = await runSmoke()
  console.log(JSON.stringify(summary, null, 2))
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exitCode = 1
} finally {
  child.stdin.end()
  setTimeout(() => {
    if (!exited) child.kill('SIGTERM')
  }, 200)
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

  const toolList = await request('tools/list')
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

  const summary = {
    binary: bin,
    server: initialized.serverInfo,
    protocolVersion: initialized.protocolVersion,
    toolCount: tools.length,
    help: {
      topic: 'genre',
      bytes: Buffer.byteLength(helpText),
    },
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
  const id = nextId
  nextId += 1
  const message = { jsonrpc: '2.0', id, method }
  if (params !== undefined) message.params = params
  child.stdin.write(`${JSON.stringify(message)}\n`)

  return new Promise((resolve) => {
    pending.set(id, { resolve })
    setTimeout(() => {
      if (!pending.delete(id)) return
      resolve({
        timeout: method,
        stderr: stderr.slice(-2_000),
      })
    }, timeoutMs)
  })
}

function notify(method, params) {
  const message = { jsonrpc: '2.0', method }
  if (params !== undefined) message.params = params
  child.stdin.write(`${JSON.stringify(message)}\n`)
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
