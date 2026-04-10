#!/bin/sh
# Build release binary and overwrite the Homebrew-installed binary.
# Usage: ./scripts/deploy-local.sh
#
# Requires sudo. The Cellar binary stays overwritten until the next
# `brew upgrade` or release, so no manual revert is needed.

set -e

CELLAR_VERSION=$(brew list --versions reklawdbox | awk '{print $2}')
if [ -z "$CELLAR_VERSION" ]; then
  echo "error: reklawdbox not installed via Homebrew" >&2
  exit 1
fi

CELLAR_BIN="/opt/homebrew/Cellar/reklawdbox/${CELLAR_VERSION}/bin/reklawdbox"
if [ ! -f "$CELLAR_BIN" ]; then
  echo "error: expected binary not found at $CELLAR_BIN" >&2
  exit 1
fi

cd "$(dirname "$0")/.."
cargo build --release

sudo cp target/release/reklawdbox "$CELLAR_BIN"
codesign --sign - --force "$CELLAR_BIN"
echo "deployed to $CELLAR_BIN"
