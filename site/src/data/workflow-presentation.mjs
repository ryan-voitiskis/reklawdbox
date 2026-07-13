// @ts-check

const impactLabels = {
  'read-only': 'Collection read-only',
  'staged-metadata': 'Stages metadata for XML',
  'direct-audio-files': 'Writes audio-file tags or names',
  'direct-library-files': 'Writes and organizes library files',
  mixed: 'Direct file writes and staged metadata',
}

const effectLabels = {
  'audio-tags': 'Audio tags',
  'embedded-artwork': 'Embedded artwork',
  'extracted-artwork': 'Extracted artwork',
  'downloaded-artwork': 'Downloaded artwork',
  'move-rename': 'File or directory moves/renames',
  'archive-extraction': 'Archive extraction',
  'archive-move': 'Archive moves',
  'directory-create-remove': 'Directory creation/removal',
  'enrichment-cache': 'Enrichment cache',
  'audio-cache': 'Audio-analysis cache',
  'audit-state': 'Audit state',
  preset: 'Saved scoring preset',
  'timbral-normalization': 'Timbral normalization statistics',
  'provider-session': 'Provider session',
  backup: 'Rekordbox database backup',
  'metadata-xml': 'Metadata XML file',
  'playlist-xml': 'Playlist XML file',
  'artwork-file': 'Artwork file',
  'organized-library-files': 'Organized library files',
  'reload-tag': 'Reload Tag for changed imported tracks',
  'library-file-import': 'Import files or folders into Collection',
  'manual-cover-art': 'Add WAV cover art manually',
  'manual-relocate': 'Relocate files in Rekordbox',
  'import-or-delete-orphans': 'Import or remove orphan files',
  'assign-playlists': 'Assign tracks to playlists',
  'remove-duplicates': 'Review and remove duplicates',
}

const modeLabels = {
  always: 'Always',
  conditional: 'When needed',
  optional: 'Optional',
  'on-export': 'On export',
}

const networkLabels = {
  none: 'No network',
  conditional: 'Network when needed',
  required: 'Network required',
}

/**
 * @param {import('./workflows.mjs').LibraryImpact} impact
 * @returns {string}
 */
export function impactLabel(impact) {
  return impactLabels[impact]
}

/**
 * Preserve unknown effect names as readable fallbacks for forward-compatible
 * documentation builds.
 *
 * @param {string} kind
 * @returns {string}
 */
export function effectLabel(kind) {
  return effectLabels[kind] ?? kind
}

/**
 * @param {import('./workflows.mjs').EffectMode} mode
 * @returns {string}
 */
export function modeLabel(mode) {
  return modeLabels[mode]
}

/**
 * @param {import('./workflows.mjs').NetworkLevel} level
 * @returns {string}
 */
export function networkLabel(level) {
  return networkLabels[level]
}

/**
 * Describe network use without exposing provider, cache, or executor details in
 * the human-facing quick start. The full canonical condition remains available
 * in WorkflowContract.
 *
 * @param {import('./workflows.mjs').NetworkContract} network
 * @returns {string | null}
 */
export function quickStartNetworkMessage(network) {
  if (network.level === 'none') return null
  if (network.level === 'required') {
    return 'This workflow needs an online service to complete.'
  }
  return 'Online services are used only when the chosen step needs information that is not available locally.'
}

/**
 * @param {import('./workflows.mjs').Workflow} workflow
 * @returns {boolean}
 */
export function hasMaterialDirectWrite(workflow) {
  return workflow.libraryImpact === 'mixed'
    || workflow.sideEffects.directUserFiles.length > 0
}

/**
 * @param {import('./workflows.mjs').Workflow} workflow
 * @returns {boolean}
 */
export function hasXmlOutput(workflow) {
  return workflow.sideEffects.outputs.some(({ kind }) =>
    kind === 'metadata-xml' || kind === 'playlist-xml'
  )
}

/**
 * @param {import('./workflows.mjs').Workflow} workflow
 * @returns {boolean}
 */
export function hasExportFlushRisk(workflow) {
  return workflow.sideEffects.stagedMetadata.flushesExistingOnExport
}

/**
 * Summarize a canonical workflow's user-visible collection impact.
 *
 * @param {import('./workflows.mjs').Workflow} workflow
 * @returns {{ tone: 'safe' | 'review' | 'write', label: string }}
 */
export function compactSafety(workflow) {
  if (workflow.libraryImpact === 'staged-metadata') {
    return {
      tone: 'review',
      label: 'Review first · Changes require XML import',
    }
  }

  if (workflow.libraryImpact === 'direct-audio-files') {
    return {
      tone: 'write',
      label: 'Can change audio files · Approval required',
    }
  }

  if (workflow.libraryImpact === 'direct-library-files') {
    return {
      tone: 'write',
      label: 'Can tag, rename, or move files · Approval required',
    }
  }

  if (workflow.libraryImpact === 'mixed') {
    return {
      tone: 'write',
      label: 'Can change files and prepare XML · Approval required',
    }
  }

  const outputs = workflow.sideEffects.outputs
  const hasPlaylistXml = outputs.some((output) =>
    output.kind === 'playlist-xml'
  )
  const flushesStagedMetadata =
    workflow.sideEffects.stagedMetadata.flushesExistingOnExport

  if (hasPlaylistXml || flushesStagedMetadata) {
    return {
      tone: 'review',
      label: 'Read-only while planning · Optional XML export',
    }
  }

  const mayUseOnlineLookups = workflow.network.level !== 'none'
    || workflow.sideEffects.localStateWrites.length > 0

  if (workflow.kind === 'catalog' && mayUseOnlineLookups) {
    return {
      tone: 'review',
      label: 'Read-only · Some options may use online lookups',
    }
  }

  return {
    tone: 'safe',
    label: 'Read-only · No network',
  }
}
