# Reklawdbox branding

The production mark is **Claw + Groove**: a crab pincer doubles as a tonearm
over a vinyl record. Its palette is:

- orange gradient `#ff5b08` to `#f34700`
- lifted-ink gradient `#1d2a3d` to `#111a29`
- warm off-white `#faf9f6`

## Source of truth

- `site/src/assets/logo.svg` is the full logo master.
- `site/public/favicon.svg` is a generated public copy of that master, so the
  claw construction and all four grooves cannot drift from the production mark.

Run the generator after editing the master:

```bash
node scripts/generate-brand-assets.mjs
```

The generator requires `rsvg-convert` and refreshes the public SVG copy, the
tracked PNG fallbacks, the broker source PNG, and the broker callback's embedded
PNG data URI. Do not edit those generated outputs by hand.
