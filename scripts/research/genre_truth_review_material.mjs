function flat(value) {
  return String(value ?? '').replaceAll('\t', ' ').replaceAll('\n', ' ')
}

export function validateReviewRoster(selected, maximumTracksPerArtist = 1) {
  if (
    !Number.isInteger(maximumTracksPerArtist)
    || maximumTracksPerArtist < 1
    || maximumTracksPerArtist > 20
  ) {
    throw new Error('maximum tracks per artist must be an integer from 1 to 20')
  }
  const artistCounts = new Map()
  for (const row of selected) {
    artistCounts.set(
      row.artist_group,
      (artistCounts.get(row.artist_group) ?? 0) + 1,
    )
  }
  if (
    selected.length < 1
    || selected.length > 20
    || new Set(selected.map((row) => row.track_id)).size !== selected.length
    || new Set(selected.map((row) => row.file_path)).size !== selected.length
    || Math.max(...artistCounts.values()) > maximumTracksPerArtist
    || new Set(selected.map((row) => row.release_group)).size
      !== selected.length
  ) {
    throw new Error(
      'truth-review roster violates its size or identity-diversity constraints',
    )
  }
}

export function reviewSheet(selected) {
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
      row.artist,
      row.title,
      '',
      '',
      '',
      '',
    ].map(flat).join('\t')
  )
  return `${[header, ...rows].join('\n')}\n`
}

export function reviewGuide(selected, batchLabel = 'B01') {
  const lines = [
    `# Genre Intelligence Blind Review ${batchLabel}`,
    '',
    'Classify each track by ear at the broad parent-genre level. Hidden sampling labels and model outputs are deliberately absent.',
    '',
    'Use one canonical genre with high, medium, or low confidence; `ambiguous` with plausible alternatives; or `skip`. Optional notes can describe meter, kick pattern, tempo feel, groove, bass movement, timbre, density, arrangement, vocals, or scene associations.',
    '',
  ]
  for (const row of selected) {
    lines.push(
      `## ${row.code}: ${row.artist} – ${row.title}`,
      '',
      'Verdict:',
      '',
    )
  }
  return `${lines.join('\n')}\n`
}

const BLIND_XML_TRACK_ATTRIBUTES = [
  'TrackID',
  'Name',
  'Artist',
  'Album',
  'Location',
]

export function blindReviewXml(xml) {
  let collectionTracks = 0
  const scrubbed = xml.replace(
    /^(\s*)<TRACK ([^>]*)\/>$/gm,
    (line, indentation, attributeText) => {
      const attributes = new Map()
      const pattern = /([A-Za-z][A-Za-z0-9]*)="([^"]*)"/g
      let cursor = 0
      for (
        const match of attributeText.matchAll(pattern)
      ) {
        if (attributeText.slice(cursor, match.index).trim() !== '') {
          throw new Error('collection track contains an unparsed XML attribute')
        }
        attributes.set(match[1], match[2])
        cursor = match.index + match[0].length
      }
      if (attributes.has('Key')) return line
      for (const name of BLIND_XML_TRACK_ATTRIBUTES) {
        if (!attributes.has(name)) {
          throw new Error(`collection track is missing ${name}`)
        }
      }
      if (attributeText.slice(cursor).trim() !== '') {
        throw new Error('collection track contains an unparsed XML attribute')
      }
      collectionTracks += 1
      const retained = BLIND_XML_TRACK_ATTRIBUTES.map((name) =>
        `${name}="${attributes.get(name)}"`
      ).join(' ')
      return `${indentation}<TRACK ${retained}/>`
    },
  )
  if (collectionTracks === 0) {
    throw new Error('blind-review XML contains no collection tracks')
  }
  return scrubbed
}
