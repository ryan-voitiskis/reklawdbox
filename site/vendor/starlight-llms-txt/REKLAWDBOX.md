# Local Astro 7 Patch

This is `starlight-llms-txt` 0.10.0 vendored locally because the published npm
package still declares `astro@^6.0.0` and blocks a clean Astro 7/Starlight 0.41
install.

Local package metadata changes:

- `version`: `0.10.0-reklawdbox.0`
- `@astrojs/mdx`: `^7.0.0`
- peer `astro`: `^7.0.2`
- peer `@astrojs/starlight`: `>=0.41.0`
- backward-compatible `excludeFull` option for excluding selected docs from
  generic `llms-full.txt` without changing `llms-small.txt` or custom sets

Remove this vendor copy and switch `site/package.json` back to the npm package
only when upstream publishes Astro 7-compatible peer metadata and provides an
equivalent generic-full exclusion. Any migration must prove that generic full
and small outputs exclude agent routes while dedicated custom agent sets remain
complete.
