import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import {
  goalDefinitions,
  validateWorkflows,
  workflows,
  XML_BACKUP_SUCCESS_CONDITION,
} from '../site/src/data/workflows.mjs'
import {
  compareRuntimeHelp,
  compareToolMappings,
  deriveAgentPairs,
  extractSopCalls,
  FIRST_SESSION_PROMPT,
  parseApplicationCliHelp,
  parseDocsGatePathInventories,
  parseRootCliHelp,
  parseRootCliOptions,
  runtimeHelpTopics,
  validateBatchAudioExtensionsContract,
  validateBuiltLinkSet,
  validateCliContracts,
  validateColorPaletteContract,
  validateDocsGatePathInventories,
  validateFirstSessionPage,
  validateMcpContracts,
  validateMcpOutputContracts,
  validateOnboardingSources,
  validatePublishingAudiences,
  validateRuntimeHelpUrls,
  validateSopContracts,
  validateXmlBackupContracts,
} from './check-doc-contract.mjs'
import { decodeProtocolLine, routeProtocolLine } from './lib/mcp-stdio.mjs'

function tool(name, properties = {}, required = []) {
  return {
    name,
    inputSchema: { type: 'object', properties, required },
  }
}

function document(content, file = 'fixture.mdx') {
  return { file, content }
}

function table(
  rows,
  headers = ['Parameter', 'Type', 'Required', 'Description'],
) {
  return [
    `| ${headers.join(' | ')} |`,
    `| ${headers.map(() => '---').join(' | ')} |`,
    ...rows.map((row) => `| ${row.join(' | ')} |`),
  ].join('\n')
}

function mcpMarker(name, body, attributes = '') {
  return `<!-- doc-contract:mcp tool=${name} schema=/ requiredness=global${
    attributes ? ` ${attributes}` : ''
  } -->\n${body}\n<!-- /doc-contract:mcp -->`
}

function outputTool(name, properties = {}, required = []) {
  return {
    name,
    inputSchema: { type: 'object', properties: {} },
    outputSchema: { type: 'object', properties, required },
  }
}

function mcpOutputMarker(name, body, schema = '/') {
  return `<!-- doc-contract:mcp-output tool=${name} schema=${schema} requiredness=global -->\n${body}\n<!-- /doc-contract:mcp-output -->`
}

function publishingAudienceFixture() {
  const pairs = deriveAgentPairs(workflows)
  const sourceArtifacts = new Map()
  const builtArtifacts = new Map()
  const fullHeadings = []
  const smallHeadings = []
  const agentHeadings = []
  const sitemapPaths = [
    '/workflows/',
    '/mcp-tools/',
    '/getting-started/',
  ]

  for (const workflow of workflows) {
    sourceArtifacts.set(
      `site/src/content/docs${workflow.route.replace(/\/$/, '')}.mdx`,
      '# Human explanation\n',
    )
  }

  sourceArtifacts.set(
    'site/astro.config.mjs',
    `import sitemap from '@astrojs/sitemap'
integrations: [
  sitemap({ filter: (page) => !new URL(page).pathname.startsWith('/agent/') }),
]
starlightLlmsTxt({
  exclude: ['agent/**'],
  excludeFull: ['agent/**'],
  customSets: [{ paths: ['agent/**'] }],
})`,
  )
  sourceArtifacts.set(
    'site/vendor/starlight-llms-txt/llms-full.txt.ts',
    'exclude: starlightLllmsTxtContext.excludeFull',
  )
  sourceArtifacts.set(
    'site/vendor/starlight-llms-txt/llms-small.txt.ts',
    'exclude: starlightLllmsTxtContext.exclude',
  )
  sourceArtifacts.set(
    'site/vendor/starlight-llms-txt/llms-custom.txt.ts',
    'include: context.props.paths',
  )

  builtArtifacts.set(
    'agent/index.html',
    '<html><head><meta name="robots" content="noindex, nofollow"></head><h1>Agent SOPs</h1></html>',
  )
  for (const pair of pairs) {
    const humanHeading = `# ${pair.title}`
    const agentHeading = `# Agent SOP: ${pair.title}`
    sourceArtifacts.set(pair.humanSource, '# Human explanation\n')
    sourceArtifacts.set(
      pair.agentSource,
      `import SOP from '../../../partials/sops/${pair.id}.mdx'\n\n<SOP />\n`,
    )
    builtArtifacts.set(
      pair.humanHtml,
      `<html><main data-pagefind-body><h1>${pair.title}</h1></main></html>`,
    )
    builtArtifacts.set(
      pair.agentHtml,
      `<html><head><meta content="noindex, nofollow" name="robots"></head><h1>Agent SOP: ${pair.title}</h1></html>`,
    )
    builtArtifacts.set(pair.sopText, `${agentHeading}\n`)
    fullHeadings.push(humanHeading)
    smallHeadings.push(humanHeading)
    agentHeadings.push(agentHeading)
    sitemapPaths.push(pair.humanRoute)
  }
  builtArtifacts.set('llms-full.txt', `${fullHeadings.join('\n')}\n`)
  builtArtifacts.set('llms-small.txt', `${smallHeadings.join('\n')}\n`)
  builtArtifacts.set(
    '_llms-txt/agent-sops.txt',
    `# Agent SOPs\n${agentHeadings.join('\n')}\n`,
  )
  builtArtifacts.set(
    'llms.txt',
    [
      '/_llms-txt/agent-sops.txt',
      ...pairs.map((pair) => `/${pair.sopText}`),
    ].join('\n'),
  )
  builtArtifacts.set(
    'sitemap-0.xml',
    `<urlset>${
      sitemapPaths.map((route) =>
        `<url><loc>https://reklawdbox.com${route}</loc></url>`
      ).join('')
    }</urlset>`,
  )
  return { workflows, sourceArtifacts, builtArtifacts }
}

function expectAudienceFailure(mutate, expected) {
  const fixture = publishingAudienceFixture()
  mutate(fixture, deriveAgentPairs(workflows))
  assert.throws(() => validatePublishingAudiences(fixture), expected)
}

function selectedOutputFixture(enrichRows) {
  const selected = [
    ['analyze_audio_batch', [['page', 'object', 'yes', 'Continuation']]],
    ['backfill_labels', [['conflict_page', 'object', 'yes', 'Continuation']]],
    ['enrich_tracks', enrichRows],
    ['scan_duplicates', [['page', 'object', 'yes', 'Continuation']]],
  ]
  const tools = selected.map(([name, rows]) =>
    outputTool(
      name,
      Object.fromEntries(rows.map(([field, type]) => [field, { type }])),
      rows.map(([field]) => field),
    )
  )
  const docs = [
    document(
      selected.map(([name, rows]) =>
        mcpOutputMarker(
          name,
          table(rows, ['Field', 'Type', 'Required', 'Description']),
        )
      ).join('\n'),
    ),
  ]
  return { tools, docs }
}

test('selected MCP output contracts compare marked fields with live outputSchema', () => {
  const fixture = selectedOutputFixture([
    ['summary', 'object', 'yes', 'Batch summary'],
    ['page', 'object', 'yes', 'Continuation'],
  ])
  assert.doesNotThrow(() =>
    validateMcpOutputContracts(fixture.docs, fixture.tools)
  )
})

test('required nullable output fields retain null while optional fields do not', () => {
  const fixture = selectedOutputFixture([
    ['summary', 'object', 'yes', 'Batch summary'],
    ['page', 'object', 'yes', 'Continuation'],
    ['hint', 'string', '', 'Optional hint'],
  ])
  const enrich = fixture.tools.find((item) => item.name === 'enrich_tracks')
  enrich.outputSchema.required = ['summary', 'page']
  enrich.outputSchema.properties.hint = { type: ['string', 'null'] }
  enrich.outputSchema.properties.page = {
    type: 'object',
    properties: {
      next_offset: { type: ['integer', 'null'] },
    },
    required: ['next_offset'],
  }
  fixture.docs[0].content += '\n'
    + mcpOutputMarker(
      'enrich_tracks',
      table([
        ['`next_offset`', 'integer \\| null', '**yes**', 'Next cursor'],
      ], ['Field', 'Type', 'Required', 'Description']),
      '/properties/page',
    )

  assert.doesNotThrow(() =>
    validateMcpOutputContracts(fixture.docs, fixture.tools)
  )

  const wrongDocs = fixture.docs.map((item) => ({
    ...item,
    content: item.content.replace('integer \\| null', 'integer'),
  }))
  assert.throws(
    () => validateMcpOutputContracts(wrongDocs, fixture.tools),
    /enrich_tracks\.next_offset output type is integer, live schema is integer\|null/,
  )
})

test('MCP output contracts reject an omitted live response property', () => {
  const fixture = selectedOutputFixture([
    ['summary', 'object', 'yes', 'Batch summary'],
  ])
  fixture.tools.find((item) => item.name === 'enrich_tracks')
    .outputSchema.properties.page = { type: 'object' }
  fixture.tools.find((item) => item.name === 'enrich_tracks')
    .outputSchema.required.push('page')
  assert.throws(
    () => validateMcpOutputContracts(fixture.docs, fixture.tools),
    /enrich_tracks\.page is missing from marked output schema/,
  )
})

test('stdio decoder routes exact result payloads and records violations', async () => {
  assert.deepEqual(decodeProtocolLine('  '), { kind: 'blank' })
  assert.equal(decodeProtocolLine('not-json').kind, 'violation')

  const pending = new Map()
  const violations = []
  const payload = {
    tools: [{ name: 'read_library', inputSchema: { properties: {} } }],
  }
  let resolved
  pending.set(7, {
    resolve(value) {
      resolved = value
    },
    timer: setTimeout(() => {}, 10_000),
  })
  routeProtocolLine(
    JSON.stringify({ jsonrpc: '2.0', id: 7, result: payload }),
    pending,
    violations,
  )
  assert.deepEqual(resolved, payload)
  assert.equal(pending.size, 0)
  routeProtocolLine('stdout noise', pending, violations)
  assert.deepEqual(violations, ['stdout noise'])
})

test('tool mapping rejects missing and extra names', () => {
  assert.throws(
    () =>
      compareToolMappings([tool('alpha'), tool('beta')], [{ name: 'alpha' }, {
        name: 'extra',
      }]),
    /missing: beta; extra: extra/,
  )
})

test('MCP contracts reject a mapped tool with no marker', () => {
  assert.throws(
    () =>
      validateMcpContracts([], [tool('alpha')], [{
        name: 'alpha',
        route: '/mcp-tools/library-data/',
      }]),
    /library-data\.mdx:1: MCP tool alpha has no marked contract surface/,
  )
})

test('MCP markers require explicit schema and requiredness attributes', () => {
  const live = [tool('alpha')]
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          '<!-- doc-contract:mcp tool=alpha requiredness=global surface=none -->',
        ),
      ], live),
    /mcp marker needs explicit schema=/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          '<!-- doc-contract:mcp tool=alpha schema=/ surface=none -->',
        ),
      ], live),
    /mcp marker needs explicit requiredness=/,
  )
  const missingSurfaceSchema = `
