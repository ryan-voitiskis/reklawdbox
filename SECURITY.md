# Security Policy

## Supported versions

Security fixes are developed on `main` and released in the latest stable
version. Older releases are not supported; upgrade before reporting an issue
that may already be fixed.

## Report a vulnerability privately

Use GitHub's private vulnerability reporting to create a draft security
advisory:

[Report a Reklawdbox vulnerability privately](https://github.com/ryan-voitiskis/reklawdbox/security/advisories/new)

If that form is unavailable, contact the maintainer through a private channel.
Do not open a public issue or disclose details before a fix is available.

Include, where possible:

- the affected component, version or commit, and platform;
- the security impact, required preconditions, and attack scenario;
- minimal reproduction steps or a proof of concept;
- expected and actual behavior; and
- a suggested mitigation, if you have one.

## Protect private data

Use synthetic or minimized test data. Redact local paths, track metadata,
account identifiers, tokens, and credentials. Do not attach a Rekordbox
`master.db`, audio files, Reklawdbox caches, or complete library exports unless
the maintainer explicitly requests them in the private advisory.

If a credential has been exposed, revoke or rotate it immediately; reporting
it does not make it safe to keep using.

## What happens next

The maintainer will triage the report, may request more information, and will
coordinate a fix, release, and public advisory when appropriate. Please allow
time for users to update before publishing technical details.
