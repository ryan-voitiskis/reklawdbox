#!/bin/sh
# Usage: ./scripts/release.sh 0.25.0
#
# Bumps version in Cargo.toml, commits, tags, and pushes.
# The tag push triggers .github/workflows/release.yml which builds,
# creates the GitHub Release, and updates the Homebrew formula.

set -e

VERSION="${1:?Usage: ./scripts/release.sh <version> (e.g. 0.25.0)}"

# Strip leading 'v' if provided
VERSION="${VERSION#v}"

# Validate semver format
echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' || {
  echo "error: '$VERSION' is not valid semver (expected X.Y.Z)"
  exit 1
}

TAG="v${VERSION}"

changed_since_base() {
  diff_status=0
  git diff --quiet "$BASE_TAG"..HEAD -- "$@" || diff_status=$?

  case "$diff_status" in
    0)
      return 1
      ;;
    1)
      return 0
      ;;
    *)
      echo "error: failed to compare changed paths against $BASE_TAG"
      exit "$diff_status"
      ;;
  esac
}

run_mcp_smoke() {
  if [ "${REKLAWDBOX_RELEASE_SKIP_DB_SMOKE:-}" = "1" ]; then
    echo "Running MCP smoke without Rekordbox DB (REKLAWDBOX_RELEASE_SKIP_DB_SMOKE=1)."
    ./scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
  else
    echo "Running MCP smoke with playlist 'genre_verified'."
    echo "Set REKLAWDBOX_RELEASE_SKIP_DB_SMOKE=1 to use the no-DB smoke when the local DB/playlist is unavailable."
    ./scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --playlist genre_verified --timeout-ms 60000
  fi
}

run_docs_gate_if_needed() {
  if changed_since_base site .github/workflows/docs-pages.yml; then
    echo "Docs site changed since $BASE_TAG; running docs build gate."
    (cd site && npm ci && npm run build)
  else
    echo "Docs site unchanged since $BASE_TAG; skipping docs build gate."
  fi
}

run_broker_gate_if_needed() {
  if changed_since_base broker .github/workflows/broker-ci.yml; then
    echo "Broker changed since $BASE_TAG; running broker gate."
    (cd broker && npm ci && npm run typecheck && npm run build && npm test)
  else
    echo "Broker unchanged since $BASE_TAG; skipping broker gate."
  fi
}

check_doc_drift() {
  echo "Doc drift reminder: if MCP tools, params, CLI flags, README claims, or embedded SOPs changed, run docs/workflows/doc-drift/README.md before release."

  if changed_since_base \
    src/tools/params.rs \
    src/tools/mod.rs \
    site/src/partials/sops \
    README.md \
    src/main.rs \
    src/cli; then
    if [ "${REKLAWDBOX_DOC_DRIFT_DONE:-}" != "1" ]; then
      echo "error: release-sensitive tool, CLI, README, or embedded SOP surfaces changed since $BASE_TAG."
      echo "Run docs/workflows/doc-drift/README.md, then rerun with REKLAWDBOX_DOC_DRIFT_DONE=1."
      exit 1
    fi

    echo "REKLAWDBOX_DOC_DRIFT_DONE=1 set; continuing after operator-confirmed doc-drift workflow."
  fi
}

run_cargo_audit_if_available() {
  if command -v cargo-audit >/dev/null 2>&1; then
    echo "Running cargo audit."
    cargo audit
  else
    echo "cargo-audit not installed; skipping advisory audit."
  fi
}

run_release_preflight() {
  echo "Running release preflight checks..."
  cargo fmt --check
  dprint check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test -p reklawdbox --no-fail-fast
  cargo test -p stratum-dsp --no-fail-fast
  cargo build --release
  ./target/release/reklawdbox --version
  ./target/release/reklawdbox --help
  run_mcp_smoke
  run_docs_gate_if_needed
  run_broker_gate_if_needed
  check_doc_drift
  run_cargo_audit_if_available
}

# Ensure clean working tree, including untracked files.
if [ -n "$(git status --porcelain)" ]; then
  echo "error: working tree is dirty — commit, stash, or remove changes first"
  git status --short
  exit 1
fi

# Ensure we're on main
BRANCH=$(git branch --show-current)
if [ "$BRANCH" != "main" ]; then
  echo "error: releases must be from main (currently on '$BRANCH')"
  exit 1
fi

# Check tag doesn't already exist
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "error: tag $TAG already exists"
  exit 1
fi

BASE_TAG=$(git describe --tags --abbrev=0) || {
  echo "error: could not determine latest tag for release preflight comparison"
  exit 1
}
echo "Using $BASE_TAG as release comparison base."

# Read current version
CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "Releasing: $CURRENT -> $VERSION"

run_release_preflight

# Bump version in Cargo.toml
sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" Cargo.toml

# Bump version on site homepage
sed -i '' "s/v${CURRENT} —/v${VERSION} —/" site/src/content/docs/index.mdx

# Update Cargo.lock
cargo generate-lockfile --quiet

# Commit, tag, push
git add Cargo.toml Cargo.lock site/src/content/docs/index.mdx
git commit -m "chore: bump version to $VERSION"
git tag "$TAG"
git push origin main "$TAG"

echo ""
echo "Released $TAG — CI will build and publish."
echo "https://github.com/ryan-voitiskis/reklawdbox/actions"
