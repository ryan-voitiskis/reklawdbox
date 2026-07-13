// @ts-check

/**
 * Canonical workflow vocabulary.
 *
 * Collection impact describes effects on the user's Rekordbox/audio collection.
 * Local cache/state writes and exported files are disclosed separately so a
 * collection-read-only workflow is not misrepresented as process-read-only.
 *
 * @typedef {'workflow' | 'catalog'} WorkflowKind
 * @typedef {'read-only' | 'staged-metadata' | 'direct-audio-files' | 'direct-library-files' | 'mixed'} LibraryImpact
 * @typedef {'always' | 'conditional' | 'optional' | 'on-export'} EffectMode
 * @typedef {'audio-tags' | 'embedded-artwork' | 'extracted-artwork' | 'downloaded-artwork' | 'move-rename' | 'archive-extraction' | 'archive-move' | 'directory-create-remove'} DirectUserFileKind
 * @typedef {'enrichment-cache' | 'audio-cache' | 'audit-state' | 'preset' | 'timbral-normalization' | 'provider-session'} LocalStateKind
 * @typedef {'backup' | 'metadata-xml' | 'playlist-xml' | 'artwork-file' | 'organized-library-files'} OutputKind
 * @typedef {'reload-tag' | 'metadata-xml' | 'playlist-xml' | 'library-file-import' | 'manual-cover-art' | 'manual-relocate' | 'import-or-delete-orphans' | 'assign-playlists' | 'remove-duplicates'} HandoffKind
 * @typedef {'none' | 'conditional' | 'required'} NetworkLevel
 *
 * @typedef {{ kind: DirectUserFileKind | LocalStateKind | OutputKind | HandoffKind, mode: EffectMode, condition?: string }} EffectEntry
 * @typedef {{ level: NetworkLevel, condition?: string, reason?: string }} NetworkContract
 * @typedef {{ topic: string, menuOrder: number, recommendedOrder: number | null }} RuntimeHelp
 * @typedef {{
 *   id: string,
 *   title: string,
 *   summary: string,
 *   network: NetworkContract,
 *   localStateWrites: EffectEntry[],
 *   prerequisites: string[],
 *   duration: string,
 *   output: string,
 * }} WorkflowVariant
 * @typedef {{
 *   id: string,
 *   title: string,
 *   route: string,
 *   summary: string,
 *   audience: string,
 *   chooseWhen: string,
 *   kind: WorkflowKind,
 *   goals: string[],
 *   libraryImpact: LibraryImpact,
 *   sideEffects: {
 *     stagedMetadata: { creates: boolean, flushesExistingOnExport: boolean },
 *     directUserFiles: EffectEntry[],
 *     localStateWrites: EffectEntry[],
 *     outputs: EffectEntry[],
 *   },
 *   network: NetworkContract,
 *   runtimeHelp: RuntimeHelp | null,
 *   prerequisites: string[],
 *   scope: string,
 *   duration: string,
 *   resumability: string,
 *   approval: string[],
 *   recovery: string[],
 *   output: string,
 *   rekordboxHandoff: EffectEntry[],
 *   variants: WorkflowVariant[],
 * }} Workflow
 */

const WORKFLOW_IDS = [
  'library-cleanup',
  'collection-audit',
  'metadata-backfill',
  'genre-classification',
  'genre-audit',
  'set-building',
  'pool-building',
  'chapter-set-planning',
  'batch-import',
  'library-health',
  'dj-prompts',
]

const GOAL_BLUEPRINTS = [
  {
    id: 'inspect-health',
    title: 'Check my library for problems',
    summary:
      'Find missing files, untracked music, playlist gaps, and duplicates.',
    workflows: ['library-health'],
  },
  {
    id: 'clean-library',
    title: 'Fix missing or messy track information',
    summary:
      'Choose a full cleanup, fix file tags, or fill missing Rekordbox fields.',
    workflows: ['library-cleanup', 'collection-audit', 'metadata-backfill'],
  },
  {
    id: 'prepare-downloads',
    title: 'Prepare new downloads',
    summary: 'Tag and organise new music before importing it into Rekordbox.',
    workflows: ['batch-import'],
  },
  {
    id: 'classify-genres',
    title: 'Improve genre tags',
    summary: 'Add missing genres or check whether existing tags still fit.',
    workflows: ['genre-classification', 'genre-audit'],
  },
  {
    id: 'build-for-mixing',
    title: 'Build a set or crate',
    summary: 'Order a set, grow a flexible crate, or connect several chapters.',
    workflows: ['set-building', 'pool-building', 'chapter-set-planning'],
  },
  {
    id: 'explore-dj-ideas',
    title: 'Plan a DJ session',
    summary:
      'Get help preparing a gig, digging, practising, or reviewing a set.',
    workflows: ['dj-prompts'],
  },
]

export const goalDefinitions = GOAL_BLUEPRINTS.map(
  ({ id, title, summary }) => ({ id, title, summary }),
)

const DJ_VARIANTS = [
  ['gig-prep', 'Gig Prep'],
  ['collection-gap-analysis', 'Collection Gap Analysis'],
  ['dig-session-partner', 'Dig Session Partner'],
  ['post-gig-debrief', 'Post-Gig Debrief'],
  ['harmonic-journey-planning', 'Harmonic Journey Planning'],
  ['practice-session-design', 'Practice Session Design'],
]

const RUNTIME_HELP = new Map([
  ['collection-audit', ['audit', 1, 1]],
  ['metadata-backfill', ['metadata', 2, 2]],
  ['genre-classification', ['genre', 3, 3]],
  ['genre-audit', ['genre audit', 4, 4]],
  ['set-building', ['set', 5, 5]],
  ['pool-building', ['pool', 6, 6]],
  ['chapter-set-planning', ['chapter', 7, 7]],
  ['batch-import', ['import', 8, null]],
  ['library-health', ['health', 9, null]],
])

export const XML_BACKUP_SUCCESS_CONDITION =
  'XML export proceeds only after the built-in backup succeeds or the configured custom script exits zero'

const backupOnExport = {
  kind: 'backup',
  mode: 'on-export',
  condition: XML_BACKUP_SUCCESS_CONDITION,
}

