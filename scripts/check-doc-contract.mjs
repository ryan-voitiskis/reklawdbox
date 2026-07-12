#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

import { McpStdioClient } from './lib/mcp-stdio.mjs'

const GENERATED_APPLICATION_CLI_OPTIONS = new Set(['-h', '--help'])
const GENERATED_ROOT_CLI_OPTIONS = new Set([
  '-h',
  '--help',
  '-V',
  '--version',
])
const SITE_ORIGIN = 'https://reklawdbox.com'
const PARTIAL_ALTERNATIVE = Symbol('partial-alternative-schema')
const MCP_OUTPUT_CONTRACT_TOOLS = new Set([
  'analyze_audio_batch',
  'backfill_labels',
  'enrich_tracks',
  'scan_duplicates',
])
const REQUIRED_XML_BACKUP_SUCCESS_CONDITION =
  'XML export proceeds only after the built-in backup succeeds or the configured custom script exits zero'

export function compareToolMappings(liveTools, references) {
  const liveNames = [...new Set(liveTools.map((tool) => tool.name))].sort()
  const mappedNames = [...new Set(references.map((tool) => tool.name))].sort()
  const missing = liveNames.filter((name) => !mappedNames.includes(name))
  const extra = mappedNames.filter((name) => !liveNames.includes(name))
  if (missing.length || extra.length) {
    throw new Error(
      `site/src/data/tool-reference.mjs:1: tool mapping mismatch; missing: ${
        missing.join(', ') || 'none'
      }; extra: ${extra.join(', ') || 'none'}`,
    )
  }
}

export function parseContractMarkers(documents) {
  const markers = []
  for (const document of documents) {
    const source = document.content
    const startPattern =
      /(?:<!--\s*|\{\/\*\s*)doc-contract:(mcp-output|mcp-surface|mcp|cli)\s+([^\n]*?)(?:\s*-->|\s*\*\/\})/g
    let match
    while ((match = startPattern.exec(source)) !== null) {
      const [opening, kind, rawAttributes] = match
      const attributes = parseAttributes(rawAttributes)
      const line = lineNumberAt(source, match.index)
      const marker = {
        kind,
        attributes,
        file: document.file,
        line,
        body: '',
      }

      if (attributes.surface === 'none') {
        markers.push(marker)
        continue
      }

      const closings = [
        `<!-- /doc-contract:${kind} -->`,
        `{/* /doc-contract:${kind} */}`,
      ]
      const bodyStart = match.index + opening.length
      const foundClosings = closings
        .map((closing) => ({
          closing,
          index: source.indexOf(closing, bodyStart),
        }))
        .filter(({ index }) => index >= 0)
        .sort((left, right) => left.index - right.index)
      const bodyEnd = foundClosings[0]?.index ?? -1
      if (bodyEnd < 0) {
        throw markerError(
          marker,
          `missing closing marker for doc-contract:${kind}`,
        )
      }
      const closing = foundClosings[0].closing
      marker.body = source.slice(bodyStart, bodyEnd)
      markers.push(marker)
      startPattern.lastIndex = bodyEnd + closing.length
    }
  }
  return markers
}

function parseAttributes(source) {
  const attributes = {}
  const pattern = /([a-zA-Z][a-zA-Z0-9_-]*)=([^\s]+)/g
  let match
  while ((match = pattern.exec(source)) !== null) {
    attributes[match[1]] = match[2]
  }
  return attributes
}

function lineNumberAt(source, index) {
  return source.slice(0, index).split('\n').length
}

function markerError(marker, message) {
  return new Error(`${marker.file}:${marker.line}: ${message}`)
}

export function parseMarkdownTable(
  source,
  marker = { file: '<fixture>', line: 1 },
) {
  const lines = source.split('\n')
  const headerIndexes = []
  for (let index = 0; index + 1 < lines.length; index += 1) {
    if (!lines[index].trim().startsWith('|')) continue
    const separator = splitTableRow(lines[index + 1])
    if (
      separator.length
      && separator.every((cell) => /^:?-{3,}:?$/.test(cell.trim()))
    ) {
      headerIndexes.push(index)
    }
  }
  if (headerIndexes.length === 0) {
    throw markerError(marker, 'marked surface has no Markdown table')
  }
  if (headerIndexes.length !== 1) {
    throw markerError(
      marker,
      `marked surface must contain exactly one Markdown table; found ${headerIndexes.length}`,
    )
  }
  const [headerIndex] = headerIndexes

  const headers = splitTableRow(lines[headerIndex]).map((cell) =>
    stripMarkdown(cell).toLowerCase()
  )
  const rows = []
  for (let index = headerIndex + 2; index < lines.length; index += 1) {
    if (!lines[index].trim().startsWith('|')) break
    const cells = splitTableRow(lines[index])
    if (cells.length !== headers.length) {
      throw markerError(marker, 'marked table row has the wrong column count')
    }
    rows.push({
      columns: Object.fromEntries(
        headers.map((header, cell) => [header, cells[cell].trim()]),
      ),
      raw: lines[index],
    })
  }
  return { headers, rows }
}

function splitTableRow(line) {
  const trimmed = line.trim().replace(/^\|/, '').replace(/\|$/, '')
  const cells = []
  let cell = ''
  for (let index = 0; index < trimmed.length; index += 1) {
    const char = trimmed[index]
    if (char === '\\' && trimmed[index + 1] === '|') {
      cell += '|'
      index += 1
    } else if (char === '|') {
      cells.push(cell)
      cell = ''
    } else {
      cell += char
    }
  }
  cells.push(cell)
  return cells.map((value) => value.trim())
}

