function flat(value) {
  return String(value ?? '').replaceAll('\t', ' ').replaceAll('\n', ' ')
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

export function reviewGuide(selected) {
  const lines = [
    '# Genre Intelligence Blind Review B01',
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