/** @type {Workflow[]} */
export const workflows = [
  {
    id: 'library-cleanup',
    title: 'Library Cleanup',
    route: '/workflows/library-cleanup/',
    summary:
      'Clean file metadata, hydrate evidence, backfill metadata, then classify and audit genres in an ordered five-session path.',
    audience:
      'New users and anyone preparing a disorganized library for reliable enrichment and DJ workflows.',
    chooseWhen: 'You want one guided path through a generally messy library.',
    kind: 'workflow',
    goals: ['clean-library'],
    libraryImpact: 'mixed',
    sideEffects: {
      stagedMetadata: {
        creates: true,
        flushesExistingOnExport: false,
      },
      directUserFiles: [
        {
          kind: 'audio-tags',
          mode: 'conditional',
          condition: 'Session 1 finds and the user approves tag fixes.',
        },
        {
          kind: 'move-rename',
          mode: 'conditional',
          condition:
            'Session 1 finds naming issues on files that are not already imported into Rekordbox.',
        },
      ],
      localStateWrites: [
        {
          kind: 'audit-state',
          mode: 'conditional',
          condition:
            'Session 1 is run; users with already-clean metadata can skip it.',
        },
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition:
            'Selected Discogs, Beatport, Bandcamp, or MusicBrainz evidence is missing.',
        },
        {
          kind: 'audio-cache',
          mode: 'conditional',
          condition:
            'Selected Stratum or optional Essentia analysis is missing.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required for uncached enrichment.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'metadata-xml',
          mode: 'on-export',
          condition: 'Sessions 3–5 have approved staged metadata to export.',
        },
      ],
    },
    network: {
      level: 'conditional',
      condition:
        'Hydration or gap research needs provider data that is not already cached; local audit and classification passes are cache-first.',
    },
    runtimeHelp: null,
    prerequisites: [
      'reklawdbox connected to the intended Rekordbox library.',
      'A file backup before direct tag or rename work.',
      'Enough time to complete and verify each session checkpoint before moving on.',
    ],
    scope:
      'Usually the full collection, with individual audit, hydration, and review passes scoped when needed.',
    duration:
      'Several sessions; preparing a large library can run overnight and review time depends on library size.',
    resumability:
      'Session boundaries are checkpoints and caches/audit state persist. Bounded enrichment/audio preparation advances through page.next_offset while page.has_more, records failures, and retries failed explicit track IDs after traversal; restart at offset zero after selector/library-order changes, enrichment provider/cache-policy changes, or changed audio skip_cached/Essentia availability. Staged metadata is in memory, so export it before restarting the MCP host.',
    approval: [
      'Approve direct file fixes and complete the scoped Reload Tag checkpoint after Session 1.',
      'Review conflicts and low-confidence recommendations before staging them.',
      'Approve each XML export and verify its Rekordbox import before starting the next session.',
    ],
    recovery: [
      'Re-run the audit or hydration pass after correcting unreadable files, provider errors, or incomplete coverage.',
      'Clear and re-stage unwanted in-memory metadata before export.',
      'Keep each XML file until its imported values are verified; direct audio-file changes require the user’s own file backup to undo.',
    ],
    output:
      'A cleaner, enriched, genre-ready library plus approved metadata XML files for manual Rekordbox import.',
    rekordboxHandoff: [
      {
        kind: 'reload-tag',
        mode: 'conditional',
        condition:
          'After successful Session 1 tag changes to files already imported into Rekordbox; complete this before enrichment.',
      },
      {
        kind: 'metadata-xml',
        mode: 'on-export',
        condition: 'After each approved staged-metadata session.',
      },
    ],
    variants: [],
  },
  {
    id: 'collection-audit',
    title: 'Collection Audit',
    route: '/workflows/collection-audit/',
    summary:
      'Find naming and tag problems, approve safe fixes, verify file writes, and synchronize changed imported tracks.',
    audience:
      'Users cleaning artist, title, tag-layer, filename, and directory conventions.',
    chooseWhen:
      'File names or embedded artist and title tags are inconsistent.',
    kind: 'workflow',
    goals: ['clean-library'],
    libraryImpact: 'direct-audio-files',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: false,
      },
      directUserFiles: [
        {
          kind: 'audio-tags',
          mode: 'conditional',
          condition:
            'The scan finds a tag issue and the user approves the proposed write.',
        },
        {
          kind: 'move-rename',
          mode: 'conditional',
          condition:
            'The scan finds a naming issue on a file not already imported into Rekordbox and the user approves the rename.',
        },
      ],
      localStateWrites: [
        { kind: 'audit-state', mode: 'always' },
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition: 'A review-tier issue needs an uncached provider lookup.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required for an uncached lookup.',
        },
      ],
      outputs: [],
    },
    network: {
      level: 'conditional',
      condition:
        'Only ambiguous review-tier issues that need provider lookup; scanning and file-tag operations are local.',
    },
    runtimeHelp: { topic: 'audit', menuOrder: 1, recommendedOrder: 1 },
    prerequisites: [
      'reklawdbox connected to the intended library.',
      'A confirmed scan scope; the full collection is the default.',
      'A file backup before approving tag or rename changes.',
    ],
    scope:
      'Full collection by default, or a user-selected directory or subset.',
    duration:
      'Scope-dependent; review volume and file-read failures determine the work.',
    resumability:
      'Audit issues and resolutions persist in local state. Direct file changes are durable; re-scan after resuming before declaring the audit clean.',
    approval: [
      'Approve the fix plan before any file write.',
      'Choose how to resolve review-tier issues and ambiguous metadata.',
      'Confirm the scoped Rekordbox Reload Tag checkpoint before the final report.',
    ],
    recovery: [
      'Retry unreadable files and re-scan; persistent failures mean the audit remains incomplete.',
      'Defer imported-file renames and use Rekordbox relocation instead of changing paths behind Rekordbox.',
      'Use the file backup to undo direct tag or rename changes, then re-scan.',
    ],
    output:
      'A persisted audit report, verified file fixes, and a list of intentionally deferred issues.',
    rekordboxHandoff: [
      {
        kind: 'reload-tag',
        mode: 'conditional',
        condition:
          'Successful tag changes affected files already imported into Rekordbox.',
      },
      {
        kind: 'manual-relocate',
        mode: 'conditional',
        condition: 'An imported file needs a path or filename change.',
      },
    ],
    variants: [],
  },
  {
    id: 'metadata-backfill',
    title: 'Metadata Backfill',
    route: '/workflows/metadata-backfill/',
    summary:
      'Fill missing labels, years, and albums from file, folder, cached provider, and researched evidence.',
    audience:
      'Users preparing missing metadata for stronger search and genre-classification evidence.',
    chooseWhen: 'Rekordbox is missing labels, years, or albums.',
    kind: 'workflow',
    goals: ['clean-library'],
    libraryImpact: 'staged-metadata',
    sideEffects: {
      stagedMetadata: {
        creates: true,
        flushesExistingOnExport: false,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition:
            'Targeted or automatic enrichment and lookup work is needed for remaining gaps.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required for uncached label or year research.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'metadata-xml',
          mode: 'on-export',
          condition:
            'The user approves the staged label, year, or album changes.',
        },
      ],
    },
    network: {
      level: 'conditional',
      condition:
        'Cached evidence is incomplete, auto_enrich is enabled, or remaining label/year gaps require provider or web research.',
    },
    runtimeHelp: { topic: 'metadata', menuOrder: 2, recommendedOrder: 2 },
    prerequisites: [
      'A scoped cache_coverage baseline for the tracks with missing metadata.',
      'Completed Discogs and Beatport searches where relevant; a cached no-match counts as searched.',
      'Time to research unresolved label and year gaps before bypassing the export gate.',
    ],
    scope:
      'The collection’s missing labels, year-zero tracks, and empty albums; gap research is paginated manually.',
    duration:
      'Scope-dependent; automatic passes are followed by conflict review and gap research.',
    resumability:
      'Provider caches persist. Bounded enrichment advances through page.next_offset while page.has_more, then retries failed explicit track IDs; restart at zero if selector order, providers, skip_cached, or force_refresh changes. Run the label-mutating pass once and fetch later conflicts with dry_run=true, auto_enrich=false, and conflict_page.next_offset. Staged metadata and the research gate belong to the current MCP process, so export before restarting or re-run the staging passes.',
    approval: [
      'Resolve label conflicts and approve researched labels or uncertain years.',
      'Acknowledge that every remaining label and year gap was genuinely researched before bypassing the label gate.',
      'Review preview_changes and approve the XML export.',
    ],
    recovery: [
      'Retry actual provider errors, accept legitimate no-matches, and re-run the relevant backfill after new evidence is cached.',
      'Clear unwanted staged fields or tracks and re-run preview_changes.',
      'A failed XML write keeps staged changes available for retry; keep a successful XML until Rekordbox import is verified.',
    ],
    output:
      'Approved label, year, and album changes in a Rekordbox-compatible metadata XML file.',
    rekordboxHandoff: [
      {
        kind: 'metadata-xml',
        mode: 'on-export',
        condition: 'After the research gate and final preview are approved.',
      },
    ],
    variants: [],
  },
  {
    id: 'genre-classification',
    title: 'Genre Classification',
    route: '/workflows/genre-classification/',
    summary:
      'Classify ungenred tracks from cached provider, label, BPM, and audio evidence with confidence-based review.',
    audience:
      'Users assigning consistent DJ-oriented genres to ungenred or noncanonical tracks.',
    chooseWhen: 'Tracks have no useful genre tag.',
    kind: 'workflow',
    goals: ['classify-genres'],
    libraryImpact: 'staged-metadata',
    sideEffects: {
      stagedMetadata: {
        creates: true,
        flushesExistingOnExport: false,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition:
            'Preparation fills missing Discogs or Beatport searches before cache-only classification.',
        },
        {
          kind: 'audio-cache',
          mode: 'conditional',
          condition:
            'Preparation fills missing Stratum or optional Essentia analysis.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required while filling preparation gaps.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'metadata-xml',
          mode: 'on-export',
          condition: 'The user approves staged genre changes.',
        },
      ],
    },
    network: {
      level: 'conditional',
      condition:
        'Only preparation for missing provider evidence; classify_tracks itself is cache-only.',
    },
    runtimeHelp: { topic: 'genre', menuOrder: 3, recommendedOrder: 3 },
    prerequisites: [
      'A scoped cache_coverage baseline for ungenred tracks.',
      'Core Discogs, Beatport, and Stratum gaps filled where practical; incomplete evidence is allowed.',
      'A user-selected confidence approval policy.',
    ],
    scope:
      'Ungenred and noncanonical-genre tracks, processed with explicit max_tracks and offset pagination.',
    duration:
      'Scope-dependent; low and insufficient confidence tracks require individual review.',
    resumability:
      'Enrichment/audio preparation traverses page.next_offset while page.has_more, records failures, and retries failed explicit track IDs after the scope; restart after selector/library-order changes, enrichment provider/cache-policy changes, or changed audio skip_cached/Essentia availability. Classification keeps its separate caller-managed offset. Caches persist, but staged genres are lost when the MCP process restarts unless exported.',
    approval: [
      'Choose which confidence tiers, if any, may be staged in bulk.',
      'Review low and insufficient confidence tracks individually.',
      'Review the aggregate staged result and approve XML export.',
    ],
    recovery: [
      'Fill evidence gaps or adjust genre overrides, then re-run the affected page of classification.',
      'Clear unwanted staged genres before export.',
      'A failed XML write keeps staged changes available for retry; keep the exported XML until import is verified.',
    ],
    output:
      'Approved genre assignments in a Rekordbox-compatible metadata XML file.',
    rekordboxHandoff: [
      {
        kind: 'metadata-xml',
        mode: 'on-export',
        condition: 'After all approved genre changes are previewed.',
      },
    ],
    variants: [],
  },
  {
    id: 'genre-audit',
    title: 'Genre Audit',
    route: '/workflows/genre-audit/',
    summary:
      'Compare existing genres with cached evidence and review only conflicts or uncertain classifications.',
    audience:
      'Users verifying existing genre tags after classification, imports, or taxonomy changes.',
    chooseWhen: 'Genre tags exist, but you want to check them.',
    kind: 'workflow',
    goals: ['classify-genres'],
    libraryImpact: 'staged-metadata',
    sideEffects: {
      stagedMetadata: {
        creates: true,
        flushesExistingOnExport: false,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition:
            'Preparation fills missing Discogs or Beatport searches before the cache-only audit.',
        },
        {
          kind: 'audio-cache',
          mode: 'conditional',
          condition:
            'Preparation fills missing Stratum or optional Essentia analysis.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required while filling preparation gaps.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'metadata-xml',
          mode: 'on-export',
          condition: 'The user approves staged genre corrections.',
        },
      ],
    },
    network: {
      level: 'conditional',
      condition:
        'Only preparation for missing evidence; audit_genres itself is cache-only.',
    },
    runtimeHelp: { topic: 'genre audit', menuOrder: 4, recommendedOrder: 4 },
    prerequisites: [
      'Existing genre tags to audit.',
      'A scoped cache_coverage baseline for genred tracks.',
      'Core evidence gaps filled where practical; incomplete evidence is allowed but needs individual review.',
    ],
    scope:
      'Genred tracks, paginated with caller-managed max_tracks and offset values.',
    duration:
      'Scope-dependent; only conflicts and manual-review tracks are presented.',
    resumability:
      'Enrichment/audio preparation traverses page.next_offset while page.has_more, then retries failed explicit track IDs; restart after selector/library-order changes, enrichment provider/cache-policy changes, or changed audio skip_cached/Essentia availability. Audit pagination keeps its separate caller-managed offset. Caches persist, but staged corrections must be exported before the MCP process restarts.',
    approval: [
      'Approve or reject each conflict group; low-confidence items require individual judgment.',
      'Review preview_changes and approve XML export.',
    ],
    recovery: [
      'Gather more evidence and re-run the affected page when a conflict is ambiguous.',
      'Clear unwanted staged corrections before export.',
      'A failed XML write keeps staged changes available for retry; keep the exported XML until import is verified.',
    ],
    output:
      'Approved corrections to existing genre tags in a Rekordbox-compatible metadata XML file.',
    rekordboxHandoff: [
      {
        kind: 'metadata-xml',
        mode: 'on-export',
        condition: 'After all approved audit corrections are previewed.',
      },
    ],
    variants: [],
  },
  {
    id: 'set-building',
    title: 'Set Building',
    route: '/workflows/set-building/',
    summary:
      'Compare transition-scored ordered set candidates, refine one with the user, and optionally export the approved playlist.',
    audience:
      'DJs planning a fixed sequence with a chosen duration, energy arc, and transition priorities.',
    chooseWhen: 'You have candidate tracks and want one ordered set.',
    kind: 'workflow',
    goals: ['build-for-mixing'],
    libraryImpact: 'read-only',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: true,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'preset',
          mode: 'optional',
          condition:
            'The user chooses to save custom transition weights for later sessions.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'playlist-xml',
          mode: 'on-export',
          condition: 'The user finalizes a set and approves export.',
        },
      ],
    },
    network: {
      level: 'none',
      reason:
        'Search and scoring use Rekordbox metadata plus local cached analysis.',
    },
    runtimeHelp: { topic: 'set', menuOrder: 5, recommendedOrder: 5 },
    prerequisites: [
      'A resolvable candidate pool and valid build_set parameters.',
      'A user-defined duration, scope, and energy/priority preferences.',
      'BPM, key, genre, and optional analysis improve scoring but missing axes degrade rather than block.',
    ],
    scope:
      'A selected candidate pool and requested target length, refined interactively.',
    duration:
      'Scope-dependent; driven by requested set length and refinement rounds.',
    resumability:
      'Candidate and edit state lives in the conversation, not a durable workflow cursor. Rebuild after restart; saved custom presets and exported XML persist.',
    approval: [
      'Confirm parameters and candidate-pool scope.',
      'Choose and refine a candidate ordering.',
      'Preview unrelated staged metadata, then approve playlist export.',
    ],
    recovery: [
      'Adjust filters, weights, or pinned tracks and regenerate candidates.',
      'If export fails, retry; staged metadata taken for the export is restored on failure.',
      'Keep the XML until the imported playlist name, count, endpoints, and representative order are verified.',
    ],
    output:
      'An approved ordered set plan, with an optional playlist XML file for Rekordbox.',
    rekordboxHandoff: [
      {
        kind: 'playlist-xml',
        mode: 'on-export',
        condition: 'The approved set is exported.',
      },
      {
        kind: 'metadata-xml',
        mode: 'conditional',
        condition:
          'The playlist export also includes unrelated metadata that was already staged.',
      },
    ],
    variants: [],
  },
  {
    id: 'pool-building',
    title: 'Pool Building',
    route: '/workflows/pool-building/',
    summary:
      'Grow and inspect a symmetric compatibility pool for live improvisation, then optionally export it as a playlist.',
    audience:
      'DJs building flexible crates whose tracks should mix well in any order.',
    chooseWhen: 'You want a flexible crate whose tracks mix well together.',
    kind: 'workflow',
    goals: ['build-for-mixing'],
    libraryImpact: 'read-only',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: true,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'timbral-normalization',
          mode: 'conditional',
          condition:
            'Pool scoring has enough Essentia vectors and the derived statistics need to be created, refreshed, or cleared.',
        },
        {
          kind: 'preset',
          mode: 'optional',
          condition:
            'The user chooses to save custom pool weights for later sessions.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'playlist-xml',
          mode: 'on-export',
          condition: 'The user locks a pool and approves export.',
        },
      ],
    },
    network: {
      level: 'none',
      reason:
        'Discovery and scoring use Rekordbox metadata plus local cached analysis.',
    },
    runtimeHelp: { topic: 'pool', menuOrder: 6, recommendedOrder: 6 },
    prerequisites: [
      'Valid seed or candidate tracks and valid pool-tool parameters.',
      'A confirmed master-tempo setting.',
      'BPM, key, genre, and optional analysis improve scoring but missing axes degrade rather than block.',
    ],
    scope:
      'A selected seed set or filtered library subset, expanded in user-approved rounds.',
    duration:
      'Scope-dependent; driven by pool size, discovery rounds, and review.',
    resumability:
      'The current pool lives in the conversation, not a durable workflow cursor. Re-submit approved members after restart; presets and exported XML persist.',
    approval: [
      'Confirm seeds, master-tempo behavior, and discovery scope.',
      'Approve additions and any weak-member removals.',
      'Preview unrelated staged metadata, then approve playlist export and confirm import before calling the pool locked.',
    ],
    recovery: [
      'Widen filters, change seeds, or relax the scoring goal if expansion stops early.',
      'If export fails, retry; staged metadata taken for the export is restored on failure.',
      'Keep the XML until the imported playlist is verified.',
    ],
    output:
      'An approved improvisation pool, with an optional playlist XML file for Rekordbox.',
    rekordboxHandoff: [
      {
        kind: 'playlist-xml',
        mode: 'on-export',
        condition: 'The approved pool is exported.',
      },
      {
        kind: 'metadata-xml',
        mode: 'conditional',
        condition:
          'The playlist export also includes unrelated metadata that was already staged.',
      },
    ],
    variants: [],
  },
  {
    id: 'chapter-set-planning',
    title: 'Chapter Set Planning',
    route: '/workflows/chapter-set-planning/',
    summary:
      'Order locked chapters, find bridge tracks, refine internal sequences, and optionally export the full plan.',
    audience:
      'DJs planning a longer performance from multiple approved pools or chapters.',
    chooseWhen:
      'You already have several chapters or pools and need a full-night plan.',
    kind: 'workflow',
    goals: ['build-for-mixing'],
    libraryImpact: 'read-only',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: true,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'timbral-normalization',
          mode: 'conditional',
          condition:
            'Pool or bridge scoring has enough Essentia vectors and the derived statistics need to be created, refreshed, or cleared.',
        },
        {
          kind: 'preset',
          mode: 'optional',
          condition:
            'The user chooses to save custom pool or transition weights.',
        },
      ],
      outputs: [
        backupOnExport,
        {
          kind: 'playlist-xml',
          mode: 'on-export',
          condition: 'The user approves the complete set plan for export.',
        },
      ],
    },
    network: {
      level: 'none',
      reason:
        'Chapter, bridge, and sequence scoring use Rekordbox metadata plus local cached analysis.',
    },
    runtimeHelp: { topic: 'chapter', menuOrder: 7, recommendedOrder: 7 },
    prerequisites: [
      'At least two locked chapter playlists.',
      'A sequenced or unordered choice for every chapter.',
      'Valid bridge candidates and pool/set parameters; missing scoring axes degrade rather than block.',
    ],
    scope:
      'The selected chapters, every boundary between them, and optional internal sequencing.',
    duration:
      'Scope-dependent; driven by chapter count, bridge searches, and refinement.',
    resumability:
      'The plan lives in the conversation, not a durable workflow cursor. Chapter playlists persist, but ordering and bridge choices must be reconstructed after restart.',
    approval: [
      'Approve chapter order, bridge choices, and any proposed internal sequences.',
      'Approve the complete plan and any optional per-chapter exports.',
      'Preview unrelated staged metadata, then confirm the Rekordbox playlist import.',
    ],
    recovery: [
      'Change chapter order, boundary tracks, or scoring parameters and re-run the affected bridge search.',
      'If export fails, retry; staged metadata taken for the export is restored on failure.',
      'Keep the XML until every imported playlist’s name, count, and order are verified.',
    ],
    output:
      'An approved chapter-and-bridge set plan, with an optional playlist XML file for Rekordbox.',
    rekordboxHandoff: [
      {
        kind: 'playlist-xml',
        mode: 'on-export',
        condition:
          'The approved full plan or optional chapter playlists are exported.',
      },
      {
        kind: 'metadata-xml',
        mode: 'conditional',
        condition:
          'The playlist export also includes unrelated metadata that was already staged.',
      },
    ],
    variants: [],
  },
  {
    id: 'batch-import',
    title: 'Batch Import',
    route: '/workflows/batch-import/',
    summary:
      'Extract, identify, tag, rename, add artwork to, and organize newly acquired music before Rekordbox import.',
    audience:
      'Users preparing downloads, albums, loose tracks, or ZIP archives for a consistent library layout.',
    chooseWhen: 'You have new downloads to tag and organise before importing.',
    kind: 'workflow',
    goals: ['prepare-downloads'],
    libraryImpact: 'direct-library-files',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: false,
      },
      directUserFiles: [
        { kind: 'audio-tags', mode: 'always' },
        {
          kind: 'embedded-artwork',
          mode: 'conditional',
          condition: 'A local or remote cover can be identified.',
        },
        {
          kind: 'downloaded-artwork',
          mode: 'conditional',
          condition:
            'No suitable local cover exists and a provider returns an artwork URL.',
        },
        { kind: 'move-rename', mode: 'always' },
        {
          kind: 'archive-extraction',
          mode: 'conditional',
          condition: 'The incoming batch contains an unprocessed ZIP archive.',
        },
        {
          kind: 'archive-move',
          mode: 'conditional',
          condition:
            'The incoming batch contains a processed or failed ZIP archive.',
        },
        { kind: 'directory-create-remove', mode: 'always' },
      ],
      localStateWrites: [
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition: 'Metadata or artwork lookup is not already cached.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'Discogs authentication is required for an uncached lookup.',
        },
      ],
      outputs: [
        { kind: 'organized-library-files', mode: 'always' },
        {
          kind: 'artwork-file',
          mode: 'conditional',
          condition:
            'Downloaded artwork is kept, including the manual-import cover beside a WAV file.',
        },
      ],
    },
    network: {
      level: 'conditional',
      condition:
        'Uncached metadata/artwork lookup or cover download is needed.',
    },
    runtimeHelp: { topic: 'import', menuOrder: 8, recommendedOrder: null },
    prerequisites: [
      'A confirmed incoming batch path and destination library convention.',
      'Permission to write tags, create directories, rename/move files, and extract archives in those locations.',
      'User input for ambiguous artists, releases, years, labels, filenames, or artwork.',
    ],
    scope:
      'One incoming batch, processed album by album and then as loose tracks.',
    duration:
      'Scope-dependent; batch size, archive contents, lookup gaps, and ambiguity determine the work.',
    resumability:
      'Completed file operations are durable but there is no transactional cursor. Resume from the last verified album or phase after re-reading tags and paths.',
    approval: [
      'Confirm the detected destination layout before any move.',
      'Resolve ambiguous metadata and choose among multiple artwork images.',
      'Verify tags and filenames before moving files to their final location.',
    ],
    recovery: [
      'Failed archives are isolated instead of deleted; retry them separately.',
      'No-clobber moves avoid overwriting an existing target; stop and reconcile collisions.',
      'Undo direct tag, artwork, rename, or move changes from the user’s file backup; there is no automatic rollback.',
    ],
    output:
      'A verified, consistently tagged and organized batch ready for normal Rekordbox file or folder import.',
    rekordboxHandoff: [
      { kind: 'library-file-import', mode: 'always' },
      {
        kind: 'manual-cover-art',
        mode: 'conditional',
        condition:
          'A WAV file has a colocated cover image that Rekordbox cannot read from embedded tags.',
      },
    ],
    variants: [],
  },
  {
    id: 'library-health',
    title: 'Library Health',
    route: '/workflows/library-health/',
    summary:
      'Scan broken links, orphan files, playlist gaps, and duplicate groups without changing the collection.',
    audience:
      'Users exploring a library safely or checking structure before and after larger changes.',
    chooseWhen:
      'You want to check files, duplicates, or playlist gaps without changing anything.',
    kind: 'workflow',
    goals: ['inspect-health'],
    libraryImpact: 'read-only',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: false,
      },
      directUserFiles: [],
      localStateWrites: [],
      outputs: [],
    },
    network: {
      level: 'none',
      reason: 'All scans use the local Rekordbox database and filesystem.',
    },
    runtimeHelp: { topic: 'health', menuOrder: 9, recommendedOrder: null },
    prerequisites: [
      'reklawdbox connected to the intended library.',
      'Readable content roots for complete broken-link, orphan, and exact-duplicate scans.',
    ],
    scope:
      'All content roots by default, with optional path/filter limits for individual scans.',
    duration:
      'Scope-dependent; filesystem traversal and exact duplicate hashing can take longer on large libraries.',
    resumability:
      'Results are point-in-time snapshots rather than saved workflow state. Duplicate groups traverse page.next_offset while page.has_more; exact mode rehashes each request. Restart at offset zero, and re-run affected scans, after any Rekordbox scope or filesystem change.',
    approval: [
      'No approval is needed for the read-only scan.',
      'Review every recommendation before taking the separate manual action in Rekordbox or on disk.',
    ],
    recovery: [
      'Correct unreadable roots or narrow the scope, then re-run the scan.',
      'Re-run all affected scans after relocating, importing, assigning, or removing tracks.',
    ],
    output:
      'A point-in-time health report with counts, examples, and suggested manual follow-up.',
    rekordboxHandoff: [
      {
        kind: 'manual-relocate',
        mode: 'conditional',
        condition: 'Broken links have credible relocation suggestions.',
      },
      {
        kind: 'import-or-delete-orphans',
        mode: 'conditional',
        condition:
          'Files exist in a content root but are not imported into Rekordbox.',
      },
      {
        kind: 'assign-playlists',
        mode: 'conditional',
        condition: 'Tracks are intentionally kept but missing from playlists.',
      },
      {
        kind: 'remove-duplicates',
        mode: 'conditional',
        condition: 'Metadata or exact-file duplicate groups are confirmed.',
      },
    ],
    variants: [],
  },
  {
    id: 'dj-prompts',
    title: 'DJ Prompts',
    route: '/workflows/dj-prompts/',
    summary:
      'Choose a conversational recipe for gig prep, collection analysis, candidate evaluation, debriefing, harmonic planning, or practice.',
    audience:
      'DJs who want a guided conversation rather than one fixed operational workflow.',
    chooseWhen:
      'You want conversational help planning, practising, digging, or reviewing a gig.',
    kind: 'catalog',
    goals: ['explore-dj-ideas'],
    libraryImpact: 'read-only',
    sideEffects: {
      stagedMetadata: {
        creates: false,
        flushesExistingOnExport: false,
      },
      directUserFiles: [],
      localStateWrites: [
        {
          kind: 'enrichment-cache',
          mode: 'conditional',
          condition:
            'Collection Gap Analysis or Dig Session Partner evaluates a concrete user-supplied candidate whose lookup is not cached.',
        },
        {
          kind: 'provider-session',
          mode: 'conditional',
          condition:
            'That supplied-candidate lookup needs Discogs authentication.',
        },
      ],
      outputs: [],
    },
    network: {
      level: 'conditional',
      condition:
        'Only recipes evaluating a concrete track or release candidate supplied by the user use provider lookups; local planning recipes do not browse catalogs or discover releases.',
    },
    runtimeHelp: null,
    prerequisites: [
      'reklawdbox connected to a library with the collection or history data needed by the chosen recipe.',
      'Recipe-specific context such as gig constraints, a practice focus, or a session date.',
    ],
    scope:
      'One selected recipe and the collection subset or session context supplied to it.',
    duration:
      'Scope-dependent and set by the chosen recipe and conversation depth.',
    resumability:
      'Recipes do not maintain a durable workflow cursor. Reuse the prompt and prior constraints after a restart.',
    approval: [
      'The user chooses the recipe, constraints, and whether to act on any recommendation.',
    ],
    recovery: [
      'Narrow the request, correct the context, or rerun the recipe with different constraints.',
    ],
    output:
      'A conversation result that varies by recipe; no file or Rekordbox change is made.',
    rekordboxHandoff: [],
    variants: [
      {
        id: 'gig-prep',
        title: 'Gig Prep',
        summary:
          'Translate venue, slot, duration, and vibe into a curated performance pool or suggested sequence.',
        network: {
          level: 'none',
          reason:
            'Track search and transition scoring use local library data and cached analysis.',
        },
        localStateWrites: [],
        prerequisites: [
          'Gig context and enough local candidate tracks for the requested constraints.',
        ],
        duration: 'Scope-dependent; driven by pool size and refinement.',
        output: 'A curated pool and, optionally, a suggested local sequence.',
      },
      {
        id: 'collection-gap-analysis',
        title: 'Collection Gap Analysis',
        summary:
          'Map thin and deep areas in the local library, then frame digging directions or evaluate a concrete candidate supplied by the user.',
        network: {
          level: 'conditional',
          condition:
            'The user supplies a specific track or release candidate that needs an uncached Discogs lookup.',
        },
        localStateWrites: [
          {
            kind: 'enrichment-cache',
            mode: 'conditional',
            condition:
              'The supplied candidate’s Discogs lookup is not already cached.',
          },
          {
            kind: 'provider-session',
            mode: 'conditional',
            condition:
              'Discogs authentication is required for the supplied candidate lookup.',
          },
        ],
        prerequisites: [
          'A library large enough for meaningful genre, key, BPM, energy, or rating distributions; a concrete candidate is required before provider lookup.',
        ],
        duration:
          'Scope-dependent; local analysis can be followed by evaluation of a supplied candidate.',
        output:
          'An actionable description of local collection strengths, blind spots, and broad digging directions, plus evaluation of any supplied candidate.',
      },
      {
        id: 'dig-session-partner',
        title: 'Dig Session Partner',
        summary:
          'Use the local collection as taste context to frame directions and evaluate concrete candidates supplied by the user.',
        network: {
          level: 'conditional',
          condition:
            'A user-supplied track or release candidate needs a Discogs or Beatport lookup that is not already cached.',
        },
        localStateWrites: [
          {
            kind: 'enrichment-cache',
            mode: 'conditional',
            condition:
              'The supplied candidate’s Discogs or Beatport lookup is not already cached.',
          },
          {
            kind: 'provider-session',
            mode: 'conditional',
            condition:
              'Discogs authentication is required for the supplied candidate lookup.',
          },
        ],
        prerequisites: [
          'A desired direction and enough collection context to infer taste; a concrete candidate is required before provider lookup.',
        ],
        duration:
          'Scope-dependent; driven by local analysis, supplied candidates, and follow-up conversation.',
        output:
          'Collection-grounded directions and evaluation of supplied candidates; reklawdbox does not browse label catalogs or discover releases.',
      },
      {
        id: 'post-gig-debrief',
        title: 'Post-Gig Debrief',
        summary:
          'Review a recorded Rekordbox session for energy, tempo, harmonic movement, and rotation patterns.',
        network: {
          level: 'none',
          reason:
            'Session history and track data are read from the local Rekordbox library.',
        },
        localStateWrites: [],
        prerequisites: [
          'A Rekordbox session or history entry for the gig being reviewed.',
        ],
        duration:
          'Scope-dependent; driven by session length and comparison depth.',
        output:
          'A structured debrief with observations to apply to future preparation.',
      },
      {
        id: 'harmonic-journey-planning',
        title: 'Harmonic Journey Planning',
        summary:
          'Plan an extended Camelot-key path and identify track choices or gaps along it.',
        network: {
          level: 'none',
          reason:
            'Key mapping, track search, and transition scoring use local data and cached analysis.',
        },
        localStateWrites: [],
        prerequisites: [
          'A starting point, journey style, duration, and candidate scope with usable key/BPM evidence.',
        ],
        duration:
          'Scope-dependent; driven by journey length and library coverage.',
        output:
          'A harmonic path with candidate tracks and explicit gaps or difficult transitions.',
      },
      {
        id: 'practice-session-design',
        title: 'Practice Session Design',
        summary:
          'Turn a practice focus and time limit into ordered exercises using tracks from the local library.',
        network: {
          level: 'none',
          reason:
            'Track selection and pair scoring use local data and cached analysis.',
        },
        localStateWrites: [],
        prerequisites: [
          'A practice focus, duration, and enough local tracks for the requested exercises.',
        ],
        duration:
          'User-defined practice duration plus scope-dependent preparation.',
        output: 'An ordered set of focused mixing exercises.',
      },
    ],
  },
]

