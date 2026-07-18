#!/usr/bin/env node

import { execFileSync } from 'node:child_process'
import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const repositoryRoot = join(scriptDirectory, '..')
const logoSvg = join(repositoryRoot, 'site/src/assets/logo.svg')
const faviconSvg = join(repositoryRoot, 'site/public/favicon.svg')
const siteLogoPng = join(repositoryRoot, 'site/src/assets/logo.png')
const siteFaviconPng = join(repositoryRoot, 'site/public/favicon.png')
const brokerLogoPng = join(
  repositoryRoot,
  'broker/assets/branding/reklawdbox-logo-source.png',
)
const brokerBrandingModule = join(repositoryRoot, 'broker/src/branding.ts')
const temporaryDirectory = mkdtempSync(join(tmpdir(), 'reklawdbox-brand-'))
const callbackLogoPng = join(temporaryDirectory, 'callback-logo.png')

function renderSvg(input, output, size) {
  execFileSync(
    'rsvg-convert',
    [
      '--width',
      String(size),
      '--height',
      String(size),
      input,
      '--output',
      output,
    ],
    { stdio: 'inherit' },
  )
}

try {
  execFileSync('rsvg-convert', ['--version'], { stdio: 'ignore' })

  copyFileSync(logoSvg, faviconSvg)
  renderSvg(logoSvg, siteLogoPng, 1024)
  copyFileSync(siteLogoPng, brokerLogoPng)
  renderSvg(faviconSvg, siteFaviconPng, 64)
  renderSvg(logoSvg, callbackLogoPng, 384)

  const callbackLogoDataUri = `data:image/png;base64,${
    readFileSync(
      callbackLogoPng,
    ).toString('base64')
  }`
  const brandingSource = readFileSync(brokerBrandingModule, 'utf8')
  const callbackLogoPattern =
    /export const CALLBACK_LOGO_DATA_URI = "data:image\/png;base64,[^"]+";/

  if (!callbackLogoPattern.test(brandingSource)) {
    throw new Error('Unable to find CALLBACK_LOGO_DATA_URI in branding.ts')
  }

  writeFileSync(
    brokerBrandingModule,
    brandingSource.replace(
      callbackLogoPattern,
      `export const CALLBACK_LOGO_DATA_URI = "${callbackLogoDataUri}";`,
    ),
  )
} finally {
  rmSync(temporaryDirectory, { recursive: true, force: true })
}

console.log('Generated site and broker brand assets from the SVG master.')
