#!/usr/bin/env bash
# Publishes the version in VERSION once "git flow release/hotfix finish" has tagged it:
# pushes the main and develop branches and the tags, then creates the GitHub release with
# the matching CHANGELOG.md section as notes. The Release workflow builds the binaries and
# attaches them to it.
#
# Usage: scripts/release.sh [gh release create options...]
#   e.g. scripts/release.sh --draft
set -euo pipefail

cd "$(dirname "$0")/.."

die() { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }

command -v gh >/dev/null 2>&1 || die "gh not found — install it from https://cli.github.com"
VERSION=$(tr -d '[:space:]' < VERSION)
[ -n "$VERSION" ] || die "VERSION is empty"
# An empty git-flow prefix (git flow init's default) still means "v": release.yml only runs on v* tags.
PREFIX=$(git config gitflow.prefix.versiontag || true)
TAG="${PREFIX:-v}$VERSION"
MAIN=$(git config gitflow.branch.master || echo main)
DEVELOP=$(git config gitflow.branch.develop || echo develop)

git diff --quiet HEAD -- || die "uncommitted changes; commit or stash them first"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null ||
  die "tag $TAG not found; finish the release first (git flow release finish $VERSION)"
git merge-base --is-ancestor "$TAG" "$MAIN" || die "$TAG is not on $MAIN"

# The "## [x.y.z]" section of the changelog, up to the next "## [" heading.
NOTES=$(awk -v h="## [$VERSION]" '/^## \[/ { p = index($0, h) == 1; next } p' CHANGELOG.md |
  sed '/./,$!d')
[ -n "$NOTES" ] || die "no '## [$VERSION]' section in CHANGELOG.md"

BRANCHES=("$MAIN")
git rev-parse -q --verify "refs/heads/$DEVELOP" >/dev/null && BRANCHES+=("$DEVELOP")
info "pushing ${BRANCHES[*]} and tags"
git push origin "${BRANCHES[@]}" && git push origin --tags

info "creating GitHub release $TAG"
gh release create "$TAG" --verify-tag --title "$TAG" --notes "$NOTES" "$@"
