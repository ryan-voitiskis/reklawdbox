// @ts-check
import starlight from '@astrojs/starlight'
import { defineConfig, passthroughImageService } from 'astro/config'
import starlightLlmsTxt from 'starlight-llms-txt'

export default defineConfig({
  site: 'https://reklawdbox.com',
  image: {
    service: passthroughImageService(),
  },
  integrations: [
    starlight({
      title: 'reklawdbox',
      logo: {
        src: './src/assets/logo.png',
        alt: 'reklawdbox',
      },
      favicon: '/favicon.png',
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
        {
          label: 'Getting Started',
          autogenerate: { directory: 'getting-started' },
        },
        {
          label: 'Concepts',
          autogenerate: { directory: 'concepts' },
        },
        {
          label: 'Workflows',
          autogenerate: { directory: 'workflows' },
        },
        {
          label: 'MCP Tools',
          autogenerate: { directory: 'mcp-tools' },
        },
        {
          label: 'CLI',
          autogenerate: { directory: 'cli' },
        },
        {
          label: 'Reference',
          autogenerate: { directory: 'reference' },
        },
        {
          label: 'Agent SOPs',
          autogenerate: { directory: 'agent' },
        },
        {
          label: 'Troubleshooting',
          autogenerate: { directory: 'troubleshooting' },
        },
      ],
    }),
  ],
})