const KINDS = new Set(['workflow', 'catalog'])
const LIBRARY_IMPACTS = new Set([
  'read-only',
  'staged-metadata',
  'direct-audio-files',
  'direct-library-files',
  'mixed',
])
const MODES = new Set(['always', 'conditional', 'optional', 'on-export'])
const NETWORK_LEVELS = new Set(['none', 'conditional', 'required'])
const DIRECT_KINDS = new Set([
  'audio-tags',
  'embedded-artwork',
  'extracted-artwork',
  'downloaded-artwork',
  'move-rename',
  'archive-extraction',
  'archive-move',
  'directory-create-remove',
])
const LOCAL_KINDS = new Set([
  'enrichment-cache',
  'audio-cache',
  'audit-state',
  'preset',
  'timbral-normalization',
  'provider-session',
])
const OUTPUT_KINDS = new Set([
  'backup',
  'metadata-xml',
  'playlist-xml',
  'artwork-file',
  'organized-library-files',
])
const HANDOFF_KINDS = new Set([
  'reload-tag',
  'metadata-xml',
  'playlist-xml',
  'library-file-import',
  'manual-cover-art',
  'manual-relocate',
  'import-or-delete-orphans',
  'assign-playlists',
  'remove-duplicates',
])

function fail(message) {
  throw new Error(`Invalid workflow catalog: ${message}`)
}

