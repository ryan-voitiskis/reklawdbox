---
title: Troubleshooting
description: Common failure modes and fast resolution paths.
---

## Summary

Most issues fall into MCP wiring, database pathing, credentials, or XML import validation.

## Steps

1. Verify binary build and executable path.
2. Confirm MCP host uses stdio and the expected command.
3. Validate `REKORDBOX_DB_PATH` and file permissions.
4. Re-run with `preview_changes` before `write_xml` to isolate logic issues.
5. Check host logs for tool invocation and JSON payload errors.

## Examples

`master.db` not found:

- Set `REKORDBOX_DB_PATH` explicitly.
- Confirm Rekordbox has initialized the database on this machine.

Discogs lookup not authenticating:

- Verify broker URL/token env vars.
- Complete broker auth flow via returned `auth_url`.

## Related

- [Getting Started](/getting-started/)
- [Reference](/reference/)
