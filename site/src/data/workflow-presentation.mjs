// @ts-check

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
