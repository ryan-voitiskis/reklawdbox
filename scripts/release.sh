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

docs_contract_changed() {
  changed_since_base \
    site \
    src/tools \
    src/audio.rs \
    src/color.rs \
    src/genre.rs \
    src/types.rs \
    src/tags.rs \
    src/cli \
    src/main.rs \
    Cargo.toml \
    Cargo.lock \
    scripts/mcp-smoke.mjs \
    scripts/lib/mcp-stdio.mjs \
    scripts/check-doc-contract.mjs \
    scripts/check-doc-contract.test.mjs \
    scripts/release.sh \
    .github/workflows/docs-pages.yml \
    docs/workflows/doc-drift \
    site/src/content/docs/mcp-tools \
    site/src/content/docs/cli \
    site/src/partials/sops \
    site/src/data/workflows.mjs \
    site/src/data/tool-reference.mjs \
    site/astro.config.mjs \
    README.md
}

run_docs_gate_if_needed() {
  if docs_contract_changed; then
    echo "Code-backed documentation contracts changed since $BASE_TAG; running docs gate."
    node --test scripts/check-doc-contract.test.mjs
    (cd site && npm ci && npm run build)
    node scripts/check-doc-contract.mjs \
      --bin ./target/release/reklawdbox \
      --dist ./site/dist
  else
    echo "Code-backed documentation contracts unchanged since $BASE_TAG; skipping docs gate."
  fi
}

run_broker_gate_if_needed() {
  if changed_since_base broker .github/workflows/broker-ci.yml; then
    echo "Broker changed since $BASE_TAG; running broker gate."
    # Keep sharp on its locked prebuilt binary instead of workstation-global libvips.
    (cd broker && SHARP_IGNORE_GLOBAL_LIBVIPS=1 npm ci && npm run typecheck && npm run build && npm test)
  else
    echo "Broker unchanged since $BASE_TAG; skipping broker gate."
  fi
}

check_doc_drift() {
  if docs_contract_changed; then
    echo "Automated documentation contracts passed."
    echo "Semantic reminder: review workflow intent, risks, recovery guidance, external UI steps, and user-facing clarity using docs/workflows/doc-drift/README.md."
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