function nonEmptyString(value, path) {
  if (typeof value !== 'string' || value.trim() === '') {
    fail(`${path} must be a non-empty string`)
  }
}

function validateWordLimit(value, maximum, path) {
  const wordCount = value.trim().split(/\s+/u).length
  if (wordCount > maximum) {
    fail(`${path} must contain at most ${maximum} words`)
  }
}

function stringArray(value, path) {
  if (!Array.isArray(value)) fail(`${path} must be an array`)
  value.forEach((item, index) => nonEmptyString(item, `${path}[${index}]`))
}

function validateNetwork(network, path) {
  if (!network || typeof network !== 'object' || Array.isArray(network)) {
    fail(`${path} must be an object`)
  }
  if (!NETWORK_LEVELS.has(network.level)) fail(`${path}.level is unknown`)
  if (network.level === 'conditional') {
    nonEmptyString(network.condition, `${path}.condition`)
  }
  if (network.condition !== undefined) {
    nonEmptyString(network.condition, `${path}.condition`)
  }
  if (network.reason !== undefined) {
    nonEmptyString(network.reason, `${path}.reason`)
  }
}

function validateEffects(entries, allowedKinds, path) {
  if (!Array.isArray(entries)) fail(`${path} must be an array`)
  entries.forEach((entry, index) => {
    const itemPath = `${path}[${index}]`
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) {
      fail(`${itemPath} must be an object`)
    }
    if (!allowedKinds.has(entry.kind)) fail(`${itemPath}.kind is unknown`)
    if (!MODES.has(entry.mode)) fail(`${itemPath}.mode is unknown`)
    if (entry.mode === 'conditional' || entry.mode === 'optional') {
      nonEmptyString(entry.condition, `${itemPath}.condition`)
    }
    if (entry.condition !== undefined) {
      nonEmptyString(entry.condition, `${itemPath}.condition`)
    }
  })
}

