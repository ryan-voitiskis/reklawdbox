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

# Ensure clean working tree
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "error: working tree is dirty — commit or stash changes first"
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

# Read current version
CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "Releasing: $CURRENT -> $VERSION"

# Bump version in Cargo.toml
sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" Cargo.toml

# Update Cargo.lock
cargo generate-lockfile --quiet

# Commit, tag, push
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to $VERSION"
git tag "$TAG"
git push origin main "$TAG"

echo ""
echo "Released $TAG — CI will build and publish."
echo "https://github.com/ryan-voitiskis/reklawdbox/actions"