function stripMarkdown(value) {
  return value
    .replace(/`/g, '')
    .replace(/\*\*/g, '')
    .replace(/<[^>]+>/g, '')
    .trim()
}

function markerIncludes(marker) {
  return (marker.attributes.include ?? '')
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean)
}

function markerSchemaPath(marker) {
  return marker.attributes.schema
}

function markerRequiredness(marker) {
  const mode = marker.attributes.requiredness
  if (!['global', 'conditional'].includes(mode)) {
    throw markerError(marker, `unknown requiredness mode: ${mode}`)
  }
  return mode
}

function parseMcpRows(marker) {
  if (marker.attributes.surface === 'none') return []
  const table = parseMarkdownTable(marker.body, marker)
  const nameHeader = table.headers.includes('parameter')
    ? 'parameter'
    : table.headers.includes('field')
    ? 'field'
    : null
  if (!nameHeader || !table.headers.includes('type')) {
    throw markerError(
      marker,
      'MCP table needs Parameter/Field and Type columns',
    )
  }
  return table.rows.map((row) => {
    const name = stripMarkdown(row.columns[nameHeader])
    const enumValues = parseDocumentedJsonArray(
      row.columns.values,
      marker,
      `${name} Values`,
    )
    const itemEnumValues = parseDocumentedJsonArray(
      row.columns['item values'],
      marker,
      `${name} Item values`,
    )
    const documentedTypes = normalizeDocumentedTypeSurface(
      row.columns.type,
      marker,
      name,
    )
    return {
      name,
      types: documentedTypes.types,
      itemTypes: documentedTypes.itemTypes,
      required: /\byes\b|\brequired\b/i.test(row.columns.required ?? ''),
      defaultText: (row.columns.default ?? '').trim(),
      enumValues,
      itemEnumValues,
      raw: row.raw,
      marker,
      schemaPath: markerSchemaPath(marker),
      requiredness: markerRequiredness(marker),
    }
  })
}

function parseDocumentedJsonArray(value, marker, label) {
  const literal = parseDocumentedJsonLiteral(value, marker, label)
  if (!literal.present) return null
  if (!Array.isArray(literal.value)) {
    throw markerError(marker, `${label} must be a JSON array`)
  }
  return literal.value
}

function parseDocumentedJsonLiteral(value, marker, label) {
  let source = (value ?? '').trim()
  if (!source) return { present: false, value: undefined }
  if (source.startsWith('`') && source.endsWith('`')) {
    source = source.slice(1, -1)
  }
  try {
    return { present: true, value: JSON.parse(source) }
  } catch (error) {
    throw markerError(
      marker,
      `${label} must contain one valid JSON literal: ${error.message}`,
    )
  }
}

function normalizeDocumentedTypeSurface(value, marker, name) {
  const normalized = stripMarkdown(value)
    .toLowerCase()
    .replace(/\s+or\s+/g, '|')
  const parts = normalized.split('|').map((part) => part.trim()).filter(Boolean)
  if (!parts.length) {
    throw markerError(marker, `${name} has an empty documented type`)
  }
  const types = new Set()
  const itemTypes = new Set()
  const scalarTypes = new Map([
    ['string', 'string'],
    ['integer', 'integer'],
    ['number', 'number'],
    ['boolean', 'boolean'],
    ['flag', 'boolean'],
    ['object', 'object'],
    ['array', 'array'],
    ['list', 'array'],
    ['null', 'null'],
  ])
  for (const part of parts) {
    const array = part.match(/^(string|integer|number|boolean|object)\s*\[\]$/)
    if (array) {
      types.add('array')
      itemTypes.add(array[1])
      continue
    }
    const scalar = scalarTypes.get(part)
    if (!scalar) {
      throw markerError(
        marker,
        `${name} has unsupported documented type ${JSON.stringify(part)}`,
      )
    }
    types.add(scalar)
  }
  return {
    types: [...types].sort(),
    itemTypes: [...itemTypes].sort(),
  }
}

function resolveLocalRef(root, node, seen = new Set()) {
  if (!node || typeof node !== 'object' || Array.isArray(node) || !node.$ref) {
    return node
  }
  if (!node.$ref.startsWith('#/')) return node
  if (seen.has(node.$ref)) {
    throw new Error(`cyclic JSON Schema ref: ${node.$ref}`)
  }

  const nextSeen = new Set(seen).add(node.$ref)
  const target = node.$ref
    .slice(2)
    .split('/')
    .map(decodeJsonPointer)
    .reduce((value, key) => value?.[key], root)
  const resolvedTarget = resolveLocalRef(root, target, nextSeen)
  if (resolvedTarget === undefined) return undefined

  const siblings = Object.fromEntries(
    Object.entries(node).filter(([name]) => name !== '$ref'),
  )
  if (!Object.keys(siblings).length) return resolvedTarget

  // JSON Schema applies local $ref siblings conjunctively. Representing that
  // merge as allOf preserves both sides without one silently overwriting the
  // other (including sibling defaults and required properties).
  return { allOf: [resolvedTarget, siblings] }
}

function decodeJsonPointer(value) {
  return value.replace(/~1/g, '/').replace(/~0/g, '~')
}

function schemaAt(root, pointer) {
  if (pointer === '/' || pointer === '') return resolveLocalRef(root, root)
  let current = root
  for (
    const segment of pointer.replace(/^\//, '').split('/').map(
      decodeJsonPointer,
    )
  ) {
    current = resolveLocalRef(root, current)
    current = current?.[segment]
    if (current === undefined) return undefined
  }
  return resolveLocalRef(root, current)
}

function mergeConjunctiveSurfaces(left, right) {
  const properties = { ...left.properties }
  for (const [name, schema] of Object.entries(right.properties)) {
    properties[name] = Object.hasOwn(properties, name)
      ? { allOf: [properties[name], schema] }
      : schema
  }
  return {
    properties,
    required: new Set([...left.required, ...right.required]),
  }
}

function mergeAlternativeSurfaces(surfaces, keyword) {
  if (!surfaces.length) return { properties: {}, required: new Set() }
  const names = new Set(
    surfaces.flatMap((surface) => Object.keys(surface.properties)),
  )
  const requiredNames = new Set(
    surfaces.flatMap((surface) => [...surface.required]),
  )
  const properties = {}
  for (const name of names) {
    properties[name] = {
      [keyword]: surfaces.map((surface) =>
        surface.properties[name] ?? { [PARTIAL_ALTERNATIVE]: true }
      ),
    }
  }
  const required = new Set(
    [...requiredNames].filter((name) =>
      surfaces.every((surface) => surface.required.has(name))
    ),
  )
  return { properties, required }
}

function schemaObjectSurface(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (!resolved || typeof resolved !== 'object' || Array.isArray(resolved)) {
    return { properties: {}, required: new Set() }
  }
  if (seen.has(resolved)) {
    throw new Error('cyclic object schema composition')
  }
  const nextSeen = new Set(seen).add(resolved)
  let surface = {
    properties: { ...(resolved.properties ?? {}) },
    required: new Set(resolved.required ?? []),
  }

  for (const variant of resolved.allOf ?? []) {
    surface = mergeConjunctiveSurfaces(
      surface,
      schemaObjectSurface(root, variant, nextSeen),
    )
  }
  for (const keyword of ['anyOf', 'oneOf']) {
    const variants = resolved[keyword] ?? []
    if (!variants.length) continue
    surface = mergeConjunctiveSurfaces(
      surface,
      mergeAlternativeSurfaces(
        variants.map((variant) => schemaObjectSurface(root, variant, nextSeen)),
        keyword,
      ),
    )
  }
  return surface
}

function schemaProperties(root, pointer) {
  return schemaObjectSurface(root, schemaAt(root, pointer))
}

function normalizeTypeConstraint(values) {
  const normalized = new Set(values)
  if (normalized.has('number')) normalized.delete('integer')
  return normalized
}

function intersectTypeConstraints(left, right) {
  if (left === null) {
    return right === null ? null : normalizeTypeConstraint(right)
  }
  if (right === null) return normalizeTypeConstraint(left)
  const intersection = new Set()
  for (const leftType of left) {
    for (const rightType of right) {
      if (leftType === rightType) intersection.add(leftType)
      else if (
        (leftType === 'number' && rightType === 'integer')
        || (leftType === 'integer' && rightType === 'number')
      ) {
        intersection.add('integer')
      }
    }
  }
  return normalizeTypeConstraint(intersection)
}

function unionTypeConstraints(constraints) {
  if (constraints.some((constraint) => constraint === null)) return null
  return normalizeTypeConstraint(
    constraints.flatMap((constraint) => [...constraint]),
  )
}

function jsonSchemaType(value) {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  if (typeof value === 'number' && Number.isInteger(value)) return 'integer'
  return typeof value
}

function schemaTypeConstraint(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (
    !resolved || typeof resolved !== 'object' || Array.isArray(resolved)
    || seen.has(resolved)
  ) {
    return null
  }
  if (resolved[PARTIAL_ALTERNATIVE]) {
    throw new Error('property is unconstrained in some alternative branches')
  }
  const nextSeen = new Set(seen).add(resolved)
  let constraint = null
  if (resolved.type !== undefined) {
    constraint = normalizeTypeConstraint(
      (Array.isArray(resolved.type) ? resolved.type : [resolved.type]).filter(
        Boolean,
      ),
    )
  } else if (Array.isArray(resolved.enum)) {
    constraint = new Set(resolved.enum.map(jsonSchemaType))
  } else if (Object.hasOwn(resolved, 'const')) {
    constraint = new Set([jsonSchemaType(resolved.const)])
  }

  for (const variant of resolved.allOf ?? []) {
    constraint = intersectTypeConstraints(
      constraint,
      schemaTypeConstraint(root, variant, nextSeen),
    )
  }

  for (const keyword of ['anyOf', 'oneOf']) {
    const variants = resolved[keyword] ?? []
    if (variants.length) {
      constraint = intersectTypeConstraints(
        constraint,
        unionTypeConstraints(
          variants.map((variant) =>
            schemaTypeConstraint(root, variant, nextSeen)
          ),
        ),
      )
    }
  }

  return constraint
}

function schemaTypes(root, node, { includeNull = false } = {}) {
  const constraint = schemaTypeConstraint(root, node)
  if (constraint === null) {
    return []
  }
  const supported = new Set([
    'array',
    'boolean',
    'integer',
    'number',
    'object',
    'string',
    'null',
  ])
  const unsupported = [...constraint].filter((type) => !supported.has(type))
  const types = [...constraint]
    .filter((type) => includeNull || type !== 'null')
    .sort()
  if (unsupported.length || !types.length) {
    throw new Error(
      `type composition has no supported non-null type${
        unsupported.length ? ` (${unsupported.sort().join(', ')})` : ''
      }`,
    )
  }
  return types
}

const JSON_TYPE_ATOMS = [
  'array',
  'boolean',
  'integer',
  'number',
  'object',
  'string',
  'null',
]

function atomicTypes(types) {
  if (types === undefined) return new Set(JSON_TYPE_ATOMS)
  const atoms = new Set(Array.isArray(types) ? types : [types])
  if (atoms.has('number')) atoms.add('integer')
  return atoms
}

function openLiteralProfile(types) {
  return { accepted: atomicTypes(types), finite: new Map() }
}

function finiteLiteralProfile(values) {
  const accepted = new Set()
  const finite = new Map()
  for (const value of values) {
    const type = jsonSchemaType(value)
    accepted.add(type)
    if (!finite.has(type)) finite.set(type, new Map())
    finite.get(type).set(canonicalJsonString(value), value)
  }
  return { accepted, finite }
}

function intersectLiteralProfiles(left, right) {
  const accepted = new Set(
    [...left.accepted].filter((type) => right.accepted.has(type)),
  )
  const finite = new Map()
  for (const type of [...accepted]) {
    const leftValues = left.finite.get(type)
    const rightValues = right.finite.get(type)
    if (!leftValues && !rightValues) continue
    if (!leftValues || !rightValues) {
      finite.set(type, new Map(leftValues ?? rightValues))
      continue
    }
    const intersection = new Map(
      [...leftValues].filter(([key]) => rightValues.has(key)),
    )
    if (intersection.size) finite.set(type, intersection)
    else accepted.delete(type)
  }
  if (!accepted.size) {
    throw new Error('literal composition has an empty intersection')
  }
  return { accepted, finite }
}

function unionLiteralProfiles(profiles, keyword) {
  const accepted = new Set(
    profiles.flatMap((profile) => [...profile.accepted]),
  )
  const finite = new Map()
  for (const type of accepted) {
    const matching = profiles.filter((profile) => profile.accepted.has(type))
    if (keyword === 'anyOf') {
      if (matching.some((profile) => !profile.finite.has(type))) continue
      finite.set(
        type,
        new Map(
          matching.flatMap((profile) => [...profile.finite.get(type)]),
        ),
      )
      continue
    }

    if (matching.length === 1) {
      const values = matching[0].finite.get(type)
      if (values) finite.set(type, new Map(values))
      continue
    }
    if (matching.some((profile) => !profile.finite.has(type))) {
      throw new Error(
        `oneOf literal composition has overlapping unconstrained ${type} branches`,
      )
    }
    const counts = new Map()
    for (const profile of matching) {
      for (const [key, value] of profile.finite.get(type)) {
        const entry = counts.get(key) ?? { count: 0, value }
        entry.count += 1
        counts.set(key, entry)
      }
    }
    const exclusive = new Map(
      [...counts]
        .filter(([, entry]) => entry.count === 1)
        .map(([key, entry]) => [key, entry.value]),
    )
    if (exclusive.size) finite.set(type, exclusive)
    else accepted.delete(type)
  }
  if (!accepted.size) {
    throw new Error(`${keyword} literal composition accepts no values`)
  }
  return { accepted, finite }
}

function schemaLiteralProfile(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (
    !resolved || typeof resolved !== 'object' || Array.isArray(resolved)
  ) {
    return openLiteralProfile()
  }
  if (seen.has(resolved)) throw new Error('cyclic literal schema composition')
  const nextSeen = new Set(seen).add(resolved)
  let profile = openLiteralProfile(resolved.type)
  if (Array.isArray(resolved.enum)) {
    profile = intersectLiteralProfiles(
      profile,
      finiteLiteralProfile(resolved.enum),
    )
  }
  if (Object.hasOwn(resolved, 'const')) {
    profile = intersectLiteralProfiles(
      profile,
      finiteLiteralProfile([resolved.const]),
    )
  }

  for (const variant of resolved.allOf ?? []) {
    profile = intersectLiteralProfiles(
      profile,
      schemaLiteralProfile(root, variant, nextSeen),
    )
  }

  for (const keyword of ['anyOf', 'oneOf']) {
    const variants = resolved[keyword] ?? []
    if (variants.length) {
      profile = intersectLiteralProfiles(
        profile,
        unionLiteralProfiles(
          variants.map((variant) =>
            schemaLiteralProfile(root, variant, nextSeen)
          ),
          keyword,
        ),
      )
    }
  }

  return profile
}

function schemaEnum(root, node) {
  const profile = schemaLiteralProfile(root, node)
  return [...profile.finite.values()]
    .flatMap((values) => [...values.values()])
    .filter((value) => value !== null)
}

function collectSchemaDefaults(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (!resolved || typeof resolved !== 'object' || Array.isArray(resolved)) {
    return new Map()
  }
  if (seen.has(resolved)) throw new Error('cyclic default schema composition')
  const nextSeen = new Set(seen).add(resolved)
  const defaults = new Map()
  if (Object.hasOwn(resolved, 'default')) {
    defaults.set(canonicalJsonString(resolved.default), resolved.default)
  }
  for (
    const variant of [
      ...(resolved.allOf ?? []),
      ...(resolved.anyOf ?? []),
      ...(resolved.oneOf ?? []),
    ]
  ) {
    for (const [key, value] of collectSchemaDefaults(root, variant, nextSeen)) {
      defaults.set(key, value)
    }
  }
  return defaults
}

function schemaDefault(root, node) {
  const defaults = collectSchemaDefaults(root, node)
  if (defaults.size > 1) {
    throw new Error(
      `default composition has conflicting values: ${
        [...defaults.keys()].sort().join(', ')
      }`,
    )
  }
  const [value] = defaults.values()
  return defaults.size
    ? { present: true, value }
    : { present: false, value: undefined }
}

function schemaSupportsObjectSurface(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (!resolved || typeof resolved !== 'object' || Array.isArray(resolved)) {
    return false
  }
  if (seen.has(resolved)) return false
  seen.add(resolved)
  if (resolved.properties) return true
  const direct = Array.isArray(resolved.type) ? resolved.type : [resolved.type]
  if (direct.includes('object')) return true
  return [
    ...(resolved.allOf ?? []),
    ...(resolved.anyOf ?? []),
    ...(resolved.oneOf ?? []),
  ]
    .some((variant) => schemaSupportsObjectSurface(root, variant, seen))
}

function arrayDomain(kind, schema = null) {
  return { kind, schema }
}

function intersectArrayDomains(left, right) {
  if (left.kind === 'none' || right.kind === 'none') {
    return arrayDomain('none')
  }
  if (left.kind === 'open') return right
  if (right.kind === 'open') return left
  return arrayDomain('schema', { allOf: [left.schema, right.schema] })
}

function unionArrayDomains(domains, keyword) {
  const accepting = domains.filter((domain) => domain.kind !== 'none')
  if (!accepting.length) return arrayDomain('none')
  if (accepting.length === 1) return accepting[0]
  if (accepting.some((domain) => domain.kind === 'open')) {
    if (keyword === 'oneOf') {
      throw new Error(
        'oneOf array composition has overlapping unconstrained array branches',
      )
    }
    return arrayDomain('open')
  }
  return arrayDomain('schema', {
    [keyword]: accepting.map((domain) => domain.schema),
  })
}

function arrayItemSignature(root, schema) {
  return JSON.stringify({
    types: schemaTypes(root, schema),
    enumValues: normalizedLiteralSet(schemaEnum(root, schema)),
  })
}

function directArrayItemDomain(root, resolved) {
  let directTypes = null
  if (resolved.type !== undefined) {
    directTypes = normalizeTypeConstraint(
      Array.isArray(resolved.type) ? resolved.type : [resolved.type],
    )
  } else if (Array.isArray(resolved.enum)) {
    directTypes = normalizeTypeConstraint(resolved.enum.map(jsonSchemaType))
  } else if (Object.hasOwn(resolved, 'const')) {
    directTypes = normalizeTypeConstraint([jsonSchemaType(resolved.const)])
  }
  if (directTypes !== null && !directTypes.has('array')) {
    return arrayDomain('none')
  }

  const prefixItems = resolved.prefixItems ?? []
  const hasMaxItems = Object.hasOwn(resolved, 'maxItems')
  if (
    hasMaxItems
    && (!Number.isInteger(resolved.maxItems) || resolved.maxItems < 0)
  ) {
    throw new Error('array maxItems must be a non-negative integer')
  }
  const maxItems = hasMaxItems ? resolved.maxItems : null
  const reachablePrefixes = maxItems === null
    ? prefixItems
    : prefixItems.slice(0, maxItems)
  const candidates = []
  for (const item of reachablePrefixes) {
    if (item && typeof item === 'object') candidates.push(item)
    else {
      throw new Error(
        'array prefixItems contain a non-object item schema that cannot be compared exactly',
      )
    }
  }

  const closedByMaxItems = maxItems !== null && maxItems <= prefixItems.length
  const remainderAllowed = resolved.items !== false && !closedByMaxItems
  if (remainderAllowed) {
    if (resolved.items && typeof resolved.items === 'object') {
      candidates.push(resolved.items)
    } else {
      return arrayDomain('open')
    }
  }
  if (!candidates.length) return arrayDomain('open')

  const signatures = candidates.map((schema) =>
    arrayItemSignature(root, schema)
  )
  if (!signatures.every((signature) => signature === signatures[0])) {
    throw new Error(
      'array items and prefixItems do not share one effective item contract',
    )
  }
  return arrayDomain('schema', candidates[0])
}

function schemaArrayDomain(root, node, seen = new Set()) {
  const resolved = resolveLocalRef(root, node)
  if (!resolved || typeof resolved !== 'object' || Array.isArray(resolved)) {
    return arrayDomain('open')
  }
  if (resolved[PARTIAL_ALTERNATIVE]) {
    throw new Error('property is unconstrained in some alternative branches')
  }
  if (seen.has(resolved)) throw new Error('cyclic array schema composition')
  const nextSeen = new Set(seen).add(resolved)
  let domain = directArrayItemDomain(root, resolved)

  for (const variant of resolved.allOf ?? []) {
    domain = intersectArrayDomains(
      domain,
      schemaArrayDomain(root, variant, nextSeen),
    )
  }
  for (const keyword of ['anyOf', 'oneOf']) {
    const variants = resolved[keyword] ?? []
    if (!variants.length) continue
    domain = intersectArrayDomains(
      domain,
      unionArrayDomains(
        variants.map((variant) => schemaArrayDomain(root, variant, nextSeen)),
        keyword,
      ),
    )
  }
  return domain
}

function schemaArrayDetails(root, node) {
  const domain = schemaArrayDomain(root, node)
  const details = {
    hasArray: domain.kind !== 'none',
    itemTypes: new Set(),
    itemEnums: new Set(),
  }
  if (domain.kind !== 'schema') return details
  for (const type of schemaTypes(root, domain.schema)) {
    details.itemTypes.add(type)
  }
  for (const value of schemaEnum(root, domain.schema)) {
    details.itemEnums.add(value)
  }
  return details
}

function canonicalJsonValue(value) {
  if (Array.isArray(value)) return value.map(canonicalJsonValue)
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value).sort().map((key) => [
        key,
        canonicalJsonValue(value[key]),
      ]),
    )
  }
  return value
}

function canonicalJsonString(value) {
  return JSON.stringify(canonicalJsonValue(value))
}

function normalizedLiteralSet(values) {
  return [...values].map(canonicalJsonString).sort()
}

function compareLiteralSets(documented, live) {
  const documentedSet = new Set(normalizedLiteralSet(documented ?? []))
  const liveSet = new Set(normalizedLiteralSet(live))
  return {
    missing: [...liveSet].filter((value) => !documentedSet.has(value)),
    extra: [...documentedSet].filter((value) => !liveSet.has(value)),
  }
}

function defaultSignature(value) {
  let source = (value ?? '').trim()
  if (source.startsWith('`') && source.endsWith('`')) {
    source = source.slice(1, -1)
  }
  if (!source) return ''
  try {
    return `json:${canonicalJsonString(JSON.parse(source))}`
  } catch {
    return `text:${source}`
  }
}

function rowSignature(row) {
  return JSON.stringify({
    name: row.name,
    types: row.types,
    itemTypes: row.itemTypes,
    schemaPath: row.schemaPath,
    requiredness: row.requiredness,
    required: row.requiredness === 'global' ? row.required : null,
    enumValues: row.enumValues === null
      ? null
      : normalizedLiteralSet(row.enumValues),
    itemEnumValues: row.itemEnumValues === null
      ? null
      : normalizedLiteralSet(row.itemEnumValues),
    defaultText: defaultSignature(row.defaultText),
  })
}

export function validateMcpContracts(documents, liveTools, references = []) {
  const markers = parseContractMarkers(documents)
  const surfaceMarkers = new Map()
  const toolMarkers = []
  const issues = []

  for (const marker of markers) {
    if (marker.kind === 'mcp' || marker.kind === 'mcp-surface') {
      if (!Object.hasOwn(marker.attributes, 'schema')) {
        throw markerError(
          marker,
          `${marker.kind} marker needs explicit schema=`,
        )
      }
      if (!Object.hasOwn(marker.attributes, 'requiredness')) {
        throw markerError(
          marker,
          `${marker.kind} marker needs explicit requiredness=`,
        )
      }
      markerRequiredness(marker)
    }
    if (marker.kind === 'mcp-surface') {
      const name = marker.attributes.name
      if (!name) throw markerError(marker, 'MCP surface marker needs name=')
      if (surfaceMarkers.has(name)) {
        throw markerError(marker, `duplicate MCP surface: ${name}`)
      }
      surfaceMarkers.set(name, marker)
    } else if (marker.kind === 'mcp') {
      if (!marker.attributes.tool) {
        throw markerError(marker, 'MCP marker needs tool=')
      }
      toolMarkers.push(marker)
    }
  }

  const liveByName = new Map(liveTools.map((tool) => [tool.name, tool]))
  const referenceByName = new Map(
    references.map((reference) => [reference.name, reference]),
  )
  const markedTools = new Set()
  const rootMarkedTools = new Set()
  const groups = new Map()
  for (const marker of toolMarkers) {
    const toolName = marker.attributes.tool
    markedTools.add(toolName)
    if (markerSchemaPath(marker) === '/') rootMarkedTools.add(toolName)
    if (!liveByName.has(toolName)) {
      issues.push(`${marker.file}:${marker.line}: unknown MCP tool ${toolName}`)
      continue
    }
    const reference = referenceByName.get(toolName)
    if (reference) {
      const slug = reference.route.split('/').filter(Boolean).at(-1)
      const expectedFile = `site/src/content/docs/mcp-tools/${slug}.mdx`
      const actualFile = marker.file.replaceAll(path.sep, '/')
      if (!actualFile.endsWith(expectedFile)) {
        issues.push(
          `${marker.file}:${marker.line}: ${toolName} contract belongs in ${expectedFile}`,
        )
      }
    }
    const key = `${toolName}\u0000${markerSchemaPath(marker)}`
    if (!groups.has(key)) {
      groups.set(key, {
        toolName,
        schemaPath: markerSchemaPath(marker),
        markers: [],
      })
    }
    groups.get(key).markers.push(marker)
  }

  for (const tool of liveTools) {
    if (!markedTools.has(tool.name)) {
      const reference = referenceByName.get(tool.name)
      if (reference) {
        const slug = reference.route.split('/').filter(Boolean).at(-1)
        issues.push(
          `site/src/content/docs/mcp-tools/${slug}.mdx:1: MCP tool ${tool.name} has no marked contract surface`,
        )
      } else {
        issues.push(
          `site/src/data/tool-reference.mjs:1: MCP tool ${tool.name} has no marked contract surface and no canonical page mapping`,
        )
      }
    } else if (!rootMarkedTools.has(tool.name)) {
      const marker = toolMarkers.find((candidate) =>
        candidate.attributes.tool === tool.name
      )
      issues.push(
        `${marker.file}:${marker.line}: MCP tool ${tool.name} has no root schema=/ contract surface`,
      )
    }
  }

  const expandSurface = (name, stack = []) => {
    if (stack.includes(name)) {
      throw new Error(
        `cyclic MCP surface include: ${[...stack, name].join(' -> ')}`,
      )
    }
    const marker = surfaceMarkers.get(name)
    if (!marker) throw new Error(`unknown MCP surface include: ${name}`)
    const rows = parseMcpRows(marker)
    for (const include of markerIncludes(marker)) {
      rows.push(...expandSurface(include, [...stack, name]))
    }
    return rows
  }

  for (const group of groups.values()) {
    const tool = liveByName.get(group.toolName)
    const root = tool.inputSchema ?? {}
    const firstMarker = group.markers[0]
    let schemaNode
    try {
      schemaNode = schemaAt(root, group.schemaPath)
    } catch (error) {
      issues.push(
        `${firstMarker.file}:${firstMarker.line}: ${group.toolName} schema path ${group.schemaPath} failed to resolve: ${error.message}`,
      )
      continue
    }
    if (!schemaSupportsObjectSurface(root, schemaNode)) {
      issues.push(
        `${firstMarker.file}:${firstMarker.line}: ${group.toolName} schema path ${group.schemaPath} does not resolve to an object schema`,
      )
      continue
    }
    if (
      group.markers.some((marker) => marker.attributes.surface === 'none')
      && group.markers.some((marker) => marker.attributes.surface !== 'none')
    ) {
      issues.push(
        `${firstMarker.file}:${firstMarker.line}: ${group.toolName} mixes empty and non-empty contract surfaces for schema ${group.schemaPath}`,
      )
      continue
    }
    let properties
    let required
    try {
      const surface = schemaProperties(root, group.schemaPath)
      properties = surface.properties
      required = surface.required
    } catch (error) {
      issues.push(
        `${firstMarker.file}:${firstMarker.line}: ${group.toolName} live object schema composition is unsupported: ${error.message}`,
      )
      continue
    }
    const combined = []

    try {
      for (const marker of group.markers) {
        if (marker.attributes.surface === 'none') {
          if (Object.keys(properties).length) {
            issues.push(
              `${marker.file}:${marker.line}: ${group.toolName} declares an empty surface but live schema has ${
                Object.keys(properties).sort().join(', ')
              }`,
            )
          }
          continue
        }
        combined.push(...parseMcpRows(marker))
        for (const include of markerIncludes(marker)) {
          combined.push(...expandSurface(include))
        }
      }
    } catch (error) {
      const message = error.message
      issues.push(
        /:\d+:/.test(message)
          ? message
          : `${firstMarker.file}:${firstMarker.line}: ${message}`,
      )
      continue
    }

    const documented = new Map()
    for (const row of combined) {
      if (row.schemaPath !== group.schemaPath) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: included field ${row.name} uses schema ${row.schemaPath}, expected ${group.schemaPath}`,
        )
        continue
      }
      const property = properties[row.name]
      if (!property) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} is not in live schema ${group.schemaPath}`,
        )
        continue
      }

      let liveTypes
      let arrayDetails
      let liveEnum
      let liveDefault
      try {
        liveTypes = schemaTypes(root, property)
        arrayDetails = schemaArrayDetails(root, property)
        liveEnum = schemaEnum(root, property)
        liveDefault = schemaDefault(root, property)
      } catch (error) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} live schema composition is unsupported: ${error.message}`,
        )
        continue
      }
      if (
        liveTypes.length
        && JSON.stringify(row.types) !== JSON.stringify(liveTypes)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} type is ${
            row.types.join('|') || 'unknown'
          }, live schema is ${liveTypes.join('|') || 'unknown'}`,
        )
      }
      const liveItemTypes = [...arrayDetails.itemTypes].sort()
      if (
        row.itemTypes.length
        && JSON.stringify(row.itemTypes) !== JSON.stringify(liveItemTypes)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} array item type is ${
            row.itemTypes.join('|')
          }, live schema is ${liveItemTypes.join('|') || 'unconstrained'}`,
        )
      }
      if (
        row.requiredness === 'global' && row.required !== required.has(row.name)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} requiredness disagrees with live schema`,
        )
      }
      const enumDifference = compareLiteralSets(row.enumValues, liveEnum)
      if (enumDifference.missing.length || enumDifference.extra.length) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} enum differs; missing ${
            enumDifference.missing.join(', ') || 'none'
          }; extra ${enumDifference.extra.join(', ') || 'none'}`,
        )
      }
      const liveItemEnum = [...arrayDetails.itemEnums]
      const itemEnumDifference = compareLiteralSets(
        row.itemEnumValues,
        liveItemEnum,
      )
      if (
        itemEnumDifference.missing.length || itemEnumDifference.extra.length
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} array item enum differs; missing ${
            itemEnumDifference.missing.join(', ') || 'none'
          }; extra ${itemEnumDifference.extra.join(', ') || 'none'}`,
        )
      }
      if (liveDefault.present) {
        let documentedDefault
        let defaultParseFailed = false
        try {
          documentedDefault = parseDocumentedJsonLiteral(
            row.defaultText,
            row.marker,
            `${row.name} Default`,
          )
        } catch (error) {
          issues.push(error.message)
          defaultParseFailed = true
          documentedDefault = { present: false, value: undefined }
        }
        if (!defaultParseFailed && !documentedDefault.present) {
          issues.push(
            `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} omits live default ${
              canonicalJsonString(liveDefault.value)
            } from a dedicated Default column`,
          )
        } else if (
          !defaultParseFailed
          && canonicalJsonString(documentedDefault.value)
            !== canonicalJsonString(liveDefault.value)
        ) {
          issues.push(
            `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} default is ${
              canonicalJsonString(documentedDefault.value)
            }, live schema is ${canonicalJsonString(liveDefault.value)}`,
          )
        }
      }

      const signature = rowSignature(row)
      if (documented.has(row.name) && documented.get(row.name) !== signature) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: conflicting duplicate field ${group.toolName}.${row.name}`,
        )
      } else {
        documented.set(row.name, signature)
      }
    }

    for (const name of Object.keys(properties).sort()) {
      if (
        !documented.has(name)
        && !group.markers.some((marker) => marker.attributes.surface === 'none')
      ) {
        issues.push(
          `${firstMarker.file}:${firstMarker.line}: ${group.toolName}.${name} is missing from marked schema ${group.schemaPath}`,
        )
      }
    }
  }

  if (issues.length) throw new Error([...new Set(issues)].sort().join('\n'))
}

