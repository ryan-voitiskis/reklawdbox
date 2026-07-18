// @ts-check
import sitemap from '@astrojs/sitemap'
import starlight from '@astrojs/starlight'
import { defineConfig, passthroughImageService } from 'astro/config'
import starlightLlmsTxt from 'starlight-llms-txt'

export default defineConfig({
  site: 'https://reklawdbox.com',
  image: {
    service: passthroughImageService(),
  },
  integrations: [
    sitemap({
      filter: (page) => !new URL(page).pathname.startsWith('/agent/'),
    }),
    starlight({
      title: 'reklawdbox',
      logo: {
        src: './src/assets/logo.svg',
        alt: '',
      },
      favicon: '/favicon.svg',
      head: [
        {
          tag: 'link',
          attrs: {
            rel: 'preload',
            href: '/fonts/InterVariable-Latin.woff2',
            as: 'font',
            type: 'font/woff2',
            crossorigin: '',
          },
        },
        {
          tag: 'link',
          attrs: {
            rel: 'preload',
            href: '/fonts/BerkeleyMonoVariable-Regular.woff2',
            as: 'font',
            type: 'font/woff2',
            crossorigin: '',
          },
        },
      ],
      customCss: ['./src/styles/custom.css'],
      social: [{
        icon: 'github',
        label: 'GitHub',
        href: 'https://github.com/ryan-voitiskis/reklawdbox',
      }],
      plugins: [
        starlightLlmsTxt({
          exclude: ['agent/**'],
          excludeFull: ['agent/**'],
          customSets: [
            {
              label: 'Agent SOPs',
              paths: ['agent/**'],
              description:
                'Token-optimized workflow instructions for AI agents',
            },
            {
              label: 'Batch Import SOP',
              paths: ['agent/batch-import'],
              description: 'Agent SOP for batch importing new music',
            },
            {
              label: 'Collection Audit SOP',
              paths: ['agent/collection-audit'],
              description: 'Agent SOP for collection audit',
            },
            {
              label: 'Genre Classification SOP',
              paths: ['agent/genre-classification'],
              description: 'Agent SOP for genre classification',
            },
            {
              label: 'Genre Audit SOP',
              paths: ['agent/genre-audit'],
              description: 'Agent SOP for genre audit',
            },
            {
              label: 'Set Building SOP',
              paths: ['agent/set-building'],
              description: 'Agent SOP for DJ set building',
            },
            {
              label: 'Pool Building SOP',
              paths: ['agent/pool-building'],
              description: 'Agent SOP for pool building',
            },
            {
              label: 'Chapter Set Planning SOP',
              paths: ['agent/chapter-set-planning'],
              description: 'Agent SOP for chapter set planning',
            },
            {
              label: 'Metadata Backfill SOP',
              paths: ['agent/metadata-backfill'],
              description: 'Agent SOP for metadata backfill',
            },
            {
              label: 'Library Health SOP',
              paths: ['agent/library-health'],
              description: 'Agent SOP for library health scanning',
            },
          ],
        }),
      ],
      sidebar: [
        { slug: 'getting-started', label: 'Install' },
        {
          slug: 'getting-started/first-session',
          label: 'First 10 minutes',
        },
        { slug: 'workflows', label: 'Choose a workflow' },
        {
          label: 'Library Cleanup',
          items: [
            { slug: 'workflows/library-cleanup', label: 'Overview' },
            { slug: 'workflows/collection-audit' },
            { slug: 'workflows/metadata-backfill' },
            { slug: 'workflows/genre-classification' },
            { slug: 'workflows/genre-audit' },
          ],
        },
        {
          label: 'Mixing & Sets',
          items: [
            { slug: 'workflows/set-building' },
            { slug: 'workflows/pool-building' },
            { slug: 'workflows/chapter-set-planning' },
            { slug: 'concepts/harmonic-mixing' },
          ],
        },
        {
          label: 'Day-to-Day',
          items: [
            { slug: 'workflows/batch-import' },
            { slug: 'workflows/library-health' },
          ],
        },
        {
          label: 'Ideas & Discovery',
          items: [{ slug: 'workflows/dj-prompts' }],
        },
        {
          label: 'How It Works',
          collapsed: true,
          items: [
            { slug: 'concepts', label: 'Overview' },
            { slug: 'concepts/architecture' },
            { slug: 'concepts/pool-discovery' },
            { slug: 'concepts/safety', label: 'Safety & Trust' },
          ],
        },
        {
          label: 'Reference',
          collapsed: true,
          items: [
            { slug: 'reference', label: 'Reference overview' },
            {
              label: 'MCP Tools',
              items: [
                { slug: 'mcp-tools', label: 'Overview' },
                { slug: 'mcp-tools/library-data' },
                { slug: 'mcp-tools/enrichment-analysis' },
                { slug: 'mcp-tools/classification-staging' },
                { slug: 'mcp-tools/mixing' },
                { slug: 'mcp-tools/files-system' },
              ],
            },
            { slug: 'cli' },
            { slug: 'reference/environment-variables' },
            { slug: 'reference/transition-scoring' },
            { slug: 'reference/xml-export' },
            { slug: 'reference/keyboard-shortcuts' },
          ],
        },
        { slug: 'troubleshooting' },
      ],
    }),
  ],
})