<!-- doc-contract:mcp-surface name=shared requiredness=global -->
${table([])}
<!-- /doc-contract:mcp-surface -->
${mcpMarker('alpha', table([]), 'include=shared')}
`
  assert.throws(
    () => validateMcpContracts([document(missingSurfaceSchema)], live),
    /mcp-surface marker needs explicit schema=/,
  )
  const missingSurfaceRequiredness = `
<!-- doc-contract:mcp-surface name=shared schema=/ -->
${table([])}
<!-- /doc-contract:mcp-surface -->
${mcpMarker('alpha', table([]), 'include=shared')}
`
  assert.throws(
    () => validateMcpContracts([document(missingSurfaceRequiredness)], live),
    /mcp-surface marker needs explicit requiredness=/,
  )
})

test('MCP tools require a root surface and object-resolving schema pointers', () => {
  const live = [
    tool('nested', {
      changes: {
        type: 'array',
        items: {
          type: 'object',
          properties: { id: { type: 'string' } },
          required: ['id'],
        },
      },
    }, ['changes']),
  ]
  const nestedOnly = `
<!-- doc-contract:mcp tool=nested schema=/properties/changes/items requiredness=global -->
${table([['\`id\`', 'string', '**yes**', 'ID']])}
<!-- /doc-contract:mcp -->
`
  assert.throws(
    () => validateMcpContracts([document(nestedOnly, 'nested.mdx')], live),
    /nested\.mdx:2: MCP tool nested has no root schema=\/ contract surface/,
  )

  const root = mcpMarker(
    'nested',
    table([['`changes`', 'array', '**yes**', 'Changes']]),
  )
  const typoedEmpty =
    '<!-- doc-contract:mcp tool=nested schema=/properties/changes/itmes surface=none requiredness=global -->'
  assert.throws(
    () =>
      validateMcpContracts(
        [document(`${root}\n${typoedEmpty}`, 'nested.mdx')],
        live,
      ),
    /nested\.mdx:\d+: nested schema path \/properties\/changes\/itmes does not resolve to an object schema/,
  )
  const scalarEmpty =
    '<!-- doc-contract:mcp tool=nested schema=/properties/changes/items/properties/id surface=none requiredness=global -->'
  assert.throws(
    () =>
      validateMcpContracts(
        [document(`${root}\n${scalarEmpty}`, 'nested.mdx')],
        live,
      ),
    /does not resolve to an object schema/,
  )
})

test('MCP tool contracts stay on their canonical reference page', () => {
  const content =
    '<!-- doc-contract:mcp tool=alpha schema=/ surface=none requiredness=global -->'
  assert.throws(
    () =>
      validateMcpContracts(
        [document(content, 'site/src/content/docs/mcp-tools/mixing.mdx')],
        [tool('alpha')],
        [{ name: 'alpha', route: '/mcp-tools/library-data/' }],
      ),
    /mixing\.mdx:1: alpha contract belongs in .*library-data\.mdx/,
  )
})

test('MCP contracts reject unknown docs and live omissions', () => {
  const live = [tool('alpha', { actual: { type: 'string' } })]
  const unknown = document(
    mcpMarker('alpha', table([['`wrong`', 'string', '', 'Wrong field']])),
  )
  assert.throws(
    () => validateMcpContracts([unknown], live),
    /alpha\.wrong is not in live schema/,
  )

  const omitted = document(mcpMarker('alpha', table([])))
  assert.throws(
    () => validateMcpContracts([omitted], live),
    /fixture\.mdx:1: alpha\.actual is missing/,
  )
})

test('non-empty contract markers bracket exactly one Markdown table', () => {
  const firstMcpTable = table([
    ['`actual`', 'string', '', 'Live field'],
  ])
  const secondMcpTable = table([
    ['`wrong`', 'string', '', 'Unknown field'],
  ])
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          mcpMarker('alpha', `${firstMcpTable}\n\n${secondMcpTable}`),
          'multiple-mcp-tables.mdx',
        ),
      ], [tool('alpha', { actual: { type: 'string' } })]),
    /multiple-mcp-tables\.mdx:1: marked surface must contain exactly one Markdown table; found 2/,
  )

  const root = `<!-- doc-contract:cli command=root surface=commands -->\n${
    table([['`alpha`', 'Alpha']], ['Command', 'Description'])
  }\n<!-- /doc-contract:cli -->\n<!-- doc-contract:cli command=root surface=none -->`
  const firstCliTable = table(
    [['`--jobs`', '`-j`', '`4`']],
    ['Flag', 'Short', 'Default'],
  )
  const secondCliTable = table(
    [['`--wrong`', '', '']],
    ['Flag', 'Short', 'Default'],
  )
  const alpha =
    `<!-- doc-contract:cli command=alpha surface=options -->\n${firstCliTable}\n\n${secondCliTable}\n<!-- /doc-contract:cli -->`
  const inventory = {
    commands: new Map([['alpha', {}]]),
    fields: new Map([
      ['root', new Map()],
      [
        'alpha',
        new Map([['--jobs', { name: '--jobs', short: '-j', default: '4' }]]),
      ],
    ]),
  }
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha}`, 'multiple-cli-tables.mdx'),
      ], inventory),
    /multiple-cli-tables\.mdx:\d+: marked surface must contain exactly one Markdown table; found 2/,
  )
})

test('MCP shared surfaces compose before completeness checks', () => {
  const live = [
    tool('search', {
      query: { type: ['string', 'null'] },
      artist: { type: ['string', 'null'] },
      limit: { type: ['integer', 'null'] },
    }),
  ]
  const content = `
<!-- doc-contract:mcp-surface name=search-base schema=/ requiredness=global -->
${table([['\`query\`', 'string', '', 'Query']])}
<!-- /doc-contract:mcp-surface -->
<!-- doc-contract:mcp-surface name=search-more schema=/ requiredness=global include=search-base -->
${table([['\`artist\`', 'string', '', 'Artist']])}
<!-- /doc-contract:mcp-surface -->
${
    mcpMarker(
      'search',
      table([['\`limit\`', 'integer', '', 'Limit']]),
      'include=search-more',
    )
  }
`
  assert.doesNotThrow(() => validateMcpContracts([document(content)], live))

  const missing = content.replace('| `artist` | string |  | Artist |', '')
  assert.throws(
    () => validateMcpContracts([document(missing)], live),
    /search\.artist is missing/,
  )
})

test('MCP shared surfaces reject unknown, cyclic, and conflicting composition', () => {
  const live = [tool('search', { query: { type: ['string', 'null'] } })]
  const local = table([['`query`', 'string', '', 'Query']])
  assert.throws(
    () =>
      validateMcpContracts([
        document(mcpMarker('search', local, 'include=missing')),
      ], live),
    /unknown MCP surface include: missing/,
  )

  const cyclic = `
<!-- doc-contract:mcp-surface name=a schema=/ requiredness=global include=b -->
${table([])}
<!-- /doc-contract:mcp-surface -->
<!-- doc-contract:mcp-surface name=b schema=/ requiredness=global include=a -->
${table([])}
<!-- /doc-contract:mcp-surface -->
${mcpMarker('search', local, 'include=a')}
`
  assert.throws(
    () => validateMcpContracts([document(cyclic)], live),
    /cyclic MCP surface include/,
  )

  const conflicting = `
<!-- doc-contract:mcp-surface name=shared schema=/ requiredness=global -->
${local}
<!-- /doc-contract:mcp-surface -->
${
    mcpMarker(
      'search',
      table([['\`query\`', 'integer', '', 'Query']]),
      'include=shared',
    )
  }
`
  assert.throws(
    () => validateMcpContracts([document(conflicting)], live),
    /conflicting duplicate field search\.query/,
  )
})

test('global requiredness is enforced while conditional requiredness is semantic', () => {
  const live = [tool('audit', { operation: { type: 'string' } }, ['operation'])]
  const body = table([['`operation`', 'string', '', 'Operation']])
  assert.throws(
    () => validateMcpContracts([document(mcpMarker('audit', body))], live),
    /requiredness disagrees/,
  )
  const conditional = mcpMarker('audit', body).replace(
    'requiredness=global',
    'requiredness=conditional',
  )
  assert.doesNotThrow(() => validateMcpContracts([document(conditional)], live))
})

test('MCP contracts detect primitive, array, object, enum, and default drift', () => {
  const live = [
    tool('shape', {
      text: { type: 'string' },
      items: { type: 'array', items: { type: 'string' } },
      config: { type: 'object' },
      mode: { type: 'string', enum: ['one', 'two'] },
      count: { type: 'integer', default: 3 },
      semantic: { type: 'string' },
    }),
  ]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Values',
    'Item values',
    'Default',
    'Description',
  ]
  const goodRows = [
    ['`text`', 'string', '', '', '', '', 'Text'],
    ['`items`', 'string[]', '', '', '', '', 'Items'],
    ['`config`', 'object', '', '', '', '', 'Config'],
    ['`mode`', 'string', '', '["one", "two"]', '', '', 'Mode'],
    [
      '`count`',
      'integer',
      '',
      '',
      '',
      '3',
      'The live literal 3 also appears here',
    ],
    [
      '`semantic`',
      'string',
      '',
      '',
      '',
      'median',
      'Handler-level prose default',
    ],
  ]
  assert.doesNotThrow(
    () =>
      validateMcpContracts(
        [document(mcpMarker('shape', table(goodRows, headers)))],
        live,
      ),
  )
  const reversedEnumRow = [
    ['`mode`', 'string', '', '["two", "one"]', '', '', 'Same enum set'],
  ]
  assert.doesNotThrow(() =>
    validateMcpContracts([
      document(
        `${mcpMarker('shape', table(goodRows, headers))}\n${
          mcpMarker('shape', table(reversedEnumRow, headers))
        }`,
      ),
    ], live)
  )
  for (
    const [needle, replacement, expected] of [
      ['| `text` | string |', '| `text` | integer |', /shape\.text type/],
      ['| `items` | string[] |', '| `items` | string |', /shape\.items type/],
      ['| `config` | object |', '| `config` | string |', /shape\.config type/],
      ['["one", "two"]', '["one"]', /enum differs; missing "two"/],
      ['["one", "two"]', '["one", "two", "three"]', /extra "three"/],
      [
        '| 3 | The live literal 3',
        '| 2 | The live literal 3',
        /default is 2, live schema is 3/,
      ],
      [
        '| 3 | The live literal 3',
        '| | The live literal 3',
        /omits live default 3 from a dedicated Default column/,
      ],
    ]
  ) {
    const bad = mcpMarker('shape', table(goodRows, headers)).replace(
      needle,
      replacement,
    )
    assert.throws(() => validateMcpContracts([document(bad)], live), expected)
  }
})