export function validateMcpOutputContracts(
  documents,
  liveTools,
  references = [],
) {
  const markers = parseContractMarkers(documents).filter((marker) =>
    marker.kind === 'mcp-output'
  )
  const liveByName = new Map(liveTools.map((tool) => [tool.name, tool]))
  const referenceByName = new Map(
    references.map((reference) => [reference.name, reference]),
  )
  const groups = new Map()
  const rootMarked = new Set()
  const issues = []

  for (const marker of markers) {
    if (!Object.hasOwn(marker.attributes, 'schema')) {
      throw markerError(marker, 'mcp-output marker needs explicit schema=')
    }
    if (!Object.hasOwn(marker.attributes, 'requiredness')) {
      throw markerError(
        marker,
        'mcp-output marker needs explicit requiredness=',
      )
    }
    markerRequiredness(marker)
    const toolName = marker.attributes.tool
    if (!toolName) throw markerError(marker, 'MCP output marker needs tool=')
    if (!MCP_OUTPUT_CONTRACT_TOOLS.has(toolName)) {
      issues.push(
        `${marker.file}:${marker.line}: ${toolName} is not a selected MCP output contract`,
      )
      continue
    }
    const tool = liveByName.get(toolName)
    if (!tool) {
      issues.push(`${marker.file}:${marker.line}: unknown MCP tool ${toolName}`)
      continue
    }
    if (!tool.outputSchema) {
      issues.push(
        `${marker.file}:${marker.line}: ${toolName} does not advertise outputSchema`,
      )
      continue
    }
    const reference = referenceByName.get(toolName)
    if (reference) {
      const slug = reference.route.split('/').filter(Boolean).at(-1)
      const expectedFile = `site/src/content/docs/mcp-tools/${slug}.mdx`
      const actualFile = marker.file.replaceAll(path.sep, '/')
      if (!actualFile.endsWith(expectedFile)) {
        issues.push(
          `${marker.file}:${marker.line}: ${toolName} output contract belongs in ${expectedFile}`,
        )
      }
    }
    const schemaPath = markerSchemaPath(marker)
    if (schemaPath === '/') rootMarked.add(toolName)
    const key = `${toolName}\u0000${schemaPath}`
    if (!groups.has(key)) {
      groups.set(key, { toolName, schemaPath, markers: [] })
    }
    groups.get(key).markers.push(marker)
  }

  for (const toolName of MCP_OUTPUT_CONTRACT_TOOLS) {
    const tool = liveByName.get(toolName)
    if (!tool) {
      issues.push(
        `src/tools/mod.rs:1: selected MCP output tool missing: ${toolName}`,
      )
    } else if (!tool.outputSchema) {
      issues.push(
        `src/tools/mod.rs:1: selected MCP output tool has no outputSchema: ${toolName}`,
      )
    } else if (!rootMarked.has(toolName)) {
      const reference = referenceByName.get(toolName)
      const slug = reference?.route.split('/').filter(Boolean).at(-1)
      issues.push(
        `${
          slug
            ? `site/src/content/docs/mcp-tools/${slug}.mdx`
            : 'src/tools/mod.rs'
        }:1: ${toolName} has no root doc-contract:mcp-output surface`,
      )
    }
  }

  for (const group of groups.values()) {
    const tool = liveByName.get(group.toolName)
    if (!tool?.outputSchema) continue
    const root = tool.outputSchema
    const firstMarker = group.markers[0]
    let properties
    let required
    try {
      const node = schemaAt(root, group.schemaPath)
      if (!schemaSupportsObjectSurface(root, node)) {
        throw new Error('does not resolve to an object schema')
      }
      const surface = schemaProperties(root, group.schemaPath)
      properties = surface.properties
      required = surface.required
    } catch (error) {
      issues.push(
        `${firstMarker.file}:${firstMarker.line}: ${group.toolName} output schema path ${group.schemaPath} failed: ${error.message}`,
      )
      continue
    }

    const rows = []
    try {
      for (const marker of group.markers) rows.push(...parseMcpRows(marker))
    } catch (error) {
      issues.push(error.message)
      continue
    }
    const documented = new Map()
    for (const row of rows) {
      const property = properties[row.name]
      if (!property) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} is not in live output schema ${group.schemaPath}`,
        )
        continue
      }
      let liveTypes
      let liveItemTypes
      try {
        liveTypes = schemaTypes(root, property, {
          includeNull: required.has(row.name),
        })
        liveItemTypes = [...schemaArrayDetails(root, property).itemTypes].sort()
      } catch (error) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} output schema composition is unsupported: ${error.message}`,
        )
        continue
      }
      if (
        liveTypes.length
        && JSON.stringify(row.types) !== JSON.stringify(liveTypes)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} output type is ${
            row.types.join('|') || 'unknown'
          }, live schema is ${liveTypes.join('|') || 'unknown'}`,
        )
      }
      if (
        row.itemTypes.length
        && JSON.stringify(row.itemTypes) !== JSON.stringify(liveItemTypes)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} output array item type is ${
            row.itemTypes.join('|')
          }, live schema is ${liveItemTypes.join('|') || 'unconstrained'}`,
        )
      }
      if (
        row.requiredness === 'global'
        && row.required !== required.has(row.name)
      ) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: ${group.toolName}.${row.name} output requiredness disagrees with live schema`,
        )
      }
      const signature = rowSignature(row)
      if (documented.has(row.name) && documented.get(row.name) !== signature) {
        issues.push(
          `${row.marker.file}:${row.marker.line}: conflicting duplicate output field ${group.toolName}.${row.name}`,
        )
      } else {
        documented.set(row.name, signature)
      }
    }
    for (const name of Object.keys(properties).sort()) {
      if (!documented.has(name)) {
        issues.push(
          `${firstMarker.file}:${firstMarker.line}: ${group.toolName}.${name} is missing from marked output schema ${group.schemaPath}`,
        )
      }
    }
  }

  if (issues.length) throw new Error([...new Set(issues)].sort().join('\n'))
}

function sopPropertiesByTool(liveTools) {
  const propertiesByName = new Map()
  for (const tool of liveTools) {
    try {
      propertiesByName.set(
        tool.name,
        schemaProperties(tool.inputSchema ?? {}, '/').properties,
      )
    } catch (error) {
      throw new Error(
        `src/tools/params.rs:1: SOP tool ${tool.name} input schema composition is unsupported: ${error.message}`,
      )
    }
  }
  return propertiesByName
}

export function extractSopCalls(
  content,
  liveTools,
  propertiesByName = sopPropertiesByTool(liveTools),
) {
  const liveByName = new Map(liveTools.map((tool) => [tool.name, tool]))
  const zeroParameterNames = new Set(
    liveTools
      .filter((tool) =>
        Object.keys(propertiesByName.get(tool.name)).length === 0
      )
      .map((tool) => tool.name),
  )
  const calls = []
  const errors = []
  const lines = content.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const opening = lines[index].match(/^```\s*(.*)$/)
    if (!opening) continue
    const label = opening[1].trim()
    const body = []
    const startLine = index + 2
    index += 1
    while (index < lines.length && !/^```\s*$/.test(lines[index])) {
      body.push(lines[index])
      index += 1
    }
    if (label) continue

    const source = body.join('\n')
    const first = source.trimStart()
    const firstName = first.match(/^([a-z][a-z0-9_]*)/i)?.[1]
    const startsCall = /^[a-z][a-z0-9_]*\s*\(/.test(first)
    const startsBare = firstName && zeroParameterNames.has(firstName)
      && first.split('\n')[0].trim() === firstName
    const parameterizedBare = firstName && liveByName.has(firstName)
      && !zeroParameterNames.has(firstName)
      && first.split('\n')[0].trim() === firstName
    if (parameterizedBare) {
      errors.push({
        line: startLine,
        message:
          `malformed SOP tool-call fence: ${firstName} requires call syntax with arguments`,
      })
      continue
    }
    if (!startsCall && !startsBare) continue

    const scanned = scanToolCalls(source, zeroParameterNames)
    for (const call of scanned.calls) {
      const line = startLine
        + source.slice(0, call.offset).split('\n').length - 1
      calls.push({ ...call, line })
    }
    if (scanned.error) {
      const line = startLine
        + source.slice(0, scanned.error.offset).split('\n').length - 1
      errors.push({ line, message: scanned.error.message })
    }
  }
  return { calls, errors, liveByName }
}

function scanToolCalls(source, zeroParameterNames) {
  const calls = []
  let index = 0
  while (index < source.length) {
    while (/\s/.test(source[index] ?? '')) index += 1
    const offset = index
    if (/[)\]}]/.test(source[index] ?? '')) {
      return {
        calls,
        error: {
          offset,
          message: `malformed SOP tool-call fence: unexpected ${source[index]}`,
        },
      }
    }
    const nameMatch = source.slice(index).match(/^([a-z][a-z0-9_]*)/)
    if (!nameMatch) break
    const name = nameMatch[1]
    index += name.length
    while (/[ \t]/.test(source[index] ?? '')) index += 1

    if (source[index] !== '(') {
      const lineEnd = source.indexOf('\n', index)
      const remainder = source.slice(
        index,
        lineEnd < 0 ? source.length : lineEnd,
      ).trim()
      if (remainder || !zeroParameterNames.has(name)) break
      calls.push({ name, namedArgs: [], offset })
      index = lineEnd < 0 ? source.length : lineEnd + 1
      continue
    }

    const balanced = findBalancedCallEnd(source, index)
    if (!balanced.ok) return { calls, error: balanced.error }
    const args = source.slice(index + 1, balanced.end)
    calls.push({ name, namedArgs: topLevelNamedArgs(args), offset })
    index = balanced.end + 1
  }
  return { calls, error: null }
}

function findBalancedCallEnd(source, opening) {
  const stack = [{ char: '(', offset: opening }]
  let quote = null
  let escaped = false
  const pairs = { ')': '(', ']': '[', '}': '{' }
  for (let index = opening + 1; index < source.length; index += 1) {
    const char = source[index]
    if (quote) {
      if (escaped) escaped = false
      else if (char === '\\') escaped = true
      else if (char === quote) quote = null
      continue
    }
    if (char === '"' || char === "'") quote = char
    else if (char === '(' || char === '[' || char === '{') {
      stack.push({ char, offset: index })
    } else if (pairs[char]) {
      if (stack.at(-1)?.char !== pairs[char]) {
        const expected = { '(': ')', '[': ']', '{': '}' }[stack.at(-1)?.char]
          ?? 'opening delimiter'
        return {
          ok: false,
          error: {
            offset: index,
            message:
              `malformed SOP tool-call fence: found ${char} while expecting ${expected}`,
          },
        }
      }
      stack.pop()
      if (!stack.length) return { ok: true, end: index }
    }
  }
  const expected = { '(': ')', '[': ']', '{': '}' }[stack.at(-1)?.char]
    ?? 'closing delimiter'
  return {
    ok: false,
    error: {
      offset: Math.max(opening, source.length - 1),
      message: `malformed SOP tool-call fence: missing ${expected}`,
    },
  }
}

function topLevelNamedArgs(source) {
  const args = []
  let start = 0
  let quote = null
  let escaped = false
  const stack = []
  const flush = (end) => {
    const token = source.slice(start, end).trim()
    const match = token.match(/^([a-z][a-z0-9_]*)\s*=/)
    if (match) args.push(match[1])
  }
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index]
    if (quote) {
      if (escaped) escaped = false
      else if (char === '\\') escaped = true
      else if (char === quote) quote = null
      continue
    }
    if (char === '"' || char === "'") quote = char
    else if (char === '(' || char === '[' || char === '{') stack.push(char)
    else if (char === ')' || char === ']' || char === '}') stack.pop()
    else if (char === ',' && !stack.length) {
      flush(index)
      start = index + 1
    }
  }
  flush(source.length)
  return args
}

export function validateSopContracts(documents, liveTools) {
  const issues = []
  const liveByName = new Map(liveTools.map((tool) => [tool.name, tool]))
  const propertiesByName = sopPropertiesByTool(liveTools)
  for (const document of documents) {
    const { calls, errors } = extractSopCalls(
      document.content,
      liveTools,
      propertiesByName,
    )
    for (const error of errors) {
      issues.push(`${document.file}:${error.line}: ${error.message}`)
    }
    for (const call of calls) {
      const tool = liveByName.get(call.name)
      if (!tool) {
        issues.push(
          `${document.file}:${call.line}: unknown SOP tool ${call.name}`,
        )
        continue
      }
      const properties = propertiesByName.get(call.name)
      for (const name of call.namedArgs) {
        if (!Object.hasOwn(properties, name)) {
          issues.push(
            `${document.file}:${call.line}: ${call.name} has no top-level argument ${name}`,
          )
        }
      }
    }
  }
  if (issues.length) throw new Error(issues.sort().join('\n'))
}

export function parseRootCliHelp(output) {
  const commands = new Map()
  let inCommands = false
  for (const line of output.split('\n')) {
    if (line.trim() === 'Commands:') {
      inCommands = true
      continue
    }
    if (inCommands && /^[A-Za-z][A-Za-z ]+:$/.test(line.trim())) break
    if (!inCommands) continue
    const match = line.match(/^\s{2}([a-z][a-z0-9-]*)\s{2,}(.*)$/)
    if (match && match[1] !== 'help') {
      commands.set(match[1], { description: match[2] })
    }
  }
  return commands
}

function parseCliOptionLine(line) {
  const option = line.match(
    /^\s{2,}(?:(-[a-zA-Z0-9])(?:,\s+(--[a-z0-9][a-z0-9-]*))?|(--[a-z0-9][a-z0-9-]*))(?=\s|$)/,
  )
  if (!option) return null
  const short = option[1] ?? ''
  const long = option[2] ?? option[3] ?? ''
  return {
    name: long || short,
    short: long ? short : '',
  }
}

export function parseApplicationCliHelp(
  output,
  excludedOptions = GENERATED_APPLICATION_CLI_OPTIONS,
) {
  const fields = new Map()
  const lines = output.split('\n')
  for (let index = 0; index < lines.length; index += 1) {
    const argument = lines[index].match(
      /^\s{2}(?:<([^>]+)>|\[([^\]]+)\])(\.\.\.)?(?:\s{2,}.*)?$/,
    )
    if (argument) {
      const name = argument[1]
        ? `<${argument[1].toLowerCase()}>`
        : `[${argument[2].toLowerCase()}]`
      fields.set(name, { name, kind: 'argument', short: '', default: null })
      continue
    }

    const option = parseCliOptionLine(lines[index])
    if (
      !option || excludedOptions.has(option.name)
      || (option.short && excludedOptions.has(option.short))
    ) continue
    let block = lines[index]
    let cursor = index + 1
    while (
      cursor < lines.length
      && !parseCliOptionLine(lines[cursor])
      && !/^\s{2}(?:<[^>]+>|\[[^\]]+\])/.test(lines[cursor])
      && !/^[A-Za-z][A-Za-z ]+:$/.test(lines[cursor].trim())
    ) {
      block += `\n${lines[cursor]}`
      cursor += 1
    }
    const defaultMatch = block.match(/\[default:\s*([^\]]+)\]/)
    fields.set(option.name, {
      name: option.name,
      kind: 'option',
      short: option.short,
      default: defaultMatch?.[1].trim() ?? null,
    })
  }
  return fields
}

export function parseRootCliOptions(output) {
  return parseApplicationCliHelp(output, GENERATED_ROOT_CLI_OPTIONS)
}

function parseCliRows(marker) {
  if (marker.attributes.surface === 'none') return []
  const table = parseMarkdownTable(marker.body, marker)
  const nameHeader = table.headers.includes('command') ? 'command' : 'flag'
  if (!table.headers.includes(nameHeader)) {
    throw markerError(marker, 'CLI table needs Command or Flag column')
  }
  return table.rows.map((row) => ({
    name: row.columns[nameHeader].replace(/`/g, '').replace(/\*\*/g, '').trim()
      .toLowerCase(),
    short: stripMarkdown(row.columns.short ?? ''),
    defaultText: stripMarkdown(row.columns.default ?? ''),
    marker,
  }))
}

