#!/usr/bin/env node

import { readFile, writeFile } from 'node:fs/promises'
import { McpStdioClient } from '../lib/mcp-stdio.mjs'

const PLAYLIST_NAME = 'genre_review_assistant_v1'
const GENRE_ALIASES = new Map([
  ['Dub Reggae', 'Dub'],
  ['Reggae Dub', 'Dub'],
])

function canonicalGenre(value) {
  return GENRE_ALIASES.get(value) ?? value
}

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

function flat(value) {
  return String(value ?? '').replaceAll('\t', ' ').replaceAll('\n', ' ')
}

function markdown(value) {
  return String(value ?? '').replaceAll('\\', '\\\\').replaceAll('|', '\\|')
}

function displayHints(row) {
  return row.hints
    .map((hint) =>
      `${
        hint.role === 'current_genre_context' ? 'current' : 'alternative'
      }: ${hint.genre}`
    )
    .join('; ')
}

function displayReferences(row) {
  return row.hints
    .flatMap((hint) =>
      hint.references.map((reference) =>
        `${hint.genre}: ${reference.artist} – ${reference.title}`
      )
    )
    .join('; ')
}

function displayCues(row) {
  return row.listening_cues.map((cue) => cue.description).join('; ')
}

function reviewSheet(selected) {
  const header = [
    'position',
    'code',
    'artist',
    'title',
    'current_genre',
    'genre_hints',
    'verified_references',
    'listening_cues',
    'verdict',
    'confidence',
    'alternatives',
    'references_helpful',
    'vocabulary_helpful',
    'notes',
  ].join('\t')
  const rows = selected.map((row) =>
    [
      row.position,
      row.code,
      row.artist,
      row.title,
      row.current_genre,
      displayHints(row),
      displayReferences(row),
      displayCues(row),
      '',
      '',
      '',
      '',
      '',
      '',
    ].map(flat).join('\t')
  )
  return `${[header, ...rows].join('\n')}\n`
}

function reviewGuide(selected) {
  const lines = [
    '# Genre Review Assistant V1',
    '',
    'This is a decision aid, not an automated classification. Similarity means proximity in one frozen representation; it is not probability or truth.',
    '',
    'For each track, decide whether the current genre is verified, should be replaced with a canonical genre, is ambiguous, or should be skipped. The vocabulary describes measured cache dimensions only.',
    '',
  ]
  for (const row of selected) {
    lines.push(
      `## ${row.code}: ${markdown(row.artist)} – ${markdown(row.title)}`,
    )
    lines.push('')
    lines.push(`- Current genre: **${markdown(row.current_genre)}**`)
    lines.push(`- Measured tempo: ${Number(row.bpm).toFixed(2)} BPM`)
    lines.push('- Listening hints:')
    for (const hint of row.hints) {
      const role = hint.role === 'current_genre_context'
        ? 'current-genre context'
        : 'alternative hint'
      lines.push(`  - ${markdown(hint.genre)} (${role})`)
      for (const reference of hint.references) {
        lines.push(
          `    - [${markdown(reference.artist)} – ${
            markdown(reference.title)
          }](<${reference.file_path}>)`,
        )
      }
    }
    lines.push('- Neutral listening cues:')
    for (const cue of row.listening_cues) {
      lines.push(`  - ${markdown(cue.description)}`)
    }
    lines.push('')
    lines.push(
      'Review: `verified`, a replacement genre, `ambiguous`, or `skip`.',
    )
    lines.push('')
  }
  return `${lines.join('\n')}\n`
}

const mappingPath = requiredArg('--mapping')
const xmlPath = requiredArg('--xml')
const reviewPath = requiredArg('--review')
const guidePath = requiredArg('--guide')
const bin = requiredArg('--bin')
const mapping = JSON.parse(await readFile(mappingPath, 'utf8'))
const selected = mapping.selected ?? []
if (mapping.experiment_id !== 'genre-review-assistant-v1') {
  throw new Error('unexpected genre-review-assistant experiment ID')
}
if (
  selected.length !== 6
  || new Set(selected.map((row) => row.track_id)).size !== 6
  || new Set(selected.map((row) => row.current_genre)).size !== 6
) {
  throw new Error(
    'genre-review roster must contain six unique tracks and genres',
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
    clientInfo: { name: 'genre-review-batch-export', version: '1' },
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
      || canonicalGenre(live.genre) !== row.current_genre
    ) {
      throw new Error(
        `live identity or genre drift for review code ${row.code}`,
      )
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

  await writeFile(reviewPath, reviewSheet(selected))
  await writeFile(guidePath, reviewGuide(selected))

  mapping.export = {
    playlist_name: PLAYLIST_NAME,
    xml_path: xmlPath,
    review_path: reviewPath,
    guide_path: guidePath,
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
      roster_sha256: mapping.roster_sha256,
      xml_path: xmlPath,
      review_path: reviewPath,
      guide_path: guidePath,
      zero_staged_changes_before_and_after: true,
      live_identity_matches: selected.length,
    },
    null,
    2,
  ))
} finally {
  await client.close()
}
