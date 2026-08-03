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