function validateCliFieldSurface(command, markers, liveFields, issues) {
  if (!markers.length) {
    issues.push(
      `site/src/content/docs/cli/index.mdx:1: CLI command ${command} has no marked contract surface`,
    )
    return
  }

  if (
    markers.some((marker) => marker.attributes.surface === 'none')
    && markers.some((marker) => marker.attributes.surface !== 'none')
  ) {
    issues.push(
      `${markers[0].file}:${
        markers[0].line
      }: CLI command ${command} mixes empty and non-empty contract surfaces`,
    )
    return
  }

  const rows = markers.flatMap(parseCliRows)
  if (markers.some((marker) => marker.attributes.surface === 'none')) {
    if (liveFields.size) {
      issues.push(
        `${markers[0].file}:${
          markers[0].line
        }: CLI command ${command} declares empty but has ${
          [...liveFields.keys()].sort().join(', ')
        }`,
      )
    }
    return
  }

  const documented = new Map()
  for (const row of rows) {
    if (documented.has(row.name)) {
      issues.push(
        `${row.marker.file}:${row.marker.line}: duplicate CLI field ${command} ${row.name}`,
      )
      continue
    }
    documented.set(row.name, row)
    const live = liveFields.get(row.name)
    if (!live) {
      issues.push(
        `${row.marker.file}:${row.marker.line}: unknown CLI field ${command} ${row.name}`,
      )
      continue
    }
    if (row.short !== live.short) {
      issues.push(
        `${row.marker.file}:${row.marker.line}: ${command} ${row.name} short flag drift`,
      )
    }
    if (live.default !== null && row.defaultText !== live.default) {
      issues.push(
        `${row.marker.file}:${row.marker.line}: ${command} ${row.name} default is ${
          row.defaultText || 'undocumented'
        }, live help is ${live.default}`,
      )
    }
  }
  for (const name of [...liveFields.keys()].sort()) {
    if (!documented.has(name)) {
      issues.push(
        `${markers[0].file}:${
          markers[0].line
        }: CLI command ${command} is missing ${name}`,
      )
    }
  }
}