test('MCP array contracts compare direct, prefix-item, and nested item schemas', () => {
  const live = [
    tool('arrays', {
      direct: {
        type: ['array', 'null'],
        items: { type: 'string' },
      },
      pair: {
        type: 'array',
        maxItems: 2,
        prefixItems: [{ type: 'number' }, { type: 'number' }],
      },
      providers: {
        type: ['array', 'null'],
        items: {
          anyOf: [{ type: 'string', enum: ['one', 'two'] }],
        },
      },
      phases: {
        anyOf: [
          {
            type: 'array',
            items: {
              oneOf: [
                { type: 'string', const: 'warmup' },
                { type: 'string', const: 'peak' },
              ],
            },
          },
          { type: 'null' },
        ],
      },
      intersection: {
        allOf: [
          {
            type: 'array',
            items: { type: 'string', enum: ['a', 'b'] },
          },
          {
            type: 'array',
            items: { type: 'string', enum: ['b', 'c'] },
          },
        ],
      },
      open_alternative: {
        anyOf: [
          {
            type: 'array',
            items: { type: 'string', enum: ['open_a'] },
          },
          { type: 'array', items: { type: 'string' } },
        ],
      },
      prefix_intersection: {
        allOf: [
          {
            type: 'array',
            maxItems: 1,
            prefixItems: [{ type: 'string', enum: ['p1', 'p2'] }],
          },
          {
            type: 'array',
            maxItems: 1,
            prefixItems: [{ type: 'string', enum: ['p2', 'p3'] }],
          },
        ],
      },
      type_intersection: {
        allOf: [
          { type: 'array', items: { type: 'number' } },
          { type: 'array', items: { type: 'integer' } },
        ],
      },
      open_prefix: {
        type: 'array',
        prefixItems: [{ type: 'string', enum: ['head'] }],
      },
      selected_prefix: {
        type: 'array',
        maxItems: 1,
        prefixItems: [
          { type: 'string', enum: ['first'] },
          { type: 'integer' },
        ],
      },
      uniform_remainder: {
        type: 'array',
        prefixItems: [{ type: 'string', enum: ['uniform'] }],
        items: { type: 'string', enum: ['uniform'] },
      },
    }),
  ]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Item values',
    'Description',
  ]
  const goodRows = [
    ['`direct`', 'string[]', '', '', 'Direct'],
    ['`pair`', 'number[]', '', '', 'Pair'],
    ['`providers`', 'string[]', '', '["one", "two"]', 'Providers'],
    ['`phases`', 'string[]', '', '["warmup", "peak"]', 'Phases'],
    ['`intersection`', 'string[]', '', '["b"]', 'Intersection'],
    ['`open_alternative`', 'string[]', '', '', 'Open alternative'],
    ['`prefix_intersection`', 'string[]', '', '["p2"]', 'Prefix intersection'],
    ['`type_intersection`', 'integer[]', '', '', 'Type intersection'],
    ['`open_prefix`', 'array', '', '', 'Open prefix'],
    ['`selected_prefix`', 'string[]', '', '["first"]', 'Selected prefix'],
    ['`uniform_remainder`', 'string[]', '', '["uniform"]', 'Uniform remainder'],
  ]
  const good = mcpMarker('arrays', table(goodRows, headers))
  assert.doesNotThrow(() => validateMcpContracts([document(good)], live))
  for (
    const [bad, expected] of [
      [
        good.replace('`direct` | string[]', '`direct` | integer[]'),
        /direct array item type/,
      ],
      [
        good.replace('`pair` | number[]', '`pair` | string[]'),
        /pair array item type/,
      ],
      [
        good.replace('["one", "two"]', '["one"]'),
        /providers array item enum differs; missing "two"/,
      ],
      [
        good.replace('["warmup", "peak"]', '["warmup", "peak", "release"]'),
        /phases array item enum differs;.*extra "release"/,
      ],
      [
        good.replace(
          '`intersection` | string[] |  | ["b"] |',
          '`intersection` | string[] |  | ["a", "b", "c"] |',
        ),
        /intersection array item enum differs; missing none; extra "a", "c"/,
      ],
      [
        good.replace(
          '`intersection` | string[] |  | ["b"] |',
          '`intersection` | string[] |  |  |',
        ),
        /intersection array item enum differs; missing "b"/,
      ],
      [
        good.replace(
          '`open_alternative` | string[] |  |  |',
          '`open_alternative` | string[] |  | ["open_a"] |',
        ),
        /open_alternative array item enum differs; missing none; extra "open_a"/,
      ],
      [
        good.replace(
          '`prefix_intersection` | string[] |  | ["p2"] |',
          '`prefix_intersection` | string[] |  | ["p1", "p2", "p3"] |',
        ),
        /prefix_intersection array item enum differs; missing none; extra "p1", "p3"/,
      ],
      [
        good.replace(
          '`type_intersection` | integer[]',
          '`type_intersection` | number[]',
        ),
        /type_intersection array item type is number, live schema is integer/,
      ],
      [
        good.replace(
          '`open_prefix` | array |  |  |',
          '`open_prefix` | string[] |  | ["head"] |',
        ),
        /open_prefix array item type is string, live schema is unconstrained/,
      ],
    ]
  ) {
    assert.throws(() => validateMcpContracts([document(bad)], live), expected)
  }

  const ambiguousPrefix = [
    tool('ambiguous_prefix', {
      values: {
        type: 'array',
        items: false,
        prefixItems: [{ type: 'string' }, { type: 'integer' }],
      },
    }),
  ]
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          mcpMarker(
            'ambiguous_prefix',
            table([['`values`', 'array', '', 'Values']]),
          ),
        ),
      ], ambiguousPrefix),
    /ambiguous_prefix\.values live schema composition is unsupported: array items and prefixItems do not share one effective item contract/,
  )
})

test('MCP schema composition and JSON literals preserve exact constraints', () => {
  const live = [{
    name: 'composed',
    inputSchema: {
      type: 'object',
      properties: {
        mode: {
          allOf: [
            { type: 'string' },
            { enum: ['one', 'two'] },
          ],
        },
        whole: {
          allOf: [{ type: 'number' }, { type: 'integer' }],
        },
        object_choice: {
          type: 'object',
          enum: [{ alpha: 1, nested: { beta: 2, gamma: 3 } }],
        },
        config: {
          $ref: '#/$defs/config',
          default: { alpha: 1, nested: { beta: 2, gamma: 3 } },
        },
      },
      $defs: {
        config: { type: 'object' },
      },
    },
  }]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Values',
    'Default',
    'Description',
  ]
  const goodRows = [
    ['`mode`', 'string', '', '["one", "two"]', '', 'Mode'],
    ['`whole`', 'integer', '', '', '', 'Whole number'],
    [
      '`object_choice`',
      'object',
      '',
      '[{"nested":{"gamma":3,"beta":2},"alpha":1}]',
      '',
      'Object enum',
    ],
    [
      '`config`',
      'object',
      '',
      '',
      '{"nested":{"gamma":3,"beta":2},"alpha":1}',
      'Referenced config',
    ],
  ]
  const good = mcpMarker('composed', table(goodRows, headers))
  assert.doesNotThrow(() => validateMcpContracts([document(good)], live))

  assert.throws(
    () =>
      validateMcpContracts([
        document(good.replace('`mode` | string', '`mode` | integer')),
      ], live),
    /composed\.mode type is integer, live schema is string/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(good.replace('`whole` | integer', '`whole` | number')),
      ], live),
    /composed\.whole type is number, live schema is integer/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(good.replace('["one", "two"]', '["one"]')),
      ], live),
    /composed\.mode enum differs; missing "two"/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '{"nested":{"gamma":3,"beta":2},"alpha":1} | Referenced config',
            ' | Referenced config',
          ),
        ),
      ], live),
    /composed\.config omits live default/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '[{"nested":{"gamma":3,"beta":2},"alpha":1}]',
            '[{"nested":{"gamma":4,"beta":2},"alpha":1}]',
          ),
        ),
      ], live),
    /composed\.object_choice enum differs/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '{"nested":{"gamma":3,"beta":2},"alpha":1} | Referenced config',
            '{"nested":{"gamma":4,"beta":2},"alpha":1} | Referenced config',
          ),
        ),
      ], live),
    /composed\.config default is/,
  )

  const impossible = [
    tool('impossible', {
      value: { allOf: [{ type: 'string' }, { type: 'integer' }] },
    }),
  ]
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          mcpMarker(
            'impossible',
            table([['`value`', 'string', '', 'Impossible']]),
          ),
        ),
      ], impossible),
    /impossible\.value live schema composition is unsupported: type composition has no supported non-null type/,
  )
})

test('MCP literal alternatives distinguish open overlapping and disjoint types', () => {
  const live = [
    tool('literal_alternatives', {
      same: {
        anyOf: [
          { type: 'string', enum: ['a'] },
          { type: 'string' },
        ],
      },
      mixed: {
        anyOf: [
          { type: 'string', enum: ['a'] },
          { type: 'integer' },
        ],
      },
      numeric: {
        anyOf: [
          { type: 'integer', enum: [1] },
          { type: 'number' },
        ],
      },
    }),
  ]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Values',
    'Description',
  ]
  const rows = [
    ['`same`', 'string', '', '', 'Any string'],
    ['`mixed`', 'string \\| integer', '', '["a"]', 'String preset or integer'],
    ['`numeric`', 'number', '', '', 'Any number'],
  ]
  const good = mcpMarker('literal_alternatives', table(rows, headers))
  assert.doesNotThrow(() => validateMcpContracts([document(good)], live))
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '`same` | string |  |  |',
            '`same` | string |  | ["a"] |',
          ),
        ),
      ], live),
    /literal_alternatives\.same enum differs; missing none; extra "a"/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '`mixed` | string \\| integer |  | ["a"] |',
            '`mixed` | string \\| integer |  |  |',
          ),
        ),
      ], live),
    /literal_alternatives\.mixed enum differs; missing "a"/,
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          good.replace(
            '`numeric` | number |  |  |',
            '`numeric` | number |  | [1] |',
          ),
        ),
      ], live),
    /literal_alternatives\.numeric enum differs; missing none; extra 1/,
  )
})

