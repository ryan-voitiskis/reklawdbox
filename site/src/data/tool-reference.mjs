export const toolGroups = [
  {
    id: 'library-data',
    title: 'Library & Data',
    route: '/mcp-tools/library-data/',
  },
  {
    id: 'enrichment-analysis',
    title: 'Enrichment & Analysis',
    route: '/mcp-tools/enrichment-analysis/',
  },
  {
    id: 'classification-staging',
    title: 'Classification & Staging',
    route: '/mcp-tools/classification-staging/',
  },
  {
    id: 'mixing',
    title: 'Mixing & Sequencing',
    route: '/mcp-tools/mixing/',
  },
  {
    id: 'files-system',
    title: 'Files & System',
    route: '/mcp-tools/files-system/',
  },
]

const groupTools = {
  'library-data': [
    'read_library',
    'search_tracks',
    'get_track',
    'get_playlists',
    'get_playlist_tracks',
    'get_sessions',
    'get_session_tracks',
    'get_play_stats',
    'resolve_track_data',
    'resolve_tracks_data',
    'cache_coverage',
  ],
  'enrichment-analysis': [
    'lookup_discogs',
    'lookup_beatport',
    'lookup_musicbrainz',
    'lookup_bandcamp',
    'enrich_tracks',
    'analyze_track_audio',
    'analyze_audio_batch',
    'setup_essentia',
  ],
  'classification-staging': [
    'get_genre_taxonomy',
    'suggest_normalizations',
    'classify_tracks',
    'audit_genres',
    'calibration_coverage',
    'calibrate_audio_profiles',
    'backfill_labels',
    'backfill_years',
    'backfill_albums',
    'update_tracks',
    'preview_changes',
    'write_xml',
    'clear_changes',
  ],
  mixing: [
    'score_transition',
    'query_transition_candidates',
    'build_set',
    'score_pool_compatibility',
    'expand_pool',
    'describe_pool',
    'discover_pools',
    'save_weight_preset',
    'list_weight_presets',
    'delete_weight_preset',
  ],
  'files-system': [
    'read_file_tags',
    'write_file_tags',
    'extract_cover_art',
    'embed_cover_art',
    'scan_broken_links',
    'scan_orphan_files',
    'scan_playlist_coverage',
    'scan_duplicates',
    'audit_state',
    'clear_caches',
    'help',
  ],
}

export const toolReferences = toolGroups.flatMap((group) =>
  groupTools[group.id].map((name) => ({
    name,
    group: group.id,
    route: group.route,
  }))
)

export const toolCounts = Object.freeze(
  Object.fromEntries(
    toolGroups.map((group) => [
      group.id,
      toolReferences.filter((tool) => tool.group === group.id).length,
    ]),
  ),
)

export function validateToolReferences(items) {
  if (!Array.isArray(items)) throw new Error('tool references must be an array')

  const groups = new Map(toolGroups.map((group) => [group.id, group]))
  const seen = new Set()
  for (const [index, item] of items.entries()) {
    const path = `toolReferences[${index}]`
    if (!item || typeof item !== 'object') {
      throw new Error(`${path} must be an object`)
    }
    if (!/^[a-z][a-z0-9_]*$/.test(item.name ?? '')) {
      throw new Error(`${path}.name must be a snake_case tool name`)
    }
    if (seen.has(item.name)) {
      throw new Error(`duplicate tool mapping: ${item.name}`)
    }
    seen.add(item.name)

    const group = groups.get(item.group)
    if (!group) throw new Error(`${path}.group is unknown: ${item.group}`)
    if (item.route !== group.route) {
      throw new Error(`${path}.route must be ${group.route}`)
    }
    if (!/^\/mcp-tools\/[a-z0-9-]+\/$/.test(item.route)) {
      throw new Error(`${path}.route is not a legal MCP reference route`)
    }
  }

  return items
}

validateToolReferences(toolReferences)