export function validateCliContracts(documents, inventory) {
  const markers = parseContractMarkers(documents).filter((marker) =>
    marker.kind === 'cli'
  )
  const issues = []
  const byCommand = new Map()
  for (const marker of markers) {
    const command = marker.attributes.command
    if (!command) throw markerError(marker, 'CLI marker needs command=')
    if (!byCommand.has(command)) byCommand.set(command, [])
    byCommand.get(command).push(marker)
  }

  const expectedCommands = [...inventory.commands.keys()].sort()
  const rootMarkers = byCommand.get('root') ?? []
  const rootCommandMarkers = rootMarkers.filter((marker) =>
    marker.attributes.surface === 'commands'
  )
  const rootRows = rootCommandMarkers.flatMap(parseCliRows)
  const documentedCommands = [...new Set(rootRows.map((row) => row.name))]
    .sort()
  if (JSON.stringify(documentedCommands) !== JSON.stringify(expectedCommands)) {
    const location = rootCommandMarkers.length
      ? `${rootCommandMarkers[0].file}:${rootCommandMarkers[0].line}: `
      : 'site/src/content/docs/cli/index.mdx:1: '
    issues.push(
      `${location}CLI command inventory mismatch; documented: ${
        documentedCommands.join(', ')
      }; live: ${expectedCommands.join(', ')}`,
    )
  }

  validateCliFieldSurface(
    'root',
    rootMarkers.filter((marker) => marker.attributes.surface !== 'commands'),
    inventory.fields.get('root') ?? new Map(),
    issues,
  )

  for (const command of expectedCommands) {
    const commandMarkers = byCommand.get(command) ?? []
    const liveFields = inventory.fields.get(command) ?? new Map()
    validateCliFieldSurface(command, commandMarkers, liveFields, issues)
  }

  for (const command of byCommand.keys()) {
    if (command !== 'root' && !inventory.commands.has(command)) {
      const marker = byCommand.get(command)[0]
      issues.push(
        `${marker.file}:${marker.line}: documented CLI command is not live: ${command}`,
      )
    }
  }
  if (issues.length) throw new Error(issues.sort().join('\n'))
}