test('MCP defaults compose across alternatives and reject ambiguity', () => {
  const unique = [
    tool('default_unique', {
      mode: {
        anyOf: [
          { type: 'string', default: 'a' },
          { type: 'null' },
        ],
      },
    }),
  ]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Default',
    'Description',
  ]
  const uniqueGood = mcpMarker(
    'default_unique',
    table([['`mode`', 'string', '', '"a"', 'Mode']], headers),
  )
  assert.doesNotThrow(() =>
    validateMcpContracts([document(uniqueGood)], unique)
  )
  assert.throws(
    () =>
      validateMcpContracts([
        document(uniqueGood.replace('| "a" | Mode |', '|  | Mode |')),
      ], unique),
    /default_unique\.mode omits live default "a"/,
  )

  const objectDefault = { alpha: 1, nested: { beta: 2, gamma: 3 } }
  const objectDefaults = [{
    name: 'default_object',
    inputSchema: {
      type: 'object',
      properties: {
        config: {
          anyOf: [
            { type: 'object', default: objectDefault },
            {
              type: 'object',
              default: { nested: { gamma: 3, beta: 2 }, alpha: 1 },
            },
          ],
        },
      },
    },
  }]
  const first = mcpMarker(
    'default_object',
    table([
      [
        '`config`',
        'object',
        '',
        '{"alpha":1,"nested":{"beta":2,"gamma":3}}',
        'Config',
      ],
    ], headers),
  )
  const reordered = mcpMarker(
    'default_object',
    table([
      [
        '`config`',
        'object',
        '',
        '{"nested":{"gamma":3,"beta":2},"alpha":1}',
        'Config',
      ],
    ], headers),
  )
  assert.doesNotThrow(() =>
    validateMcpContracts([document(`${first}\n${reordered}`)], objectDefaults)
  )

  const conflicting = [
    tool('default_conflict', {
      mode: {
        oneOf: [
          { type: 'string', default: 'a' },
          { type: 'integer', default: 1 },
        ],
      },
    }),
  ]
  const conflictDocs = mcpMarker(
    'default_conflict',
    table([['`mode`', 'string \\| integer', '', '"a"', 'Mode']], headers),
  )
  assert.throws(
    () => validateMcpContracts([document(conflictDocs)], conflicting),
    /default_conflict\.mode live schema composition is unsupported: default composition has conflicting values: "a", 1/,
  )
})

test('MCP root property composition preserves operator semantics', () => {
  const live = [{
    name: 'root_composed',
    inputSchema: {
      type: 'object',
      properties: { outer_required: { type: 'string' } },
      allOf: [
        {
          type: 'object',
          properties: { whole: { type: 'number' } },
          required: ['whole'],
        },
        {
          type: 'object',
          properties: { whole: { type: 'integer' } },
        },
      ],
      anyOf: [
        {
          type: 'object',
          properties: {
            same: { type: 'string', enum: ['fixed'] },
            mixed: { type: 'string', enum: ['fixed'] },
            optional: { type: 'string' },
          },
          required: ['same', 'mixed', 'optional', 'outer_required'],
        },
        {
          type: 'object',
          properties: {
            same: { type: 'string' },
            mixed: { type: 'integer' },
            optional: { type: 'string' },
          },
          required: ['same', 'mixed', 'outer_required'],
        },
      ],
    },
  }]
  const headers = [
    'Parameter',
    'Type',
    'Required',
    'Values',
    'Description',
  ]
  const rows = [
    ['`outer_required`', 'string', '**yes**', '', 'Outer property'],
    ['`whole`', 'integer', '**yes**', '', 'Whole'],
    ['`same`', 'string', '**yes**', '', 'Any string'],
    ['`mixed`', 'string \\| integer', '**yes**', '["fixed"]', 'Mixed'],
    ['`optional`', 'string', '', '', 'Optional'],
  ]
  const good = mcpMarker('root_composed', table(rows, headers))
  assert.doesNotThrow(() => validateMcpContracts([document(good)], live))
  for (
    const [bad, expected] of [
      [
        good.replace('`whole` | integer', '`whole` | number'),
        /root_composed\.whole type is number, live schema is integer/,
      ],
      [
        good.replace(
          '`same` | string | **yes** |  |',
          '`same` | string | **yes** | ["fixed"] |',
        ),
        /root_composed\.same enum differs; missing none; extra "fixed"/,
      ],
      [
        good.replace(
          '`mixed` | string \\| integer | **yes** | ["fixed"] |',
          '`mixed` | string \\| integer | **yes** |  |',
        ),
        /root_composed\.mixed enum differs; missing "fixed"/,
      ],
      [
        good.replace(
          '`optional` | string |  |',
          '`optional` | string | **yes** |',
        ),
        /root_composed\.optional requiredness disagrees/,
      ],
      [
        good.replace(
          '`outer_required` | string | **yes** |',
          '`outer_required` | string |  |',
        ),
        /root_composed\.outer_required requiredness disagrees/,
      ],
    ]
  ) {
    assert.throws(() => validateMcpContracts([document(bad)], live), expected)
  }

  const partial = [{
    name: 'partial_root',
    inputSchema: {
      anyOf: [
        { type: 'object', properties: { value: { type: 'string' } } },
        { type: 'object', properties: {} },
      ],
    },
  }]
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          mcpMarker(
            'partial_root',
            table([['`value`', 'string', '', 'Value']]),
          ),
        ),
      ], partial),
    /partial_root\.value live schema composition is unsupported: property is unconstrained in some alternative branches/,
  )

  const impossible = [{
    name: 'impossible_root',
    inputSchema: {
      allOf: [
        { type: 'object', properties: { value: { type: 'string' } } },
        { type: 'object', properties: { value: { type: 'integer' } } },
      ],
    },
  }]
  assert.throws(
    () =>
      validateMcpContracts([
        document(
          mcpMarker(
            'impossible_root',
            table([['`value`', 'string', '', 'Value']]),
          ),
        ),
      ], impossible),
    /impossible_root\.value live schema composition is unsupported: type composition has no supported non-null type/,
  )
})

test('MCP documented type grammar rejects substring near-misses', () => {
  const live = [
    tool('type_grammar', {
      scalar: { type: 'string' },
      union: { anyOf: [{ type: 'string' }, { type: 'object' }] },
      mixed_array: {
        anyOf: [
          { type: 'string' },
          { type: 'array', items: { type: 'string' } },
        ],
      },
      loose_list: { type: 'array' },
    }),
  ]
  const good = mcpMarker(
    'type_grammar',
    table([
      ['`scalar`', 'string', '', 'Scalar'],
      ['`union`', 'string \\| object', '', 'Union'],
      ['`mixed_array`', 'string or string[]', '', 'Mixed array'],
      ['`loose_list`', 'list', '', 'List'],
    ]),
  )
  assert.doesNotThrow(() => validateMcpContracts([document(good)], live))
  for (const nearMiss of ['not-a-string', 'stringish', 'a string maybe']) {
    assert.throws(
      () =>
        validateMcpContracts([
          document(
            good.replace('`scalar` | string', `\`scalar\` | ${nearMiss}`),
          ),
        ], live),
      new RegExp(
        `scalar has unsupported documented type ${JSON.stringify(nearMiss)}`,
      ),
    )
  }
})

test('empty MCP markers prove true emptiness and reject false emptiness', () => {
  const empty =
    '<!-- doc-contract:mcp tool=read_library schema=/ surface=none requiredness=global -->'
  assert.doesNotThrow(() =>
    validateMcpContracts([document(empty)], [tool('read_library')])
  )
  assert.throws(
    () =>
      validateMcpContracts([document(empty)], [
        tool('read_library', { limit: { type: 'integer' } }),
      ]),
    /declares an empty surface but live schema has limit/,
  )
  const mixed = `${empty}\n${mcpMarker('read_library', table([]))}`
  assert.throws(
    () => validateMcpContracts([document(mixed)], [tool('read_library')]),
    /read_library mixes empty and non-empty contract surfaces for schema \//,
  )
})

test('bounded SOP recognizer handles positional operations, nesting, multiple calls, ellipses, and bare tools', () => {
  const live = [
    tool('audit_state', {
      operation: { type: 'string' },
      scope: { type: 'string' },
      filters: { type: 'object' },
    }),
    tool('search_tracks', {
      query: { type: 'string' },
      limit: { type: 'integer' },
    }),
    tool('read_library'),
  ]
  const source = `
\`\`\`
audit_state(scan, scope="/Music", filters={nested: [1, {value: "x,y"}]})
search_tracks(query="kick (live)", limit=20, ...)
read_library
Output: prose is ignored
\`\`\`

\`\`\`bash
unknown_tool(bad=true)
\`\`\`
`
  const { calls } = extractSopCalls(source, live)
  assert.deepEqual(
    calls.map(({ name, namedArgs }) => ({ name, namedArgs })),
    [
      { name: 'audit_state', namedArgs: ['scope', 'filters'] },
      { name: 'search_tracks', namedArgs: ['query', 'limit'] },
      { name: 'read_library', namedArgs: [] },
    ],
  )
  assert.doesNotThrow(() =>
    validateSopContracts([document(source, 'sop.mdx')], live)
  )
})

test('SOP validation rejects unknown tools and unknown top-level arguments', () => {
  const live = [tool('search_tracks', { query: { type: 'string' } })]
  assert.throws(
    () =>
      validateSopContracts(
        [document('```\nmissing_tool(query="x")\n```')],
        live,
      ),
    /unknown SOP tool missing_tool/,
  )
  assert.throws(
    () =>
      validateSopContracts(
        [document('```\nsearch_tracks(typo="x")\n```')],
        live,
      ),
    /has no top-level argument typo/,
  )
})

test('recognized SOP call fences reject missing and mismatched delimiters with locations', () => {
  const live = [
    tool('search_tracks', {
      query: { type: 'string' },
      filters: { type: 'object' },
    }),
  ]
  assert.throws(
    () =>
      validateSopContracts([
        document(
          'intro\n```\nsearch_tracks(query="x"\n```',
          'missing-close.mdx',
        ),
      ], live),
    /missing-close\.mdx:3: malformed SOP tool-call fence: missing \)/,
  )
  assert.throws(
    () =>
      validateSopContracts([
        document('```\nsearch_tracks(filters=[})\n```', 'mismatched.mdx'),
      ], live),
    /mismatched\.mdx:2: malformed SOP tool-call fence: found } while expecting \]/,
  )
})

test('SOP validation reports each call at its actual fence line', () => {
  const live = [tool('search_tracks', { query: { type: 'string' } })]
  const source = `intro
\`\`\`
search_tracks(query="known")
search_tracks(typo="second")
missing_tool(query="third")
\`\`\``
  assert.throws(
    () =>
      validateSopContracts([
        document(source, 'multi-call-sop.mdx'),
      ], live),
    (error) => {
      assert.match(
        error.message,
        /multi-call-sop\.mdx:4: search_tracks has no top-level argument typo/,
      )
      assert.match(
        error.message,
        /multi-call-sop\.mdx:5: unknown SOP tool missing_tool/,
      )
      return true
    },
  )
})

