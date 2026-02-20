// @ts-check
import { defineConfig, passthroughImageService } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://reklawdbox.com',
  image: {
    service: passthroughImageService(),
  },
  integrations: [
    starlight({
      title: 'reklawdbox Docs',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/ryan-voitiskis/reklawdbox' }],
      sidebar: [
        {
          label: 'Getting Started',
          items: [{ label: 'Overview', slug: 'getting-started' }],
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
          label: 'Reference',
          autogenerate: { directory: 'reference' },
        },
        {
          label: 'Troubleshooting',
          autogenerate: { directory: 'troubleshooting' },
        },
      ],
    }),
  ],
});