export function readCliInventory(bin) {
  const root = runHelp(bin, ['--help'])
  const commands = parseRootCliHelp(root)
  const fields = new Map([['root', parseRootCliOptions(root)]])
  for (const command of commands.keys()) {
    fields.set(
      command,
      parseApplicationCliHelp(runHelp(bin, [command, '--help'])),
    )
  }
  return { commands, fields }
}

function runHelp(bin, args) {
  const source = args[0]?.startsWith('--')
    ? 'src/main.rs:1'
    : 'src/cli/mod.rs:1'
  const result = spawnSync(bin, args, { encoding: 'utf8', timeout: 30_000 })
  if (result.error) {
    throw new Error(
      `${source}: ${bin} ${args.join(' ')} failed: ${result.error.message}`,
    )
  }
  if (result.status !== 0) {
    throw new Error(
      `${source}: ${bin} ${args.join(' ')} exited ${result.status}: ${
        (result.stderr ?? '').trim()
      }`,
    )
  }
  return `${result.stdout ?? ''}\n${result.stderr ?? ''}`
}

export function compareRuntimeHelp(workflows, payload) {
  const menu = workflows
    .filter((workflow) => workflow.runtimeHelp)
    .sort((left, right) =>
      left.runtimeHelp.menuOrder - right.runtimeHelp.menuOrder
    )
  const recommended = menu
    .filter((workflow) => workflow.runtimeHelp.recommendedOrder !== null)
    .sort(
      (left, right) =>
        left.runtimeHelp.recommendedOrder - right.runtimeHelp.recommendedOrder,
    )
  const liveMenu = (payload.workflows ?? []).map((workflow) => workflow.name)
  const expectedMenu = menu.map((workflow) => workflow.title)
  if (JSON.stringify(liveMenu) !== JSON.stringify(expectedMenu)) {
    throw new Error(
      `src/tools/help_handler.rs:1: runtime help menu drift; live: ${
        liveMenu.join(', ')
      }; canonical: ${expectedMenu.join(', ')}`,
    )
  }
  const liveRecommended = String(payload.recommended_order ?? '')
    .split('\n')
    .map((line) => line.match(/^\d+\.\s+(.+?)\s+—/)?.[1])
    .filter(Boolean)
  const expectedRecommended = recommended.map((workflow) => workflow.title)
  if (JSON.stringify(liveRecommended) !== JSON.stringify(expectedRecommended)) {
    throw new Error(
      `src/tools/help_handler.rs:1: runtime recommended order drift; live: ${
        liveRecommended.join(', ')
      }; canonical: ${expectedRecommended.join(', ')}`,
    )
  }
}

export function runtimeHelpTopics(workflows) {
  return workflows
    .filter((workflow) => workflow.runtimeHelp)
    .sort((left, right) =>
      left.runtimeHelp.menuOrder - right.runtimeHelp.menuOrder
    )
    .map((workflow) => workflow.runtimeHelp.topic)
}

export function validateXmlBackupContracts(
  workflows,
  xmlBackupSuccessCondition,
) {
  const source = 'site/src/data/workflows.mjs:1'
  if (xmlBackupSuccessCondition !== REQUIRED_XML_BACKUP_SUCCESS_CONDITION) {
    throw new Error(
      `${source}: XML_BACKUP_SUCCESS_CONDITION must equal the canonical fail-closed condition`,
    )
  }

  workflows.forEach((workflow, index) => {
    const outputs = workflow?.sideEffects?.outputs ?? []
    const hasXml = outputs.some((entry) =>
      entry.kind === 'metadata-xml' || entry.kind === 'playlist-xml'
    )
    const backups = outputs.filter((entry) => entry.kind === 'backup')
    const record = `${source}: workflows[${index}] (${
      workflow?.id ?? 'unknown'
    })`

    if (!hasXml) {
      if (backups.length > 0) {
        throw new Error(`${record} declares a backup without XML output`)
      }
      return
    }
    if (backups.length !== 1) {
      throw new Error(
        `${record} must declare exactly one backup output for XML export`,
      )
    }
    if (backups[0].mode !== 'on-export') {
      throw new Error(`${record} XML backup mode must be on-export`)
    }
    if (backups[0].condition !== xmlBackupSuccessCondition) {
      throw new Error(
        `${record} XML backup condition must equal XML_BACKUP_SUCCESS_CONDITION`,
      )
    }
  })
}