test('SOP discovery resolves composed parameters and rejects bare parameterized tools', () => {
  const live = [
    {
      name: 'composed_args',
      inputSchema: {
        allOf: [{
          type: 'object',
          properties: { query: { type: 'string' } },
          required: ['query'],
        }],
      },
    },
    {
      name: 'referenced_args',
      inputSchema: {
        $ref: '#/$defs/args',
        $defs: {
          args: {
            type: 'object',
            properties: { query: { type: 'string' } },
            required: ['query'],
          },
        },
      },
    },
  ]
  const calls = '```\ncomposed_args(query="x")\nreferenced_args(query="y")\n```'
  assert.doesNotThrow(() =>
    validateSopContracts([document(calls, 'composed-sop.mdx')], live)
  )
  assert.throws(
    () =>
      validateSopContracts([
        document('```\ncomposed_args\n```', 'bare-composed-sop.mdx'),
      ], live),
    /bare-composed-sop\.mdx:2: malformed SOP tool-call fence: composed_args requires call syntax with arguments/,
  )

  const cyclic = [{
    name: 'cyclic_args',
    inputSchema: {
      $ref: '#/$defs/args',
      $defs: { args: { $ref: '#/$defs/args' } },
    },
  }]
  assert.throws(
    () => validateSopContracts([], cyclic),
    /src\/tools\/params\.rs:1: SOP tool cyclic_args input schema composition is unsupported: cyclic JSON Schema ref/,
  )
})

test('SOP call fences reject trailing unmatched closing delimiters', () => {
  const live = [tool('x')]
  for (
    const [source, closer] of [
      ['```\nx())\n```', ')'],
      ['```\nx() ]\n```', ']'],
    ]
  ) {
    assert.throws(
      () =>
        validateSopContracts([
          document(source, 'trailing-closer-sop.mdx'),
        ], live),
      new RegExp(
        `trailing-closer-sop\\.mdx:2: malformed SOP tool-call fence: unexpected \\${closer}`,
      ),
    )
  }
})

test('CLI parser inventories root, short-only, and optional surfaces exactly', () => {
  const rootHelp =
    `Commands:\n  alpha  Alpha\n  help   Synthetic\n\nOptions:\n      --config <CONFIG>  Config path\n  -h, --help             Print help\n  -V, --version          Print version`
  const root = parseRootCliHelp(rootHelp)
  assert.deepEqual([...root.keys()], ['alpha'])
  assert.deepEqual([...parseRootCliOptions(rootHelp).keys()], ['--config'])
  const fields = parseApplicationCliHelp(
    `Arguments:\n  <PATHS>...  Inputs\n  [PATH]...    Optional inputs\n\nOptions:\n  -q             Quiet\n  -j, --jobs <JOBS>\n      [default: 4]\n  --version\n      Print application version\n  -h, --help\n      Print help`,
  )
  assert.deepEqual(
    [...fields.keys()],
    ['<paths>', '[path]', '-q', '--jobs', '--version'],
  )
  assert.equal(fields.get('-q').short, '')
  assert.equal(fields.get('--jobs').default, '4')
})

test('CLI contracts detect missing/extra fields and explicit empty surfaces', () => {
  const inventory = {
    commands: new Map([
      ['alpha', {}],
      ['empty', {}],
    ]),
    fields: new Map([
      ['root', new Map()],
      [
        'alpha',
        new Map([['--jobs', { name: '--jobs', short: '-j', default: '4' }]]),
      ],
      ['empty', new Map()],
    ]),
  }
  const root = `<!-- doc-contract:cli command=root surface=commands -->\n${
    table(
      [
        ['`alpha`', 'Alpha'],
        ['`empty`', 'Empty'],
      ],
      ['Command', 'Description'],
    )
  }\n<!-- /doc-contract:cli -->\n<!-- doc-contract:cli command=root surface=none -->`
  const alpha = `<!-- doc-contract:cli command=alpha surface=options -->\n${
    table(
      [['`--jobs`', '`-j`', '`4`']],
      ['Flag', 'Short', 'Default'],
    )
  }\n<!-- /doc-contract:cli -->`
  const empty = '<!-- doc-contract:cli command=empty surface=none -->'
  assert.doesNotThrow(() =>
    validateCliContracts([document(`${root}\n${alpha}\n${empty}`)], inventory)
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(
          `${
            root.replace(
              '\n<!-- doc-contract:cli command=root surface=none -->',
              '',
            )
          }\n${alpha}\n${empty}`,
        ),
      ], inventory),
    /CLI command root has no marked contract surface/,
  )

  const rootConfigInventory = {
    ...inventory,
    fields: new Map(inventory.fields).set(
      'root',
      new Map([
        ['--config', { name: '--config', short: '', default: null }],
      ]),
    ),
  }
  assert.throws(
    () =>
      validateCliContracts(
        [document(`${root}\n${alpha}\n${empty}`)],
        rootConfigInventory,
      ),
    /CLI command root declares empty but has --config/,
  )

  const shortOnlyInventory = {
    ...inventory,
    fields: new Map(inventory.fields).set(
      'alpha',
      new Map([
        ['--jobs', { name: '--jobs', short: '-j', default: '4' }],
        ['-q', { name: '-q', short: '', default: null }],
      ]),
    ),
  }
  assert.throws(
    () =>
      validateCliContracts(
        [document(`${root}\n${alpha}\n${empty}`)],
        shortOnlyInventory,
      ),
    /CLI command alpha is missing -q/,
  )

  const optionalPathInventory = {
    ...inventory,
    fields: new Map(inventory.fields).set(
      'alpha',
      new Map([
        ['--jobs', { name: '--jobs', short: '-j', default: '4' }],
        ['[path]', { name: '[path]', short: '', default: null }],
      ]),
    ),
  }
  assert.throws(
    () =>
      validateCliContracts(
        [document(`${root}\n${alpha}\n${empty}`)],
        optionalPathInventory,
      ),
    /CLI command alpha is missing \[path\]/,
  )

  assert.throws(
    () => validateCliContracts([document(`${root}\n${empty}`)], inventory),
    /alpha has no marked contract surface/,
  )
  const alphaOmitted =
    `<!-- doc-contract:cli command=alpha surface=options -->\n${
      table([], ['Flag', 'Short', 'Default'])
    }\n<!-- /doc-contract:cli -->`
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alphaOmitted}\n${empty}`),
      ], inventory),
    /fixture\.mdx:\d+: CLI command alpha is missing --jobs/,
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${empty.replace('empty', 'alpha')}\n${empty}`),
      ], inventory),
    /alpha declares empty but has --jobs/,
  )
  const emptyOptions =
    `<!-- doc-contract:cli command=empty surface=options -->\n${
      table([['`--ghost`', '', '']], ['Flag', 'Short', 'Default'])
    }\n<!-- /doc-contract:cli -->`
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha}\n${empty}\n${emptyOptions}`),
      ], inventory),
    /CLI command empty mixes empty and non-empty contract surfaces/,
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha.replace('--jobs', '--wrong')}\n${empty}`),
      ], inventory),
    /unknown CLI field alpha --wrong/,
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha.replace('`-j`', '')}\n${empty}`),
      ], inventory),
    /alpha --jobs short flag drift/,
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha.replace('`-j`', '`-x`')}\n${empty}`),
      ], inventory),
    /alpha --jobs short flag drift/,
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha.replace('`4`', '`5`')}\n${empty}`),
      ], inventory),
    /alpha --jobs default is 5, live help is 4/,
  )
  const extraRoot = root.replace(
    '| `empty` | Empty |',
    '| `empty` | Empty |\n| `extra` | Extra |',
  )
  assert.throws(
    () =>
      validateCliContracts([
        document(`${extraRoot}\n${alpha}\n${empty}`),
      ], inventory),
    /CLI command inventory mismatch; documented: alpha, empty, extra/,
  )
  const ghost = '<!-- doc-contract:cli command=ghost surface=none -->'
  assert.throws(
    () =>
      validateCliContracts([
        document(`${root}\n${alpha}\n${empty}\n${ghost}`),
      ], inventory),
    /fixture\.mdx:\d+: documented CLI command is not live: ghost/,
  )
})

test('workflow validator rejects missing IDs, route drift, and noncanonical order', () => {
  const missing = structuredClone(workflows).slice(1)
  assert.throws(() => validateWorkflows(missing), /expected exactly/)

  const badRoute = structuredClone(workflows)
  badRoute[0].route = '/wrong/'
  assert.throws(() => validateWorkflows(badRoute), /route/)

  const reordered = structuredClone(workflows)
  ;[reordered[0], reordered[1]] = [reordered[1], reordered[0]]
  assert.throws(() => validateWorkflows(reordered), /IDs\/order/)
})

test('XML workflow backup contracts reject missing, negated, and weaker gates', () => {
  assert.doesNotThrow(() =>
    validateXmlBackupContracts(workflows, XML_BACKUP_SUCCESS_CONDITION)
  )
  const xmlIndex = workflows.findIndex((workflow) =>
    workflow.sideEffects.outputs.some((entry) =>
      entry.kind === 'metadata-xml' || entry.kind === 'playlist-xml'
    )
  )
  assert.notEqual(xmlIndex, -1)

  const missing = structuredClone(workflows)
  missing[xmlIndex].sideEffects.outputs = missing[
    xmlIndex
  ].sideEffects.outputs.filter((entry) => entry.kind !== 'backup')
  assert.throws(
    () => validateXmlBackupContracts(missing, XML_BACKUP_SUCCESS_CONDITION),
    /site\/src\/data\/workflows\.mjs:1: workflows\[\d+\].*exactly one backup/,
  )

  const negated = structuredClone(workflows)
  negated[xmlIndex].sideEffects.outputs.find((entry) => entry.kind === 'backup')
    .condition = `not ${XML_BACKUP_SUCCESS_CONDITION}`
  assert.throws(
    () => validateXmlBackupContracts(negated, XML_BACKUP_SUCCESS_CONDITION),
    /site\/src\/data\/workflows\.mjs:1: workflows\[\d+\].*condition must equal/,
  )

  const weaker = structuredClone(workflows)
  weaker[xmlIndex].sideEffects.outputs.find((entry) => entry.kind === 'backup')
    .condition = 'XML export usually follows a successful backup attempt.'
  assert.throws(
    () => validateXmlBackupContracts(weaker, XML_BACKUP_SUCCESS_CONDITION),
    /site\/src\/data\/workflows\.mjs:1: workflows\[\d+\].*condition must equal/,
  )

  assert.throws(
    () =>
      validateXmlBackupContracts(
        workflows,
        'XML export follows a successful backup.',
      ),
    /XML_BACKUP_SUCCESS_CONDITION must equal the canonical fail-closed condition/,
  )
})

test('goal definitions and workflow memberships are exact and exhaustive', () => {
  assert.deepEqual(
    goalDefinitions.map(({ id, title }) => [id, title]),
    [
      ['inspect-health', 'Inspect collection health'],
      ['clean-library', 'Clean existing metadata'],
      ['prepare-downloads', 'Prepare newly downloaded music'],
      ['classify-genres', 'Classify or audit genres'],
      ['build-for-mixing', 'Build for mixing'],
      ['explore-dj-ideas', 'Explore DJ ideas'],
    ],
  )
  assert.equal(
    workflows.flatMap((workflow) => workflow.goals).length,
    workflows.length,
  )
  assert.equal(
    new Set(workflows.flatMap((workflow) => workflow.goals)).size,
    goalDefinitions.length,
  )
  assert.doesNotThrow(() => validateWorkflows(workflows, goalDefinitions))

  const missingDefinition = structuredClone(goalDefinitions).slice(1)
  assert.throws(
    () => validateWorkflows(workflows, missingDefinition),
    /goal definitions must contain exactly/,
  )

  const duplicatedDefinition = structuredClone(goalDefinitions)
  duplicatedDefinition[1] = structuredClone(duplicatedDefinition[0])
  assert.throws(
    () => validateWorkflows(workflows, duplicatedDefinition),
    /goalDefinitions\[1\]/,
  )

  const reorderedDefinitions = structuredClone(goalDefinitions)
  ;[reorderedDefinitions[0], reorderedDefinitions[1]] = [
    reorderedDefinitions[1],
    reorderedDefinitions[0],
  ]
  assert.throws(
    () => validateWorkflows(workflows, reorderedDefinitions),
    /goalDefinitions\[0\]/,
  )

  for (
    const mutate of [
      (definitions) => {
        definitions[0].id = 'unknown-goal'
      },
      (definitions) => {
        definitions[0].summary = ''
      },
      (definitions) => {
        definitions[0].extra = true
      },
    ]
  ) {
    const definitions = structuredClone(goalDefinitions)
    mutate(definitions)
    assert.throws(() => validateWorkflows(workflows, definitions))
  }

  for (
    const goals of [
      [],
      ['unknown-goal'],
      ['clean-library', 'clean-library'],
      ['clean-library', 'inspect-health'],
    ]
  ) {
    const records = structuredClone(workflows)
    records[0].goals = goals
    assert.throws(() => validateWorkflows(records, goalDefinitions), /goals/)
  }

  const wrongMembership = structuredClone(workflows)
  wrongMembership.find((workflow) => workflow.id === 'library-health').goals = [
    'clean-library',
  ]
  assert.throws(
    () => validateWorkflows(wrongMembership, goalDefinitions),
    /goals/,
  )
})

test('first-session prompt is structurally singular and capability-bounded', () => {
  const liveTools = [
    tool('read_library'),
    tool('write_xml'),
    tool('lookup_discogs'),
  ]
  const fixture = (prompt = FIRST_SESSION_PROMPT) =>
    document(
      `Before
