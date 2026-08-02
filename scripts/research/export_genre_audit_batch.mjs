#!/usr/bin/env node

import { readFile, writeFile } from 'node:fs/promises'
import { McpStdioClient } from '../lib/mcp-stdio.mjs'

const PLAYLIST_NAME = 'genre_audit_blind_v1'

function requiredArg(name) {
  const index = process.argv.indexOf(name)
  if (index === -1 || index + 1 >= process.argv.length) {
    throw new Error(`missing required argument ${name}`)
  }
  return process.argv[index + 1]
}

function text(result) {
  return result?.content?.[0]?.text ?? ''
}

function parse(result, label) {
  if (
    result?.transportError || result?.timeout || result?.childExit !== undefined
  ) {
    throw new Error(`${label}: ${JSON.stringify(result)}`)
  }
  if (result?.isError) throw new Error(`${label}: ${text(result)}`)
  try {
    return JSON.parse(text(result))
  } catch {
    return { text: text(result) }
  }
}

const mappingPath = requiredArg('--mapping')
const xmlPath = requiredArg('--xml')
const reviewPath = requiredArg('--review')
const bin = requiredArg('--bin')
const mapping = JSON.parse(await readFile(mappingPath, 'utf8'))
const selected = mapping.selected ?? []
if (mapping.experiment_id !== 'genre-audit-consensus-v1') {
  throw new Error('unexpected genre-audit experiment ID')
}
if (
  selected.length !== 6
  || new Set(selected.map((row) => row.track_id)).size !== 6
) {
  throw new Error('genre-audit roster must contain six unique tracks')
}
if (
  mapping.cohort_counts?.ranked !== 4 || mapping.cohort_counts?.control !== 2
) {
  throw new Error(
    'genre-audit roster must contain four ranked rows and two controls',
  )
}

const client = new McpStdioClient({ bin, timeoutMs: 60_000 })

async function call(name, args = {}) {
  return parse(await client.callTool(name, args), name)
}

try {
  await client.request('initialize', {
    protocolVersion: '2025-03-26',
    capabilities: {},
    clientInfo: { name: 'genre-audit-batch-export', version: '1' },
  })
  client.notify('notifications/initialized')

  const before = await call('preview_changes', { format: 'summary' })
  if (before.text !== 'No changes staged.') {
    throw new Error(
      'refusing export because the MCP process has staged changes',
    )
  }

  for (const row of selected) {
    const live = await call('get_track', { track_id: row.track_id })
    if (
      live.id !== row.track_id
      || live.file_path !== row.file_path
      || live.artist !== row.artist
      || live.title !== row.title
    ) {
      throw new Error(`live identity drift for blind code ${row.code}`)
    }
  }

  const xmlResult = await call('write_xml', {
    output_path: xmlPath,
    playlists: [{
      name: PLAYLIST_NAME,
      track_ids: selected.map((row) => row.track_id),
    }],
  })
  const after = await call('preview_changes', { format: 'summary' })
  if (after.text !== 'No changes staged.') {
    throw new Error('playlist export unexpectedly left staged changes')
  }

  const header = [
    'position',
    'code',
    'artist',
    'title',
    'verdict',
    'confidence',
    'alternatives',
    'notes',
  ].join('\t')
  const rows = selected.map((row) =>
    [
      row.position,
      row.code,
      row.artist.replaceAll('\t', ' '),
      row.title.replaceAll('\t', ' '),
      '',
      '',
      '',
      '',
    ].join('\t')
  )
  await writeFile(reviewPath, `${[header, ...rows].join('\n')}\n`)

  mapping.export = {
    playlist_name: PLAYLIST_NAME,
    xml_path: xmlPath,
    review_path: reviewPath,
    exported_at: new Date().toISOString(),
    zero_staged_changes_before_and_after: true,
    live_identity_matches: selected.length,
    xml_result: xmlResult,
  }
  await writeFile(mappingPath, `${JSON.stringify(mapping, null, 2)}\n`)

  console.log(JSON.stringify(
    {
      experiment_id: mapping.experiment_id,
      playlist_name: PLAYLIST_NAME,
      tracks: selected.length,
      cohort_counts: mapping.cohort_counts,
      roster_sha256: mapping.roster_sha256,
      xml_path: xmlPath,
      review_path: reviewPath,
      zero_staged_changes_before_and_after: true,
      live_identity_matches: selected.length,
    },
    null,
    2,
  ))
} finally {
  await client.close()
}
