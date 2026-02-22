#!/usr/bin/env bash
set -euo pipefail

DEFAULT_VENV_PATH="${HOME}/.local/share/reklawdbox/essentia-venv"
VENV_PATH="${ESSENTIA_VENV_PATH:-$DEFAULT_VENV_PATH}"

python_supported() {
  local bin="$1"
  "${bin}" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 9) else 1)' >/dev/null 2>&1
}

pick_python() {
  if [[ -n "${ESSENTIA_PYTHON_BIN:-}" ]]; then
    if command -v "${ESSENTIA_PYTHON_BIN}" >/dev/null 2>&1; then
      if ! python_supported "${ESSENTIA_PYTHON_BIN}"; then
        echo "Configured ESSENTIA_PYTHON_BIN is unsupported (<3.9): ${ESSENTIA_PYTHON_BIN}" >&2
        return 1
      fi
      command -v "${ESSENTIA_PYTHON_BIN}"
      return 0
    fi
    echo "Configured ESSENTIA_PYTHON_BIN was not found: ${ESSENTIA_PYTHON_BIN}" >&2
    return 1
  fi

  local candidates=(
    python3.13
    python3.12
    python3.11
    python3.10
    python3.9
    python3
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    if command -v "${candidate}" >/dev/null 2>&1; then
      if ! python_supported "${candidate}"; then
        continue
      fi
      command -v "${candidate}"
      return 0
    fi
  done

  echo "No supported Python found (tried: ${candidates[*]})." >&2
  echo "Install Python 3.9+ (Python 3.13 recommended) and retry." >&2
  return 1
}

PYTHON_BIN="$(pick_python)"

echo "Using Python: ${PYTHON_BIN}"
echo "Creating/updating virtualenv: ${VENV_PATH}"
"${PYTHON_BIN}" -m venv "${VENV_PATH}"

VENV_PYTHON="${VENV_PATH}/bin/python"

echo "Upgrading pip/setuptools/wheel"
"${VENV_PYTHON}" -m pip install --upgrade pip setuptools wheel

echo "Installing Essentia (pre-release wheel channel)"
"${VENV_PYTHON}" -m pip install --upgrade --pre essentia

echo "Verifying Essentia import"
"${VENV_PYTHON}" -c 'import essentia, sys; print(sys.executable); print(essentia.__version__)'

cat <<EOF

Essentia is ready.
Set this in your MCP env:
CRATE_DIG_ESSENTIA_PYTHON=${VENV_PYTHON}

Then restart your MCP host/server process.
EOF