<!-- doc-contract:first-session-prompt tool=read_library -->

\`\`\`text
${prompt}
\`\`\`

<!-- /doc-contract:first-session-prompt -->
After`,
      'first-session.mdx',
    )

  const valid = fixture()
  assert.doesNotThrow(() => validateFirstSessionPage(valid, liveTools))

  const malformedCases = [
    valid.content.replace(
      '<!-- doc-contract:first-session-prompt tool=read_library -->',
      '',
    ),
    valid.content.replace('<!-- /doc-contract:first-session-prompt -->', ''),
    valid.content.replace('tool=read_library', 'tool=read_librar'),
    `${valid.content}\n${valid.content}`,
    valid.content.replace(
      '\`\`\`text',
      '<!-- doc-contract:first-session-prompt tool=read_library -->\n\`\`\`text',
    ),
    valid.content.replace('\`\`\`text', '\`\`\`json'),
    `${valid.content}\n\`\`\`text\nwrite_xml\n\`\`\``,
  ]
  malformedCases.forEach((content) => {
    assert.throws(() =>
      validateFirstSessionPage(document(content, 'malformed.mdx'), liveTools)
    )
  })

  for (const indent of [' ', '  ', '   ']) {
    const indentedOutside =
      `${valid.content}\n${indent}\`\`\`text\n${indent}write_xml\n${indent}\`\`\``
    assert.throws(
      () =>
        validateFirstSessionPage(
          document(indentedOutside, 'indented-outside.mdx'),
          liveTools,
        ),
      /exactly one runnable fence; found 2/,
    )
  }

  const containerFences = [
    `${valid.content}\n- hidden call\n    \`\`\`text\n    write_xml\n    \`\`\``,
    `${valid.content}\n> \`\`\`text\n> write_xml\n> \`\`\``,
  ]
  containerFences.forEach((content) => {
    assert.throws(
      () =>
        validateFirstSessionPage(
          document(content, 'container-fence.mdx'),
          liveTools,
        ),
      /exactly one runnable fence; found 2/,
    )
  })

  const indentedInside = valid.content
    .replace('\n\`\`\`text\n', '\n  \`\`\`text\n')
    .replace(
      '\n\`\`\`\n\n<!-- /doc-contract:first-session-prompt -->',
      '\n  \`\`\`\n\n<!-- /doc-contract:first-session-prompt -->',
    )
  assert.throws(
    () =>
      validateFirstSessionPage(
        document(indentedInside, 'indented-inside.mdx'),
        liveTools,
      ),
    /unindented triple-backtick text fence/,
  )

  const unmatchedOutside = `${valid.content}\n  \`\`\`text\n  write_xml`
  assert.throws(
    () =>
      validateFirstSessionPage(
        document(unmatchedOutside, 'unmatched-outside.mdx'),
        liveTools,
      ),
    /unmatched Markdown fence opening/,
  )

  assert.throws(() =>
    validateFirstSessionPage(
      fixture(
        FIRST_SESSION_PROMPT.replace(
          "Use only reklawdbox's read_library tool.",
          "Use reklawdbox's read_library and write_xml tools.",
        ),
      ),
      liveTools,
    )
  )

  const boundaryOmissions = [
    ['external services', 'outside services'],
    ['analyze audio', 'inspect audio'],
    ['use or populate enrichment/audio\ncaches', 'use existing data'],
    ['write files', 'change files'],
    ['stage changes', 'prepare changes'],
    ['export XML', 'make XML'],
  ]
  boundaryOmissions.forEach(([boundary, replacement]) => {
    assert.throws(() =>
      validateFirstSessionPage(
        fixture(FIRST_SESSION_PROMPT.replace(boundary, replacement)),
        liveTools,
      )
    )
  })

  const parameterized = structuredClone(liveTools)
  parameterized[0].inputSchema.properties = { limit: { type: 'integer' } }
  assert.throws(
    () => validateFirstSessionPage(valid, parameterized),
    /parameter-free/,
  )

  const composedParameter = structuredClone(liveTools)
  composedParameter[0].inputSchema = {
    type: 'object',
    $defs: {
      arguments: {
        type: 'object',
        properties: { limit: { type: 'integer' } },
      },
    },
    allOf: [{ $ref: '#/$defs/arguments' }],
  }
  assert.throws(
    () => validateFirstSessionPage(valid, composedParameter),
    /parameter-free/,
  )
})

test('onboarding sources preserve the three-selection journey and version sentinel', () => {
  const read = (relative) =>
    readFileSync(new URL(`../${relative}`, import.meta.url), 'utf8')
  const sources = {
    homepage: read('site/src/content/docs/index.mdx'),
    install: read('site/src/content/docs/getting-started/index.mdx'),
    firstSession: read(
      'site/src/content/docs/getting-started/first-session.mdx',
    ),
    goalChooser: read('site/src/components/GoalChooser.astro'),
    astroConfig: read('site/astro.config.mjs'),
    cargoToml: read('Cargo.toml'),
    builtPaths: new Set(['getting-started/first-session/index.html']),
  }
  assert.doesNotThrow(() => validateOnboardingSources(sources))

  const invalid = [
    { homepage: sources.homepage.replace("link: '/getting-started/'", '') },
    {
      install: sources.install.replaceAll(
        '/getting-started/first-session/',
        '/getting-started/',
      ),
    },
    {
      install:
        `${sources.install}\n\`\`\`text\n${FIRST_SESSION_PROMPT}\n\`\`\``,
    },
    { homepage: `${sources.homepage}\nStart here: Library Cleanup` },
    { homepage: `${sources.homepage}\n/workflows/library-cleanup/` },
    {
      goalChooser: sources.goalChooser.replace(
        'workflow.goals.includes(goal.id)',
        'workflow.id === goal.id',
      ),
    },
    {
      firstSession: sources.firstSession.replaceAll(
        'factory sampler',
        'built-in sample',
      ),
    },
    {
      firstSession: sources.firstSession.replace(
        'contains only factory sampler\ncontent',
        'contains only built-in sample\ncontent',
      ),
    },
    {
      goalChooser: sources.goalChooser.replaceAll(
        '--sl-color-accent-high',
        '--sl-color-text-accent',
      ),
    },
    {
      astroConfig: sources.astroConfig.replace(
        "slug: 'getting-started/first-session'",
        "slug: 'after-workflows'",
      ),
    },
    {
      astroConfig: sources.astroConfig.replace(
        "label: 'First 10 minutes'",
        "label: 'First result'",
      ),
    },
    { homepage: sources.homepage.replace(/v\d+\.\d+\.\d+\s+—/, '') },
    {
      homepage: sources.homepage.replace(/v\d+\.\d+\.\d+\s+—/, 'v9.9.9 —'),
    },
    { homepage: `${sources.homepage}\nv0.30.0 — duplicate` },
    { builtPaths: new Set() },
  ]
  invalid.forEach((changes) => {
    assert.throws(() => validateOnboardingSources({ ...sources, ...changes }))
  })
})

