#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { chmod, mkdir, readFile, writeFile } from 'node:fs/promises'
import { basename, join } from 'node:path'
import { McpStdioClient } from '../lib/mcp-stdio.mjs'
import {
  blindReviewXml,
  reviewGuide,
  reviewSheet,
  validateReviewRoster,
} from './genre_truth_review_material.mjs'

const EXPECTED_MANIFEST_SHA256 =
  '7f8c84c706f50638533b62e5478cdab0ccd88caf3fd96cdc6c9391afe37e5993'
const EXPERIMENT_ID_PATTERN = /^genre-intelligence-precision-first-v1-h\d{2}$/
const SELECTED_KEYS = new Set([
  'album',
  'artist',
  'artist_group',
  'code',
  'file_path',
  'position',
  'release_group',
  'source_row_id_private',
  'title',
  'track_id',
])

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

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

function validateMapping(mapping, batch) {
  if (!EXPERIMENT_ID_PATTERN.test(mapping.experiment_id)) {
    throw new Error(`unexpected experiment ID in ${batch}`)
  }
  const expectedSuffix = `h${batch.slice(1)}`
  if (!mapping.experiment_id.endsWith(expectedSuffix)) {
    throw new Error(`experiment and batch labels differ in ${batch}`)
  }
  const selected = mapping.selected ?? []
  validateReviewRoster(selected, 1)
  if (
    selected.length > 6
    || !mapping.selection_rule?.model_and_sampling_fields_absent
  ) {
    throw new Error(`blind-review contract differs in ${batch}`)
  }
  for (const row of selected) {
    const unexpected = Object.keys(row).filter((key) => !SELECTED_KEYS.has(key))
    if (unexpected.length > 0) {
      throw new Error(`unexpected review fields in ${batch}: ${unexpected}`)
    }
  }
  return selected
}

const manifestPath = requiredArg('--manifest')
const outputDir = requiredArg('--output-dir')
const xmlPath = requiredArg('--xml')
const receiptPath = requiredArg('--receipt')
const bin = requiredArg('--bin')
const manifestBytes = await readFile(manifestPath)
if (sha256(manifestBytes) !== EXPECTED_MANIFEST_SHA256) {
  throw new Error('review manifest SHA-256 differs')
}
const manifest = JSON.parse(manifestBytes)
if (
  manifest.experiment_id !== 'genre-intelligence-precision-first-review-v1'
  || manifest.offers !== 35
  || manifest.batch_size_cap !== 6
  || manifest.batches?.length !== 6
  || !manifest.model_and_sampling_fields_absent_from_mappings
) {
  throw new Error('review manifest contract differs')
}

const batches = []
for (const record of manifest.batches) {
  const mappingBytes = await readFile(record.mapping_path)
  if (sha256(mappingBytes) !== record.mapping_sha256) {
    throw new Error(`review mapping SHA-256 differs for ${record.batch}`)
  }
  const mapping = JSON.parse(mappingBytes)
  const selected = validateMapping(mapping, record.batch)
  if (selected.length !== record.rows) {
    throw new Error(`review row count differs for ${record.batch}`)
  }
  batches.push({ record, mapping, selected })
}
if (batches.reduce((count, batch) => count + batch.selected.length, 0) !== 35) {
  throw new Error('review mappings do not cover all 35 offers')
}

await mkdir(outputDir, { recursive: true })
const client = new McpStdioClient({ bin, timeoutMs: 60_000 })

async function call(name, args = {}) {
  return parse(await client.callTool(name, args), name)
}

try {
  await client.request('initialize', {
    protocolVersion: '2025-03-26',
    capabilities: {},
    clientInfo: {
      name: 'genre-intelligence-precision-review-export',
      version: '1',
    },
  })
  client.notify('notifications/initialized')

  const before = await call('preview_changes', { format: 'summary' })
  if (before.text !== 'No changes staged.') {
    throw new Error(
      'refusing export because the MCP process has staged changes',
    )
  }

  let identityMatches = 0
  for (const batch of batches) {
    for (const row of batch.selected) {
      const live = await call('get_track', { track_id: row.track_id })
      if (
        live.id !== row.track_id
        || live.file_path !== row.file_path
        || live.artist !== row.artist
        || live.title !== row.title
      ) {
        throw new Error(`live identity drift for blind code ${row.code}`)
      }
      identityMatches += 1
    }
  }

  const xmlResult = await call('write_xml', {
    output_path: xmlPath,
    playlists: batches.map((batch) => ({
      name: batch.mapping.export_playlist_name,
      track_ids: batch.selected.map((row) => row.track_id),
    })),
  })
  const after = await call('preview_changes', { format: 'summary' })
  if (after.text !== 'No changes staged.') {
    throw new Error('playlist export unexpectedly left staged changes')
  }
  await writeFile(xmlPath, blindReviewXml(await readFile(xmlPath, 'utf8')))

  const material = []
  for (const batch of batches) {
    const label = batch.record.batch
    const stem = `genre_intelligence_precision_first_${label.toLowerCase()}`
    const reviewPath = join(outputDir, `${stem}.tsv`)
    const guidePath = join(outputDir, `${stem}.md`)
    await writeFile(reviewPath, reviewSheet(batch.selected))
    await writeFile(guidePath, reviewGuide(batch.selected, label))
    await Promise.all([chmod(reviewPath, 0o600), chmod(guidePath, 0o600)])
    material.push({
      batch: label,
      rows: batch.selected.length,
      playlist_name: batch.mapping.export_playlist_name,
      review_path: reviewPath,
      guide_path: guidePath,
      mapping_sha256: batch.record.mapping_sha256,
    })
  }
  await chmod(xmlPath, 0o600)

  const receipt = {
    schema_version: 1,
    experiment_id: manifest.experiment_id,
    manifest_sha256: EXPECTED_MANIFEST_SHA256,
    exported_at: new Date().toISOString(),
    xml_path: xmlPath,
    xml_file: basename(xmlPath),
    xml_sha256: sha256(await readFile(xmlPath)),
    playlists: material,
    tracks: identityMatches,
    live_identity_matches: identityMatches,
    zero_staged_changes_before_and_after: true,
    hidden_fields_written_to_review_material: false,
    source_mappings_unchanged: true,
    xml_result: xmlResult,
  }
  await writeFile(receiptPath, `${JSON.stringify(receipt, null, 2)}\n`)
  await chmod(receiptPath, 0o600)
  console.log(JSON.stringify(
    {
      experiment_id: receipt.experiment_id,
      xml_path: receipt.xml_path,
      xml_sha256: receipt.xml_sha256,
      playlists: material.map((item) => ({
        batch: item.batch,
        rows: item.rows,
        playlist_name: item.playlist_name,
      })),
      tracks: identityMatches,
      zero_staged_changes_before_and_after: true,
      hidden_fields_written_to_review_material: false,
      source_mappings_unchanged: true,
    },
    null,
    2,
  ))
} finally {
  await client.close()
}
