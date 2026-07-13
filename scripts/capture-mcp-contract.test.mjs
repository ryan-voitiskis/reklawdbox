import assert from 'node:assert/strict'
import test from 'node:test'

import {
  canonicalContract,
  canonicalize,
  validateToolList,
} from './capture-mcp-contract.mjs'

const initialize = {
  serverInfo: { version: '1.0.0', name: 'fixture' },
  protocolVersion: '2025-03-26',
  capabilities: { tools: { listChanged: false }, future: { retained: true } },
  instructions: 'Complete instructions',
  unknownInitializeField: { z: 2, a: 1 },
}

const tools = [
  {
    name: 'zeta',
    description: 'Zeta description',
    inputSchema: {
      required: ['nested'],
      properties: {
        nested: {
          type: 'object',
          properties: { z: { type: 'string' }, a: { type: 'number' } },
        },
      },
      type: 'object',
    },
    outputSchema: {
      type: 'object',
      properties: { result: { type: 'array', items: { type: 'string' } } },
    },
    annotations: { readOnlyHint: true, unknownHint: 'retained' },
    futureToolField: { kept: ['z', 'a'] },
  },
  {
    inputSchema: { type: 'object', properties: {} },
    description: 'Alpha description',
    name: 'alpha',
    unknown: { beta: 2, alpha: 1 },
  },
]

test('canonicalize recursively sorts object keys without mutating arrays or input', () => {
  const source = { z: { y: 2, a: 1 }, a: [{ z: 3, a: 2 }, 'first'] }
  const before = structuredClone(source)
  assert.deepEqual(canonicalize(source), {
    a: [{ a: 2, z: 3 }, 'first'],
    z: { a: 1, y: 2 },
  })
  assert.deepEqual(source, before)
})

test('canonical contract is byte-stable, tool-sorted, and lossless', () => {
  const first = canonicalContract(initialize, tools)
  const reordered = canonicalContract(
    { ...initialize, serverInfo: { name: 'fixture', version: '1.0.0' } },
    [...tools].reverse(),
  )
  assert.equal(JSON.stringify(first), JSON.stringify(reordered))
  assert.deepEqual(first.tools.map((tool) => tool.name), ['alpha', 'zeta'])
  const zeta = first.tools[1]
  assert.equal(zeta.description, 'Zeta description')
  assert.equal(zeta.annotations.unknownHint, 'retained')
  assert.deepEqual(zeta.futureToolField, { kept: ['z', 'a'] })
  assert.equal(zeta.inputSchema.properties.nested.properties.z.type, 'string')
  assert.equal(zeta.outputSchema.properties.result.items.type, 'string')
  assert.equal(first.initialize.unknownInitializeField.z, 2)
})

test('tool-list validation fails closed on incomplete shapes and pagination', () => {
  assert.throws(
    () => validateToolList({ tools: [{ name: 'x', inputSchema: {} }] }),
    /string description/,
  )
  assert.throws(
    () => validateToolList({ tools, nextCursor: 'page-2' }),
    /pagination is unsupported/,
  )
  assert.doesNotThrow(() => validateToolList({ tools, nextCursor: null }))
})