test('docs gates preserve both manifests in push, pull request, and release inventories', () => {
  const workflow = readFileSync(
    new URL('../.github/workflows/docs-pages.yml', import.meta.url),
    'utf8',
  )
  assert.equal((workflow.match(/- "Cargo\.toml"/g) ?? []).length, 2)
  assert.equal((workflow.match(/- "Cargo\.lock"/g) ?? []).length, 2)

  const release = readFileSync(
    new URL('./release.sh', import.meta.url),
    'utf8',
  )
  const inventories = parseDocsGatePathInventories(workflow, release)
  assert.doesNotThrow(() => validateDocsGatePathInventories(inventories))

  for (const manifest of ['Cargo.toml', 'Cargo.lock']) {
    for (const inventoryName of ['push', 'pullRequest', 'release']) {
      const incomplete = structuredClone(inventories)
      incomplete[inventoryName] = incomplete[inventoryName].filter(
        (entry) => entry !== manifest,
      )
      assert.throws(
        () => validateDocsGatePathInventories(incomplete),
        new RegExp(manifest.replace('.', '\\.')),
      )
    }
  }

  for (const sourcePath of ['src/audio.rs', 'src/color.rs', 'src/genre.rs']) {
    for (const inventoryName of ['push', 'pullRequest', 'release']) {
      const incomplete = structuredClone(inventories)
      incomplete[inventoryName] = incomplete[inventoryName].filter(
        (entry) => entry !== sourcePath,
      )
      assert.throws(
        () => validateDocsGatePathInventories(incomplete),
        new RegExp(sourcePath.replaceAll('/', '\\/').replace('.', '\\.')),
      )
    }
  }

  const missingExistingCiPath = structuredClone(inventories)
  missingExistingCiPath.push = missingExistingCiPath.push.filter(
    (entry) => entry !== 'site/**',
  )
  assert.throws(() => validateDocsGatePathInventories(missingExistingCiPath))

  const missingExistingReleasePath = structuredClone(inventories)
  missingExistingReleasePath.release = missingExistingReleasePath.release
    .filter((entry) => entry !== 'site')
  assert.throws(() =>
    validateDocsGatePathInventories(missingExistingReleasePath)
  )
})

test('color palette contract follows the canonical Rust name and XML pairs', () => {
  const source = `
pub const COLORS: &[(&str, i32)] = &[
    ("Blue", 0x0000FF),
    ("Green", 0x00FF00),
    ("Lemon", 0xFFFF00),
];
`
  const content = [
    '{/* doc-contract:color-palette source=src/color.rs */}',
    '',
    '| Color name | XML hex |',
    '| --- | --- |',
    '| Blue | `0x0000FF` |',
    '| Green | `0x00FF00` |',
    '| Lemon | `0xFFFF00` |',
    '',
    '{/* /doc-contract:color-palette */}',
  ].join('\n')
  const file = 'site/src/content/docs/reference/xml-export.mdx'
  assert.doesNotThrow(() =>
    validateColorPaletteContract(document(content, file), source)
  )

  const missing = content.replace('| Lemon | `0xFFFF00` |\n', '')
  assert.throws(
    () => validateColorPaletteContract(document(missing, file), source),
    /xml-export\.mdx:1: color palette differs from src\/color\.rs; missing: Lemon/,
  )
  const wrong = content.replace('`0x00FF00`', '`0x00AA00`')
  assert.throws(
    () => validateColorPaletteContract(document(wrong, file), source),
    /xml-export\.mdx:1: Green XML hex is 0x00AA00, src\/color\.rs expects 0x00FF00/,
  )
  const duplicate = content.replace(
    '| Lemon | `0xFFFF00` |',
    '| Lemon | `0xFFFF00` |\n| Blue | `0x0000FF` |',
  )
  assert.throws(
    () => validateColorPaletteContract(document(duplicate, file), source),
    /xml-export\.mdx:1: duplicate color row Blue; source=src\/color\.rs/,
  )
})

test('batch audio extension contract follows the canonical Rust set', () => {
  const source =
    'pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["flac", "wav", "mp3", "m4a", "aac", "aiff"];'
  const content = [
    '<!-- doc-contract:batch-audio-extensions source=src/audio.rs -->',
    '',
    '```sh',
    'find "/batch" -maxdepth 1 -type f \\',
    '  \\( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" -o -iname "*.aac" -o -iname "*.aiff" \\) \\',
    '  -exec mv -n {} "/dest/" \\;',
    '```',
    '',
    '<!-- /doc-contract:batch-audio-extensions -->',
  ].join('\n')
  const file = 'site/src/partials/sops/batch-import.mdx'
  assert.doesNotThrow(() =>
    validateBatchAudioExtensionsContract(document(content, file), source)
  )

  const missing = content.replace(' -o -iname "*.aac"', '')
  assert.throws(
    () => validateBatchAudioExtensionsContract(document(missing, file), source),
    /batch-import\.mdx:1: batch audio extensions differ from src\/audio\.rs; missing: aac/,
  )
  const extra = content.replace(
    ' -o -iname "*.aiff"',
    ' -o -iname "*.aiff" -o -iname "*.ogg"',
  )
  assert.throws(
    () => validateBatchAudioExtensionsContract(document(extra, file), source),
    /batch-import\.mdx:1: batch audio extensions differ from src\/audio\.rs; missing: none; extra: ogg/,
  )
  const duplicate = content.replace(
    ' -o -iname "*.aiff"',
    ' -o -iname "*.aiff" -o -iname "*.flac"',
  )
  assert.throws(
    () =>
      validateBatchAudioExtensionsContract(document(duplicate, file), source),
    /batch-import\.mdx:1: batch audio extension block contains a duplicate extension; source=src\/audio\.rs/,
  )
})

test('runtime help keeps 11 pages, 9 menu entries, and 7 recommendations separate', () => {
  const menu = workflows
    .filter((workflow) => workflow.runtimeHelp)
    .sort((left, right) =>
      left.runtimeHelp.menuOrder - right.runtimeHelp.menuOrder
    )
  const recommended = menu
    .filter((workflow) => workflow.runtimeHelp.recommendedOrder !== null)
    .sort((left, right) =>
      left.runtimeHelp.recommendedOrder - right.runtimeHelp.recommendedOrder
    )
  const payload = {
    workflows: menu.map((workflow) => ({ name: workflow.title })),
    recommended_order: recommended
      .map((workflow, index) => `${index + 1}. ${workflow.title} — detail`)
      .join('\n'),
  }
  assert.equal(workflows.length, 11)
  assert.equal(menu.length, 9)
  assert.equal(recommended.length, 7)
  assert.doesNotThrow(() => compareRuntimeHelp(workflows, payload))
  payload.recommended_order = [
    recommended[1],
    recommended[0],
    ...recommended.slice(2),
  ]
    .map((workflow, index) => `${index + 1}. ${workflow.title} — detail`)
    .join('\n')
  assert.throws(
    () => compareRuntimeHelp(workflows, payload),
    /runtime recommended order drift/,
  )
  payload.recommended_order = recommended
    .map((workflow, index) => `${index + 1}. ${workflow.title} — detail`)
    .join('\n')
  payload.workflows.push({ name: 'Library Cleanup' })
  assert.throws(
    () => compareRuntimeHelp(workflows, payload),
    /runtime help menu drift/,
  )
})

test('built-link validation detects internal and runtime-help route omissions', () => {
  const built = new Set(['ok/index.html'])
  assert.doesNotThrow(() =>
    validateBuiltLinkSet(
      [document('<a href="/ok/">ok</a>', 'dist/index.html')],
      built,
    )
  )
  assert.doesNotThrow(() =>
    validateBuiltLinkSet(
      [document('<a href="/v1.0/">versioned directory</a>')],
      new Set(['v1.0/index.html']),
    )
  )
  assert.throws(
    () =>
      validateBuiltLinkSet(
        [document('<a href="/missing/">bad</a>', 'dist/index.html')],
        built,
      ),
    /built target missing for \/missing\//,
  )
  assert.throws(
    () =>
      validateBuiltLinkSet(
        [document(
          '<a href="https:\/\/reklawdbox.com\/runtime-missing/">bad</a>',
        )],
        built,
      ),
    /built target missing for https:\/\/reklawdbox\.com\/runtime-missing\/ \(resolved \/runtime-missing\/\)/,
  )
  assert.throws(
    () =>
      validateBuiltLinkSet(
        [
          document(
            '<a href="//reklawdbox.com/missing-protocol-relative/">bad</a>',
          ),
        ],
        built,
      ),
    /built target missing for \/\/reklawdbox\.com\/missing-protocol-relative\/ \(resolved \/missing-protocol-relative\/\)/,
  )
  assert.doesNotThrow(() =>
    validateBuiltLinkSet(
      [document('<a href="//example.com/not-built/">external</a>')],
      built,
    )
  )
  assert.doesNotThrow(() =>
    validateBuiltLinkSet([
      document('<a href="../ok/">relative</a>', 'dist/nested/index.html'),
    ], built)
  )
  assert.doesNotThrow(() =>
    validateBuiltLinkSet([
      document(
        '<a href="../../../ok/">clamped</a>',
        'dist/deep/nested/index.html',
      ),
    ], built)
  )
  assert.throws(
    () =>
      validateBuiltLinkSet([
        document(
          '<p>first line</p>\n<a href="../missing-relative/">bad</a>',
          'dist/nested/index.html',
        ),
      ], built),
    /dist\/nested\/index\.html:2: built target missing for \.\.\/missing-relative\//,
  )
})

test('runtime help derives nine topics and validates URLs from every payload', () => {
  const topics = runtimeHelpTopics(workflows)
  assert.equal(topics.length, 9)
  const built = new Set(['getting-started/index.html'])
  const payloads = topics.map((topic) => ({
    source: `src/tools/help_handler.rs:1: help(${JSON.stringify(topic)})`,
    payload: { guide: 'https://reklawdbox.com/getting-started/' },
  }))
  assert.doesNotThrow(() => validateRuntimeHelpUrls(payloads, built))
  payloads[4].payload.guide = 'https://reklawdbox.com/missing-topic-guide/'
  assert.throws(
    () => validateRuntimeHelpUrls(payloads, built),
    /src\/tools\/help_handler\.rs:1: help\(.+\): runtime-help URL is not built/,
  )
})

test('publishing audiences derive nine pairs and accept separated artifacts', () => {
  assert.equal(deriveAgentPairs(workflows).length, 9)
  assert.doesNotThrow(() =>
    validatePublishingAudiences(publishingAudienceFixture())
  )

  const tooFew = structuredClone(workflows).filter((workflow) =>
    workflow.id !== deriveAgentPairs(workflows)[0].id
  )
  assert.throws(
    () => deriveAgentPairs(tooFew),
    /expected exactly 9 agent pairs; found 8/,
  )
})