function validateVariant(variant, expected, path) {
  const fields = [
    'id',
    'title',
    'summary',
    'network',
    'localStateWrites',
    'prerequisites',
    'duration',
    'output',
  ]
  const actualFields = Object.keys(variant).sort()
  if (JSON.stringify(actualFields) !== JSON.stringify([...fields].sort())) {
    fail(`${path} must contain exactly ${fields.join(', ')}`)
  }
  if (variant.id !== expected[0] || variant.title !== expected[1]) {
    fail(`${path} must be ${expected[0]} — ${expected[1]}`)
  }
  nonEmptyString(variant.summary, `${path}.summary`)
  validateNetwork(variant.network, `${path}.network`)
  validateEffects(
    variant.localStateWrites,
    LOCAL_KINDS,
    `${path}.localStateWrites`,
  )
  stringArray(variant.prerequisites, `${path}.prerequisites`)
  nonEmptyString(variant.duration, `${path}.duration`)
  nonEmptyString(variant.output, `${path}.output`)
}

function validateGoalDefinitions(definitions) {
  if (
    !Array.isArray(definitions) || definitions.length !== GOAL_BLUEPRINTS.length
  ) {
    fail(
      `goal definitions must contain exactly ${GOAL_BLUEPRINTS.length} records`,
    )
  }

  definitions.forEach((definition, index) => {
    const path = `goalDefinitions[${index}]`
    if (
      !definition || typeof definition !== 'object' || Array.isArray(definition)
    ) {
      fail(`${path} must be an object`)
    }
    const fields = Object.keys(definition).sort()
    if (JSON.stringify(fields) !== JSON.stringify(['id', 'summary', 'title'])) {
      fail(`${path} must contain exactly id, title, summary`)
    }
    nonEmptyString(definition.title, `${path}.title`)
    nonEmptyString(definition.summary, `${path}.summary`)
    validateWordLimit(definition.title, 8, `${path}.title`)
    validateWordLimit(definition.summary, 18, `${path}.summary`)
    const expected = GOAL_BLUEPRINTS[index]
    if (definition.id !== expected.id || definition.title !== expected.title) {
      fail(`${path} must be ${expected.id} — ${expected.title}`)
    }
  })
}

