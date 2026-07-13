#!/usr/bin/env node

import path from 'node:path'
import { pathToFileURL } from 'node:url'

import { McpStdioClient } from './lib/mcp-stdio.mjs'

export function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize)
  if (value === null || typeof value !== 'object') return value

  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, canonicalize(value[key])]),
  )
}

export function canonicalContract(initialize, tools) {
  validateInitialize(initialize)
  validateTools(tools)
  return canonicalize({
    initialize,
    tools: [...tools].sort((left, right) =>
      left.name.localeCompare(right.name)
    ),
  })
}

export function validateInitialize(initialize) {
  if (
    !initialize || typeof initialize !== 'object' || Array.isArray(initialize)
  ) {
    throw new Error('initialize did not return an object')
  }
}

export function validateToolList(toolList) {
  if (!toolList || typeof toolList !== 'object' || Array.isArray(toolList)) {
    throw new Error('tools/list did not return an object')
  }
  if (
    Object.hasOwn(toolList, 'nextCursor')
    && toolList.nextCursor !== null
    && toolList.nextCursor !== ''
  ) {
    throw new Error(
      'tools/list pagination is unsupported; contract capture is incomplete',
    )
  }
  if (!Array.isArray(toolList.tools)) {
    throw new Error('tools/list did not return a tools array')
  }
  validateTools(toolList.tools)
}

function validateTools(tools) {
  if (!Array.isArray(tools)) throw new Error('tools must be an array')
  for (const [index, tool] of tools.entries()) {
    if (!tool || typeof tool !== 'object' || Array.isArray(tool)) {
      throw new Error(`tool at index ${index} is not an object`)
    }
    if (typeof tool.name !== 'string') {
      throw new Error(`tool at index ${index} is missing a string name`)
    }
    if (typeof tool.description !== 'string') {
      throw new Error(`tool ${tool.name} is missing a string description`)
    }
    if (
      !tool.inputSchema
      || typeof tool.inputSchema !== 'object'
      || Array.isArray(tool.inputSchema)
    ) {
      throw new Error(`tool ${tool.name} is missing an object inputSchema`)
    }
  }
}

function ensureTransportResult(result, method) {
  if (result?.transportError) {
    throw new Error(
      `${method} returned transport error: ${
        JSON.stringify(result.transportError)
      }`,
    )
  }
  if (result?.timeout) {
    throw new Error(`${method} timed out: ${result.stderr ?? ''}`)
  }
  if (result?.childExit !== undefined) {
    throw new Error(
      `${method} failed because the server exited: ${result.childExit}`,
    )
  }
  return result
}

export async function captureContract({ bin, timeoutMs = 20_000 } = {}) {
  if (!bin) throw new Error('--bin is required')
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new Error('--timeout-ms must be a positive number')
  }

  const env = { ...process.env }
  delete env.RUST_LOG
  const client = new McpStdioClient({
    bin,
    args: ['mcp'],
    cwd: process.cwd(),
    env,
    timeoutMs,
  })

  try {
    const initialize = ensureTransportResult(
      await client.request('initialize', {
        protocolVersion: '2025-03-26',
        capabilities: {},
        clientInfo: {
          name: 'reklawdbox-mcp-contract-capture',
          version: '0.1.0',
        },
      }),
      'initialize',
    )
    validateInitialize(initialize)
    client.notify('notifications/initialized')

    const toolList = ensureTransportResult(
      await client.listTools(),
      'tools/list',
    )
    validateToolList(toolList)
    if (client.protocolViolations.length > 0) {
      throw new Error(
        `server wrote non-JSON data to stdout: ${
          client.protocolViolations[0].slice(0, 160)
        }`,
      )
    }
    return canonicalContract(initialize, toolList.tools)
  } finally {
    await client.close()
  }
}

function parseArgs(argv) {
  let bin = './target/release/reklawdbox'
  let timeoutMs = 20_000
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === '--bin') {
      bin = argv[++index]
      if (!bin) throw new Error('--bin requires a value')
    } else if (argument === '--timeout-ms') {
      timeoutMs = Number(argv[++index])
    } else if (argument === '--help' || argument === '-h') {
      return { help: true }
    } else {
      throw new Error(`unknown argument: ${argument}`)
    }
  }
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new Error('--timeout-ms must be a positive number')
  }
  return { bin, timeoutMs }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    process.stdout.write(
      'Usage: node scripts/capture-mcp-contract.mjs [--bin PATH] [--timeout-ms MS]\n',
    )
    return
  }
  const contract = await captureContract(options)
  process.stdout.write(`${JSON.stringify(contract, null, 2)}\n`)
}

const invokedUrl = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href
  : null
if (invokedUrl === import.meta.url) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
