#!/usr/bin/env bash
set -euo pipefail

# Load user-local MCP env vars if present.
ENV_FILE="${HOME}/.config/reklawdbox/mcp.env"
if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck source=/dev/null
  source "${ENV_FILE}"
  set +a
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

exec "${REPO_ROOT}/target/release/reklawdbox"