/**
 * Validate a workflow array without reading source files or adding dependencies.
 * @param {Workflow[]} items
 * @param {{ id: string, title: string, summary: string }[]} [definitions]
 */
export function validateWorkflows(items, definitions = goalDefinitions) {
  validateGoalDefinitions(definitions)

  if (!Array.isArray(items) || items.length !== WORKFLOW_IDS.length) {
    fail(`expected exactly ${WORKFLOW_IDS.length} records`)
  }

  const ids = items.map((item) => item.id)
  if (JSON.stringify(ids) !== JSON.stringify(WORKFLOW_IDS)) {
    fail(`workflow IDs/order must be ${WORKFLOW_IDS.join(', ')}`)
  }

  const seenIds = new Set()
  const seenRoutes = new Set()
  const requiredFields = [
    'id',
    'title',
    'route',
    'summary',
    'audience',
    'chooseWhen',
    'kind',
    'goals',
    'libraryImpact',
    'sideEffects',
    'network',
    'runtimeHelp',
    'prerequisites',
    'scope',
    'duration',
    'resumability',
    'approval',
    'recovery',
    'output',
    'rekordboxHandoff',
    'variants',
  ]

  items.forEach((item, index) => {
    const path = `workflows[${index}]`
    if (!item || typeof item !== 'object' || Array.isArray(item)) {
      fail(`${path} must be an object`)
    }
    requiredFields.forEach((field) => {
      if (!(field in item)) fail(`${path}.${field} is required`)
    })

    nonEmptyString(item.id, `${path}.id`)
    nonEmptyString(item.title, `${path}.title`)
    nonEmptyString(item.route, `${path}.route`)
    nonEmptyString(item.summary, `${path}.summary`)
    nonEmptyString(item.audience, `${path}.audience`)
    nonEmptyString(item.chooseWhen, `${path}.chooseWhen`)
    validateWordLimit(item.chooseWhen, 20, `${path}.chooseWhen`)
    nonEmptyString(item.scope, `${path}.scope`)
    nonEmptyString(item.duration, `${path}.duration`)
    nonEmptyString(item.resumability, `${path}.resumability`)
    nonEmptyString(item.output, `${path}.output`)
    stringArray(item.goals, `${path}.goals`)
    stringArray(item.prerequisites, `${path}.prerequisites`)
    stringArray(item.approval, `${path}.approval`)
    stringArray(item.recovery, `${path}.recovery`)

    if (seenIds.has(item.id)) fail(`${path}.id is duplicated`)
    if (seenRoutes.has(item.route)) fail(`${path}.route is duplicated`)
    seenIds.add(item.id)
    seenRoutes.add(item.route)
    if (!item.route.startsWith('/workflows/') || !item.route.endsWith('/')) {
      fail(
        `${path}.route must be an absolute /workflows/ route with a trailing slash`,
      )
    }
    if (!KINDS.has(item.kind)) fail(`${path}.kind is unknown`)
    const expectedGoal = GOAL_BLUEPRINTS.find((goal) =>
      goal.workflows.includes(item.id)
    )
    if (
      !expectedGoal
      || item.goals.length !== 1
      || item.goals[0] !== expectedGoal.id
    ) {
      fail(
        `${path}.goals must be exactly ${
          expectedGoal?.id ?? 'one canonical goal'
        }`,
      )
    }
    if (!LIBRARY_IMPACTS.has(item.libraryImpact)) {
      fail(`${path}.libraryImpact is unknown`)
    }

    if (
      !item.sideEffects || typeof item.sideEffects !== 'object'
      || Array.isArray(item.sideEffects)
    ) {
      fail(`${path}.sideEffects must be an object`)
    }
    const staged = item.sideEffects.stagedMetadata
    if (!staged || typeof staged !== 'object' || Array.isArray(staged)) {
      fail(`${path}.sideEffects.stagedMetadata must be an object`)
    }
    if (typeof staged.creates !== 'boolean') {
      fail(`${path}.sideEffects.stagedMetadata.creates must be boolean`)
    }
    if (typeof staged.flushesExistingOnExport !== 'boolean') {
      fail(
        `${path}.sideEffects.stagedMetadata.flushesExistingOnExport must be boolean`,
      )
    }

    validateEffects(
      item.sideEffects.directUserFiles,
      DIRECT_KINDS,
      `${path}.sideEffects.directUserFiles`,
    )
    validateEffects(
      item.sideEffects.localStateWrites,
      LOCAL_KINDS,
      `${path}.sideEffects.localStateWrites`,
    )
    validateEffects(
      item.sideEffects.outputs,
      OUTPUT_KINDS,
      `${path}.sideEffects.outputs`,
    )
    validateEffects(
      item.rekordboxHandoff,
      HANDOFF_KINDS,
      `${path}.rekordboxHandoff`,
    )
    validateNetwork(item.network, `${path}.network`)

    const hasDirect = item.sideEffects.directUserFiles.length > 0
    if (item.libraryImpact === 'read-only' && (staged.creates || hasDirect)) {
      fail(
        `${path} cannot be read-only while creating staged metadata or writing user files`,
      )
    }
    if (
      item.libraryImpact === 'staged-metadata' && (!staged.creates || hasDirect)
    ) {
      fail(
        `${path} staged-metadata impact must create staged metadata and not write user files`,
      )
    }
    if (
      (item.libraryImpact === 'direct-audio-files'
        || item.libraryImpact === 'direct-library-files')
      && (!hasDirect || staged.creates)
    ) {
      fail(
        `${path} direct impact must write user files without staging metadata`,
      )
    }
    if (item.libraryImpact === 'mixed' && (!hasDirect || !staged.creates)) {
      fail(
        `${path} mixed impact must include both direct user-file writes and staged metadata`,
      )
    }

    const hasPlaylistXml = item.sideEffects.outputs.some((entry) =>
      entry.kind === 'playlist-xml'
    )
    if (staged.flushesExistingOnExport && !hasPlaylistXml) {
      fail(
        `${path} can flush existing staged metadata only when playlist XML export is possible`,
      )
    }
    const hasXml = item.sideEffects.outputs.some(
      (entry) => entry.kind === 'metadata-xml' || entry.kind === 'playlist-xml',
    )
    const backups = item.sideEffects.outputs.filter((entry) =>
      entry.kind === 'backup'
    )
    if (
      hasXml
      && (backups.length !== 1
        || backups[0].mode !== 'on-export'
        || backups[0].condition !== XML_BACKUP_SUCCESS_CONDITION)
    ) {
      fail(
        `${path} XML output requires exactly one canonical on-export backup entry`,
      )
    }
    if (!hasXml && backups.length > 0) {
      fail(`${path} cannot declare backup without XML export`)
    }

    if (item.kind === 'catalog') {
      if (item.id !== 'dj-prompts') fail(`${path} is an unsupported catalog`)
      if (item.libraryImpact !== 'read-only' || staged.creates || hasDirect) {
        fail(`${path} catalog must preserve the read-only collection invariant`)
      }
      if (
        item.sideEffects.outputs.length > 0 || item.rekordboxHandoff.length > 0
      ) {
        fail(
          `${path} catalog cannot create files or require a Rekordbox handoff`,
        )
      }
      if (
        !Array.isArray(item.variants)
        || item.variants.length !== DJ_VARIANTS.length
      ) {
        fail(`${path}.variants must contain exactly six recipes`)
      }
      item.variants.forEach((variant, variantIndex) =>
        validateVariant(
          variant,
          DJ_VARIANTS[variantIndex],
          `${path}.variants[${variantIndex}]`,
        )
      )
    } else if (!Array.isArray(item.variants) || item.variants.length !== 0) {
      fail(`${path}.variants must be empty for a normal workflow`)
    }

    const expectedRuntime = RUNTIME_HELP.get(item.id)
    if (!expectedRuntime) {
      if (item.runtimeHelp !== null) fail(`${path}.runtimeHelp must be null`)
    } else {
      if (!item.runtimeHelp || typeof item.runtimeHelp !== 'object') {
        fail(`${path}.runtimeHelp is required`)
      }
      const [topic, menuOrder, recommendedOrder] = expectedRuntime
      if (
        item.runtimeHelp.topic !== topic
        || item.runtimeHelp.menuOrder !== menuOrder
        || item.runtimeHelp.recommendedOrder !== recommendedOrder
      ) {
        fail(`${path}.runtimeHelp does not match the canonical 9/7 mapping`)
      }
    }
  })

  const menu = items
    .filter((item) => item.runtimeHelp)
    .sort((a, b) => a.runtimeHelp.menuOrder - b.runtimeHelp.menuOrder)
  const recommended = menu
    .filter((item) => item.runtimeHelp.recommendedOrder !== null)
    .sort((a, b) =>
      a.runtimeHelp.recommendedOrder - b.runtimeHelp.recommendedOrder
    )
  if (
    menu.length !== 9
    || menu.some((item, index) => item.runtimeHelp.menuOrder !== index + 1)
  ) {
    fail('runtime help must contain exactly nine contiguous menu positions')
  }
  if (
    recommended.length !== 7
    || recommended.some((item, index) =>
      item.runtimeHelp.recommendedOrder !== index + 1
    )
  ) {
    fail(
      'runtime help must contain exactly seven contiguous recommended positions',
    )
  }

  const covered = new Set()
  GOAL_BLUEPRINTS.forEach((goal) => {
    const members = items
      .filter((item) => item.goals.includes(goal.id))
      .map((item) => item.id)
    if (JSON.stringify(members) !== JSON.stringify(goal.workflows)) {
      fail(`${goal.id} membership/order must be ${goal.workflows.join(', ')}`)
    }
    members.forEach((id) => {
      if (covered.has(id)) fail(`${id} is assigned to more than one goal`)
      covered.add(id)
    })
  })
  if (covered.size !== WORKFLOW_IDS.length) {
    fail('goal groups must cover all eleven workflows exactly once')
  }

  return items
}

validateWorkflows(workflows)