test('publishing audiences fail explicitly for every missing artifact class', () => {
  const [pair] = deriveAgentPairs(workflows)
  const cases = [
    [
      (fixture) => fixture.sourceArtifacts.delete(pair.humanSource),
      /missing human workflow source/,
    ],
    [
      (fixture) => fixture.sourceArtifacts.delete(pair.agentSource),
      /missing agent SOP source/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete(pair.humanHtml),
      /missing human workflow HTML route/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete(pair.agentHtml),
      /missing agent HTML route/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('agent/index.html'),
      /missing agent HTML route: agent\/index\.html/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('sitemap-0.xml'),
      /missing generated sitemap XML/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('llms-full.txt'),
      /missing generic full LLM bundle/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('llms-small.txt'),
      /missing generic small LLM bundle/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('_llms-txt/agent-sops.txt'),
      /missing combined agent LLM bundle/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete('llms.txt'),
      /missing LLM bundle index/,
    ],
    [
      (fixture) => fixture.builtArtifacts.delete(pair.sopText),
      /missing per-SOP agent LLM bundle/,
    ],
    [
      (fixture) => fixture.sourceArtifacts.delete('site/astro.config.mjs'),
      /missing Astro publishing config/,
    ],
    [
      (fixture) =>
        fixture.sourceArtifacts.delete(
          'site/vendor/starlight-llms-txt/llms-full.txt.ts',
        ),
      /missing vendored generic-full route/,
    ],
    [
      (fixture) =>
        fixture.sourceArtifacts.delete(
          'site/vendor/starlight-llms-txt/llms-small.txt.ts',
        ),
      /missing vendored generic-small route/,
    ],
    [
      (fixture) =>
        fixture.sourceArtifacts.delete(
          'site/vendor/starlight-llms-txt/llms-custom.txt.ts',
        ),
      /missing vendored custom-set route/,
    ],
  ]

  for (const [mutate, expected] of cases) {
    expectAudienceFailure(mutate, expected)
  }
})

test('publishing audiences reject robots and Pagefind policy drift', () => {
  const [pair] = deriveAgentPairs(workflows)
  expectAudienceFailure(
    (fixture) => fixture.builtArtifacts.set(pair.agentHtml, '<html></html>'),
    /must contain exactly one robots meta; found 0/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.agentHtml,
        '<meta name="robots" content="noindex, nofollow"><meta name="robots" content="noindex, nofollow">',
      ),
    /must contain exactly one robots meta; found 2/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.agentHtml,
        '<meta name="robots" content="index, follow">',
      ),
    /robots meta must be exactly noindex, nofollow/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.agentHtml,
        '<meta name="robots" content="noindex, nofollow"><main data-pagefind-body></main>',
      ),
    /must not contain a Pagefind body marker/,
  )
  expectAudienceFailure(
    (fixture) => fixture.builtArtifacts.set(pair.humanHtml, '<main></main>'),
    /must retain its Pagefind body marker/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        'agent/index.html',
        '<meta name="robots" content="noindex, nofollow"><h1>Ordinary docs</h1>',
      ),
    /agent\/index\.html H1 must be exactly Agent SOPs/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.agentHtml,
        '<meta name="robots" content="noindex, nofollow"><h1>Ordinary docs</h1>',
      ),
    new RegExp(
      `${
        pair.agentHtml.replaceAll('/', '\\/')
      } H1 must be exactly Agent SOP: ${pair.title}`,
    ),
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.agentHtml,
        `<meta name="robots" content="noindex, nofollow"><h1>Agent SOP: ${pair.title}</h1><h1>Duplicate</h1>`,
      ),
    /must contain exactly one H1; found 2/,
  )
})

test('publishing audiences retain human sitemap routes and exclude agents', () => {
  const [pair] = deriveAgentPairs(workflows)
  expectAudienceFailure(
    (fixture) => {
      const sitemap = fixture.builtArtifacts.get('sitemap-0.xml')
      fixture.builtArtifacts.set(
        'sitemap-0.xml',
        sitemap.replace(
          '</urlset>',
          `<url><loc>https://reklawdbox.com${pair.agentRoute}</loc></url></urlset>`,
        ),
      )
    },
    /sitemap must exclude agent route/,
  )
  expectAudienceFailure(
    (fixture) => {
      const sitemap = fixture.builtArtifacts.get('sitemap-0.xml')
      fixture.builtArtifacts.set(
        'sitemap-0.xml',
        sitemap.replace(
          `<url><loc>https://reklawdbox.com${pair.humanRoute}</loc></url>`,
          '',
        ),
      )
    },
    /sitemap must include human route/,
  )
  expectAudienceFailure(
    (fixture) => {
      const sitemap = fixture.builtArtifacts.get('sitemap-0.xml')
      fixture.builtArtifacts.set(
        'sitemap-0.xml',
        sitemap.replace(
          '<url><loc>https://reklawdbox.com/mcp-tools/</loc></url>',
          '',
        ),
      )
    },
    /sitemap must retain a representative \/mcp-tools\/ route/,
  )
})

test('publishing audiences keep generic and custom LLM sets disjoint', () => {
  const [pair] = deriveAgentPairs(workflows)
  const humanHeading = `# ${pair.title}`
  const agentHeading = `# Agent SOP: ${pair.title}`
  for (const file of ['llms-full.txt', 'llms-small.txt']) {
    expectAudienceFailure(
      (fixture) =>
        fixture.builtArtifacts.set(
          file,
          `${fixture.builtArtifacts.get(file)}${agentHeading}\n`,
        ),
      new RegExp(`${file.replace('.', '\\.')} must not contain`),
    )
    expectAudienceFailure(
      (fixture) =>
        fixture.builtArtifacts.set(
          file,
          fixture.builtArtifacts.get(file).replace(humanHeading, ''),
        ),
      /must contain .* exactly once/,
    )
    expectAudienceFailure(
      (fixture) =>
        fixture.builtArtifacts.set(
          file,
          `${fixture.builtArtifacts.get(file)}${humanHeading}\n`,
        ),
      /must contain .* exactly once/,
    )
  }
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        '_llms-txt/agent-sops.txt',
        fixture.builtArtifacts.get('_llms-txt/agent-sops.txt').replace(
          agentHeading,
          '',
        ),
      ),
    /agent-sops\.txt must contain .* exactly once/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        'llms-full.txt',
        `${fixture.builtArtifacts.get('llms-full.txt')}# Agent SOPs\n`,
      ),
    /llms-full\.txt must not contain any agent SOP heading/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        '_llms-txt/agent-sops.txt',
        fixture.builtArtifacts.get('_llms-txt/agent-sops.txt').replace(
          '# Agent SOPs',
          '',
        ),
      ),
    /agent-sops\.txt must contain # Agent SOPs exactly once/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        '_llms-txt/agent-sops.txt',
        `${
          fixture.builtArtifacts.get('_llms-txt/agent-sops.txt')
        }# Agent SOP: Unexpected Workflow\n`,
      ),
    /agent-sops\.txt must contain exactly 10 agent headings; found 11/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        '_llms-txt/agent-sops.txt',
        `${
          fixture.builtArtifacts.get('_llms-txt/agent-sops.txt')
        }${agentHeading}\n`,
      ),
    /agent-sops\.txt must contain .* exactly once/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.sopText,
        `${agentHeading}\n${agentHeading}\n`,
      ),
    /must contain exactly its own/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.sopText,
        `${fixture.builtArtifacts.get(pair.sopText)}# Agent SOPs\n`,
      ),
    /must contain exactly its own/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        'llms.txt',
        fixture.builtArtifacts.get('llms.txt').replace(`/${pair.sopText}`, ''),
      ),
    /llms\.txt must link .* exactly once/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.builtArtifacts.set(
        pair.sopText,
        '# Agent SOP: Incorrect Workflow\n',
      ),
    /must contain exactly its own/,
  )
})

test('publishing audiences enforce canonical source ownership', () => {
  const pairs = deriveAgentPairs(workflows)
  const [pair, wrongPair] = pairs
  const unpaired = workflows.find((workflow) => workflow.runtimeHelp === null)
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        pair.agentSource,
        `import SOP from '../../../partials/sops/${wrongPair.id}.mdx'\n\n<SOP />\n`,
      ),
    /imports .* instead of .*\.mdx/,
  )
  expectAudienceFailure(
    (fixture) => {
      const source = fixture.sourceArtifacts.get(pair.agentSource)
      fixture.sourceArtifacts.set(
        pair.agentSource,
        `${source}import SecondSOP from '../../../partials/sops/${pair.id}.mdx'\n<SecondSOP />\n`,
      )
    },
    /must import exactly one canonical SOP; found 2/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        pair.agentSource,
        `import SOP from '../../../partials/sops/${pair.id}.mdx'\n`,
      ),
    /must render SOP exactly once; found 0/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        pair.humanSource,
        `import SOP from '../../../partials/sops/${pair.id}.mdx'\n<SOP />\n`,
      ),
    /must not import or render its matching agent SOP partial/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        pair.humanSource,
        `import OtherSOP from '../../../partials/sops/${wrongPair.id}.mdx'\n<OtherSOP />\n`,
      ),
    new RegExp(
      `must not import or render canonical agent SOP ${wrongPair.id}`,
    ),
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        `site/src/content/docs${unpaired.route.replace(/\/$/, '')}.mdx`,
        `import OtherSOP from '../../../partials/sops/${wrongPair.id}.mdx'\n<OtherSOP />\n`,
      ),
    new RegExp(
      `must not import or render canonical agent SOP ${wrongPair.id}`,
    ),
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        pair.humanSource,
        `<${pair.sopComponent} />\n`,
      ),
    /must not import or render its matching agent SOP partial/,
  )
  const unrelated = publishingAudienceFixture()
  unrelated.sourceArtifacts.set(
    pair.humanSource,
    "import XmlSteps from '../../../partials/sops/xml-playlist-import-steps.mdx'\n<XmlSteps />\n",
  )
  assert.doesNotThrow(() => validatePublishingAudiences(unrelated))
})

test('publishing audience source wiring keeps generic exclusions isolated', () => {
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        'site/astro.config.mjs',
        fixture.sourceArtifacts.get('site/astro.config.mjs').replace(
          "!new URL(page).pathname.startsWith('/agent/')",
          "!page.includes('/agent/')",
        ),
      ),
    /sitemap filter must parse each absolute URL/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        'site/vendor/starlight-llms-txt/llms-full.txt.ts',
        'exclude: starlightLllmsTxtContext.exclude',
      ),
    /generic full route must use only excludeFull/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        'site/vendor/starlight-llms-txt/llms-small.txt.ts',
        'exclude: starlightLllmsTxtContext.excludeFull',
      ),
    /generic small route must use only exclude/,
  )
  expectAudienceFailure(
    (fixture) =>
      fixture.sourceArtifacts.set(
        'site/vendor/starlight-llms-txt/llms-custom.txt.ts',
        'exclude: starlightLllmsTxtContext.excludeFull',
      ),
    /custom-set route must not use generic exclusions/,
  )
})