export function validateBuiltLinkSet(htmlDocuments, builtPaths) {
  const issues = []
  for (const document of htmlDocuments) {
    const pattern = /\b(?:href|src)=["']([^"']+)["']/g
    let match
    while ((match = pattern.exec(document.content)) !== null) {
      const line = lineNumberAt(document.content, match.index)
      const original = match[1]
      if (/^[#?]/.test(original)) continue
      const emittingPath = normalizeBuiltDocumentPath(document).replace(
        /^\/+/,
        '',
      )
      let resolved
      try {
        resolved = new URL(
          original,
          new URL(`/${emittingPath}`, `${SITE_ORIGIN}/`),
        )
      } catch {
        continue
      }
      if (resolved.origin !== SITE_ORIGIN) continue
      const target = resolved.pathname
      if (!target || target === '/') continue
      const candidates = builtCandidates(target)
      if (!candidates.some((candidate) => builtPaths.has(candidate))) {
        issues.push(
          `${document.file}:${line}: built target missing for ${original} (resolved ${target})`,
        )
      }
    }
  }
  if (issues.length) throw new Error([...new Set(issues)].sort().join('\n'))
}

function normalizeBuiltDocumentPath(document) {
  if (document.builtPath) return document.builtPath.replaceAll(path.sep, '/')
  const normalized = document.file.replaceAll(path.sep, '/')
  if (normalized.startsWith('dist/')) return normalized.slice('dist/'.length)
  const distMarker = normalized.lastIndexOf('/dist/')
  return distMarker >= 0
    ? normalized.slice(distMarker + '/dist/'.length)
    : normalized
}

function builtCandidates(target) {
  const directoryRoute = target.endsWith('/')
  const relative = target.replace(/^\//, '').replace(/\/$/, '')
  if (!directoryRoute && path.extname(relative)) return [relative]
  return [relative, `${relative}.html`, path.join(relative, 'index.html')]
}

export function validateRuntimeHelpUrls(payloads, builtPaths) {
  const issues = []
  for (const { source, payload } of payloads) {
    const matches =
      JSON.stringify(payload).match(/https:\/\/reklawdbox\.com\/[^"\\\s<>\]]+/g)
        ?? []
    for (const matched of matches) {
      const url = matched.replace(/[),.;]+$/, '')
      const route = new URL(url).pathname
      if (
        !builtCandidates(route).some((candidate) => builtPaths.has(candidate))
      ) {
        issues.push(`${source}: runtime-help URL is not built: ${url}`)
      }
    }
  }
  if (issues.length) throw new Error([...new Set(issues)].sort().join('\n'))
}

export const FIRST_SESSION_PROMPT =
  `Use only reklawdbox's read_library tool. Show me:
- my total track and playlist counts
- my top genres
- my average BPM and key distribution

Do not call external services, analyze audio, use or populate enrichment/audio
caches, write files, stage changes, or export XML.`

const FIRST_SESSION_BOUNDARIES = [
  ['external services', /external services/],
  ['audio analysis', /analyze audio/],
  ['enrichment/audio caches', /use or populate enrichment\/audio\s+caches/],
  ['file writes', /write files/],
  ['staged changes', /stage changes/],
  ['XML export', /export XML/],
]

function fencedBlocks(source) {
  return [...source.matchAll(/^(`{3,}|~{3,})([^\n]*)\n([\s\S]*?)^\1[ \t]*$/gm)]
    .map((match) => ({
      body: match[3],
      full: match[0],
      index: match.index,
      info: match[2].trim(),
    }))
}

/**
 * Validate the source-only shape of the bounded first-session prompt against
 * the live MCP tool inventory.
 * @param {{ file: string, content: string }} document
 * @param {object[]} liveTools
 */
export function validateFirstSessionPage(document, liveTools) {
  const source = document.content.replaceAll('\r\n', '\n')
  const openingLines = [...source.matchAll(
    /^(?:<!-- doc-contract:first-session-prompt tool=read_library -->|\{\/\* <!-- doc-contract:first-session-prompt tool=read_library --> \*\/\})$/gm,
  )]
  const closingLines = [...source.matchAll(
    /^(?:<!-- \/doc-contract:first-session-prompt -->|\{\/\* <!-- \/doc-contract:first-session-prompt --> \*\/\})$/gm,
  )]
  const markerTokens = source.match(/\/?doc-contract:first-session-prompt/g)
    ?? []
  if (
    openingLines.length !== 1
    || closingLines.length !== 1
    || markerTokens.length !== 2
  ) {
    throw new Error(
      `${document.file}: first-session prompt needs exactly one well-formed marker pair`,
    )
  }

  const openingIndex = openingLines[0].index
  const bodyStart = openingIndex + openingLines[0][0].length
  const closingIndex = closingLines[0].index
  if (openingIndex < 0 || closingIndex < bodyStart) {
    throw new Error(
      `${document.file}: first-session prompt markers are unmatched`,
    )
  }

  const fences = fencedBlocks(source)
  if (fences.length !== 1) {
    throw new Error(
      `${document.file}: first-session page must contain exactly one runnable fence; found ${fences.length}`,
    )
  }
  const [fence] = fences
  const fenceEnd = fence.index + fence.full.length
  if (fence.index < bodyStart || fenceEnd > closingIndex) {
    throw new Error(
      `${document.file}: the sole runnable fence must be inside the first-session marker`,
    )
  }
  if (fence.info !== 'text') {
    throw new Error(`${document.file}: first-session prompt fence must be text`)
  }
  const markerBody = source.slice(bodyStart, closingIndex)
  const relativeFence = fence.index - bodyStart
  if (
    `${markerBody.slice(0, relativeFence)}${
      markerBody.slice(relativeFence + fence.full.length)
    }`.trim() !== ''
  ) {
    throw new Error(
      `${document.file}: first-session marker may contain only its text fence`,
    )
  }

  const prompt = fence.body.replace(/\n$/, '')
  for (const [label, pattern] of FIRST_SESSION_BOUNDARIES) {
    if (!pattern.test(prompt)) {
      throw new Error(
        `${document.file}: first-session prompt is missing the ${label} boundary`,
      )
    }
  }
  if (prompt !== FIRST_SESSION_PROMPT) {
    throw new Error(
      `${document.file}: first-session prompt must match the canonical text exactly`,
    )
  }

  const readLibrary = liveTools.find((tool) => tool.name === 'read_library')
  if (!readLibrary) {
    throw new Error(`${document.file}: live tools do not include read_library`)
  }
  let inputSurface
  try {
    inputSurface = schemaProperties(readLibrary.inputSchema ?? {}, '/')
  } catch (error) {
    throw new Error(
      `${document.file}: read_library input schema composition is unsupported: ${error.message}`,
    )
  }
  if (
    Object.keys(inputSurface.properties).length !== 0
    || inputSurface.required.size !== 0
  ) {
    throw new Error(`${document.file}: read_library must remain parameter-free`)
  }
  for (const tool of liveTools) {
    if (tool.name === 'read_library') continue
    const toolPattern = new RegExp(
      `(^|[^a-zA-Z0-9_])${tool.name.replaceAll('_', '\\_')}(?=$|[^a-zA-Z0-9_])`,
    )
    if (toolPattern.test(prompt)) {
      throw new Error(
        `${document.file}: first-session prompt names a second live tool: ${tool.name}`,
      )
    }
  }
}

function sourceAssertion(condition, label) {
  if (!condition) throw new Error(`onboarding source contract: ${label}`)
}

/** Validate source-level onboarding navigation and composition. */
export function validateOnboardingSources({
  homepage,
  install,
  firstSession,
  goalChooser,
  astroConfig,
  cargoToml,
  builtPaths,
}) {
  sourceAssertion(
    homepage.includes("link: '/getting-started/'"),
    'homepage primary action must link to Install',
  )
  sourceAssertion(
    homepage.includes('/getting-started/first-session/'),
    'homepage must link to First 10 minutes',
  )
  sourceAssertion(
    install.includes('/getting-started/first-session/'),
    'Install must link forward to First 10 minutes',
  )
  sourceAssertion(
    firstSession.includes('<GoalChooser />'),
    'First 10 minutes must render GoalChooser',
  )
  sourceAssertion(
    homepage.includes('/workflows/'),
    'homepage must keep a compact all-workflows link',
  )
  sourceAssertion(
    !/Start here: Library Cleanup|recommended starting point for every new user/
      .test(
        `${homepage}\n${install}`,
      ),
    'universal Library Cleanup onboarding language must be absent',
  )
  sourceAssertion(
    !/\/workflows\/[a-z0-9-]+\//.test(homepage),
    'homepage must not link directly to an individual workflow',
  )
  sourceAssertion(
    !homepage.includes('GoalChooser'),
    'homepage must not render GoalChooser',
  )
  sourceAssertion(
    !install.includes(FIRST_SESSION_PROMPT)
      && !fencedBlocks(install).some((fence) =>
        /(^|[^a-zA-Z0-9_])read_library(?=$|[^a-zA-Z0-9_])/.test(fence.body)
      ),
    'Install must not duplicate the first-session prompt',
  )

  const chooserRequirements = [
    ['goal definitions', /goalDefinitions/],
    ['workflow records', /workflows/],
    ['canonical goal membership', /workflow\.goals\.includes\(goal\.id\)/],
    ['collection impact', /libraryImpact/],
    ['direct user files', /directUserFiles/],
    ['local state', /localStateWrites/],
    ['outputs', /outputs/],
    ['prerequisites', /prerequisites/],
    ['all workflows link', /href="\/workflows\/"/],
    ['reference overview link', /href="\/reference\/"/],
  ]
  chooserRequirements.forEach(([label, pattern]) => {
    sourceAssertion(
      pattern.test(goalChooser),
      `GoalChooser is missing ${label}`,
    )
  })
  sourceAssertion(
    !/client:(?:load|only)/.test(goalChooser),
    'GoalChooser must render without client hydration',
  )

  const sidebarEntries = [
    /\{\s*slug:\s*'getting-started',\s*label:\s*'Install',?\s*\}/,
    /\{\s*slug:\s*'getting-started\/first-session',\s*label:\s*'First 10 minutes',?\s*\}/,
    /\{\s*slug:\s*'workflows',\s*label:\s*'Choose a workflow',?\s*\}/,
  ].map((pattern) => astroConfig.search(pattern))
  sourceAssertion(
    sidebarEntries.every((index) => index >= 0)
      && sidebarEntries[0] < sidebarEntries[1]
      && sidebarEntries[1] < sidebarEntries[2],
    'sidebar order must be Install, First 10 minutes, Choose a workflow',
  )

  const crateVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
  sourceAssertion(
    Boolean(crateVersion),
    'Cargo.toml package version is missing',
  )
  const sentinels = [...homepage.matchAll(/v(\d+\.\d+\.\d+)\s+—/g)]
  sourceAssertion(
    sentinels.length === 1,
    'homepage must contain exactly one version sentinel',
  )
  sourceAssertion(
    sentinels[0]?.[1] === crateVersion,
    `homepage version sentinel must match Cargo.toml ${crateVersion}`,
  )
  if (builtPaths) {
    sourceAssertion(
      builtPaths.has('getting-started/first-session/index.html'),
      'built First 10 minutes route is missing',
    )
  }
}

const REQUIRED_CI_DOC_PATHS = [
  'site/**',
  'src/tools/**',
  'src/types.rs',
  'src/tags.rs',
  'src/cli/**',
  'src/main.rs',
  'Cargo.toml',
  'Cargo.lock',
  'scripts/mcp-smoke.mjs',
  'scripts/lib/mcp-stdio.mjs',
  'scripts/check-doc-contract.mjs',
  'scripts/check-doc-contract.test.mjs',
  'scripts/release.sh',
  'docs/workflows/doc-drift/**',
  'README.md',
  '.github/workflows/docs-pages.yml',
]

const REQUIRED_RELEASE_DOC_PATHS = [
  'site',
  'src/tools',
  'src/types.rs',
  'src/tags.rs',
  'src/cli',
  'src/main.rs',
  'Cargo.toml',
  'Cargo.lock',
  'scripts/mcp-smoke.mjs',
  'scripts/lib/mcp-stdio.mjs',
  'scripts/check-doc-contract.mjs',
  'scripts/check-doc-contract.test.mjs',
  'scripts/release.sh',
  '.github/workflows/docs-pages.yml',
  'docs/workflows/doc-drift',
  'site/src/content/docs/mcp-tools',
  'site/src/content/docs/cli',
  'site/src/partials/sops',
  'site/src/data/workflows.mjs',
  'site/src/data/tool-reference.mjs',
  'site/astro.config.mjs',
  'README.md',
]

function unquotePath(value) {
  const trimmed = value.trim().replace(/\\\s*$/, '').trim()
  if (
    trimmed.length >= 2
    && trimmed[0] === trimmed.at(-1)
    && ['"', "'"].includes(trimmed[0])
  ) {
    return trimmed.slice(1, -1)
  }
  return trimmed
}

function parseWorkflowEventPaths(source, event) {
  const lines = source.replaceAll('\r\n', '\n').split('\n')
  const eventIndex = lines.findIndex((line) => line === `  ${event}:`)
  if (eventIndex < 0) throw new Error(`docs workflow is missing on.${event}`)
  let pathsIndex = -1
  for (let index = eventIndex + 1; index < lines.length; index += 1) {
    if (/^  \S/.test(lines[index])) break
    if (lines[index] === '    paths:') {
      pathsIndex = index
      break
    }
  }
  if (pathsIndex < 0) {
    throw new Error(`docs workflow is missing on.${event}.paths`)
  }
  const paths = []
  for (let index = pathsIndex + 1; index < lines.length; index += 1) {
    const match = lines[index].match(/^      -\s+(.+)$/)
    if (!match) break
    paths.push(unquotePath(match[1]))
  }
  return paths
}

function parseReleaseDocsContractPaths(source) {
  const start = source.indexOf('docs_contract_changed() {')
  if (start < 0) {
    throw new Error('release script is missing docs_contract_changed()')
  }
  const end = source.indexOf('\n}', start)
  if (end < 0) throw new Error('release docs_contract_changed() is unmatched')
  const lines = source.slice(start, end).split('\n')
  const command = lines.findIndex((line) =>
    line.trim() === 'changed_since_base \\'
  )
  if (command < 0) {
    throw new Error('docs_contract_changed() must call changed_since_base')
  }
  return lines
    .slice(command + 1)
    .map(unquotePath)
    .filter(Boolean)
}

/** Parse the three docs-gate path inventories without reading the filesystem. */
export function parseDocsGatePathInventories(workflowSource, releaseSource) {
  return {
    push: parseWorkflowEventPaths(workflowSource, 'push'),
    pullRequest: parseWorkflowEventPaths(workflowSource, 'pull_request'),
    release: parseReleaseDocsContractPaths(releaseSource),
  }
}

/** Require both manifest triggers and every pre-existing docs-contract path. */
export function validateDocsGatePathInventories(inventories) {
  const expectations = [
    ['push', inventories.push, REQUIRED_CI_DOC_PATHS],
    ['pull_request', inventories.pullRequest, REQUIRED_CI_DOC_PATHS],
    ['release', inventories.release, REQUIRED_RELEASE_DOC_PATHS],
  ]
  for (const [label, actual, required] of expectations) {
    const missing = required.filter((requiredPath) =>
      !actual.includes(requiredPath)
    )
    if (missing.length) {
      throw new Error(
        `docs ${label} trigger is missing required paths: ${
          missing.join(', ')
        }`,
      )
    }
  }
}

export async function readDocuments(root, relativeFiles) {
  return Promise.all(
    relativeFiles.map(async (file) => ({
      file,
      content: await fs.readFile(path.join(root, file), 'utf8'),
    })),
  )
}

export async function listFiles(root, predicate = () => true) {
  const files = []
  async function visit(directory) {
    for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name)
      if (entry.isDirectory()) await visit(absolute)
      else if (predicate(absolute)) files.push(path.relative(root, absolute))
    }
  }
  await visit(root)
  return files.sort()
}

export async function loadMcpInventory(
  bin,
  timeoutMs = 60_000,
  helpTopics = [],
) {
  const env = { ...process.env }
  delete env.RUST_LOG
  const client = new McpStdioClient({ bin, timeoutMs, env })
  try {
    const initialized = await client.request('initialize', {
      protocolVersion: '2025-03-26',
      capabilities: {},
      clientInfo: { name: 'reklawdbox-doc-contract', version: '0.1.0' },
    })
    ensureTransport(initialized, 'src/main.rs:1: initialize')
    client.notify('notifications/initialized')
    const toolList = await client.listTools()
    ensureTransport(toolList, 'src/tools/mod.rs:1: tools/list')
    const help = await client.callTool('help', {})
    ensureTransport(help, 'src/tools/help_handler.rs:1: tools/call help')
    if (help.isError) {
      throw new Error(
        'src/tools/help_handler.rs:1: DB-free help() returned a tool error',
      )
    }
    const topicHelp = new Map()
    for (const topic of helpTopics) {
      const result = await client.callTool('help', { topic })
      ensureTransport(
        result,
        `src/tools/help_handler.rs:1: tools/call help(${
          JSON.stringify(topic)
        })`,
      )
      if (result.isError) {
        throw new Error(
          `src/tools/help_handler.rs:1: DB-free help(${
            JSON.stringify(topic)
          }) returned a tool error`,
        )
      }
      topicHelp.set(topic, result)
    }
    if (client.protocolViolations.length) {
      throw new Error(
        `src/main.rs:1: server wrote non-JSON stdout: ${
          client.protocolViolations[0]
        }`,
      )
    }
    return { toolList, help, topicHelp }
  } finally {
    await client.close()
  }
}

function ensureTransport(result, label) {
  if (result?.transportError) {
    throw new Error(`${label} failed: ${JSON.stringify(result.transportError)}`)
  }
  if (result?.timeout) throw new Error(`${label} timed out: ${result.stderr}`)
  if (result?.childExit !== undefined) {
    throw new Error(`${label} server exit: ${result.childExit}`)
  }
}

function parseToolJson(result, label) {
  const text = result?.content?.find((content) => content.type === 'text')?.text
    ?? ''
  try {
    return JSON.parse(text)
  } catch (error) {
    throw new Error(`${label} returned non-JSON text: ${error.message}`)
  }
}

async function main() {
  const options = parseMainArgs(process.argv.slice(2))
  const root = process.cwd()
  const issues = []
  const check = (operation) => {
    try {
      operation()
    } catch (error) {
      issues.push(error instanceof Error ? error.message : String(error))
    }
  }
  const { toolReferences } = await import(
    pathToFileURL(path.join(root, 'site/src/data/tool-reference.mjs'))
  )
  const {
    goalDefinitions,
    workflows,
    validateWorkflows,
    XML_BACKUP_SUCCESS_CONDITION,
  } = await import(
    pathToFileURL(path.join(root, 'site/src/data/workflows.mjs'))
  )
  check(() => {
    try {
      validateWorkflows(workflows, goalDefinitions)
    } catch (error) {
      throw new Error(`site/src/data/workflows.mjs:1: ${error.message}`)
    }
  })
  check(() =>
    validateXmlBackupContracts(workflows, XML_BACKUP_SUCCESS_CONDITION)
  )

  const [
    homepageDocument,
    installDocument,
    firstSessionDocument,
    goalChooserDocument,
    astroConfigDocument,
  ] = await readDocuments(root, [
    'site/src/content/docs/index.mdx',
    'site/src/content/docs/getting-started/index.mdx',
    'site/src/content/docs/getting-started/first-session.mdx',
    'site/src/components/GoalChooser.astro',
    'site/astro.config.mjs',
  ])
  const cargoToml = await fs.readFile(path.join(root, 'Cargo.toml'), 'utf8')
  const docsWorkflow = await fs.readFile(
    path.join(root, '.github/workflows/docs-pages.yml'),
    'utf8',
  )
  const releaseScript = await fs.readFile(
    path.join(root, 'scripts/release.sh'),
    'utf8',
  )
  check(() =>
    validateDocsGatePathInventories(
      parseDocsGatePathInventories(docsWorkflow, releaseScript),
    )
  )

  const helpTopics = runtimeHelpTopics(workflows)
  const { toolList, help, topicHelp } = await loadMcpInventory(
    options.bin,
    options.timeoutMs,
    helpTopics,
  )
  const liveTools = toolList.tools ?? []
  check(() => compareToolMappings(liveTools, toolReferences))
  check(() => validateFirstSessionPage(firstSessionDocument, liveTools))

  const mcpFiles = (await listFiles(
    path.join(root, 'site/src/content/docs/mcp-tools'),
    (file) => file.endsWith('.mdx'),
  )).map((file) => path.join('site/src/content/docs/mcp-tools', file))
  const mcpDocuments = await readDocuments(root, mcpFiles)
  check(() => validateMcpContracts(mcpDocuments, liveTools, toolReferences))
  check(() =>
    validateMcpOutputContracts(mcpDocuments, liveTools, toolReferences)
  )

  const sopFiles = (await listFiles(
    path.join(root, 'site/src/partials/sops'),
    (file) => file.endsWith('.mdx'),
  )).map((file) => path.join('site/src/partials/sops', file))
  const sopDocuments = await readDocuments(root, sopFiles)
  check(() => validateSopContracts(sopDocuments, liveTools))

  const cliDocument = await readDocuments(root, [
    'site/src/content/docs/cli/index.mdx',
  ])
  check(() => validateCliContracts(cliDocument, readCliInventory(options.bin)))

  const helpPayload = parseToolJson(help, 'src/tools/help_handler.rs:1: help()')
  check(() => compareRuntimeHelp(workflows, helpPayload))
  const runtimeWorkflows = workflows
    .filter((workflow) => workflow.runtimeHelp)
    .sort((left, right) =>
      left.runtimeHelp.menuOrder - right.runtimeHelp.menuOrder
    )
  const topicPayloads = runtimeWorkflows.map((workflow) => {
    const topic = workflow.runtimeHelp.topic
    const payload = parseToolJson(
      topicHelp.get(topic),
      `src/tools/help_handler.rs:1: help(${JSON.stringify(topic)})`,
    )
    if (payload.workflow !== workflow.title) {
      issues.push(
        `src/tools/help_handler.rs:1: help(${
          JSON.stringify(topic)
        }) returned workflow ${JSON.stringify(payload.workflow)}, expected ${
          JSON.stringify(workflow.title)
        }`,
      )
    }
    return {
      source: `src/tools/help_handler.rs:1: help(${JSON.stringify(topic)})`,
      payload,
    }
  })

  const distRoot = path.resolve(root, options.dist)
  const distFiles = await listFiles(distRoot)
  const builtPaths = new Set(distFiles)
  check(() =>
    validateOnboardingSources({
      homepage: homepageDocument.content,
      install: installDocument.content,
      firstSession: firstSessionDocument.content,
      goalChooser: goalChooserDocument.content,
      astroConfig: astroConfigDocument.content,
      cargoToml,
      builtPaths,
    })
  )
  for (const workflow of workflows) {
    const route = workflow.route.replace(/^\//, '')
    if (!builtPaths.has(path.join(route, 'index.html'))) {
      issues.push(
        `site/src/data/workflows.mjs:1: canonical workflow route is not built: ${workflow.route}`,
      )
    }
  }
  const htmlFiles = distFiles.filter((file) => file.endsWith('.html'))
  const htmlDocuments = await Promise.all(
    htmlFiles.map(async (file) => ({
      file: path.join(options.dist, file),
      builtPath: file,
      content: await fs.readFile(path.join(distRoot, file), 'utf8'),
    })),
  )
  check(() => validateBuiltLinkSet(htmlDocuments, builtPaths))

  check(() =>
    validateRuntimeHelpUrls(
      [
        { source: 'src/tools/help_handler.rs:1: help()', payload: helpPayload },
        ...topicPayloads,
      ],
      builtPaths,
    )
  )

  if (issues.length) throw new Error([...new Set(issues)].sort().join('\n'))

  console.log(
    `Documentation contracts pass: ${liveTools.length} MCP tools, ${workflows.length} workflows, ${htmlFiles.length} HTML files.`,
  )
}

function parseMainArgs(args) {
  const options = {
    bin: './target/release/reklawdbox',
    dist: './site/dist',
    timeoutMs: 60_000,
  }
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === '--bin') options.bin = args[++index]
    else if (args[index] === '--dist') options.dist = args[++index]
    else if (args[index] === '--timeout-ms') {
      options.timeoutMs = Number(args[++index])
    } else if (args[index] === '--help' || args[index] === '-h') {
      console.log(
        'Usage: node scripts/check-doc-contract.mjs [--bin PATH] [--dist PATH] [--timeout-ms MS]',
      )
      process.exit(0)
    } else throw new Error(`unknown argument: ${args[index]}`)
  }
  return options
}

const invokedPath = process.argv[1]
  ? pathToFileURL(path.resolve(process.argv[1])).href
  : null
if (invokedPath === import.meta.url) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  })
}
