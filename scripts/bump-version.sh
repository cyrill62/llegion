#!/usr/bin/env bash
# Sets the project version, updates the changelog, and commits it with the description.
# A direct bump on the main branch also creates the annotated version tag. Git-flow
# release/hotfix branches are tagged by `git flow ... finish` after the merge.
#
# Usage: scripts/bump-version.sh [options] [major | minor | patch | X.Y.Z]
#
#   With no version, it is taken from the current git-flow branch (release/X.Y.Z or
#   hotfix/X.Y.Z), so the usual flow is:
#     git flow release start 0.2.0    (or: git flow hotfix start 0.1.3)
#     scripts/bump-version.sh
#     git flow release finish 0.2.0
#     scripts/release.sh
#   major / minor / patch bump the version currently in VERSION.
#   A direct bump on the main branch creates the annotated tag after committing.
#
#   Options:
#     -m, --message TEXT  the version description, instead of asking for it
#                         (without a terminal, it is read from stdin)
#     --no-commit         only edit the files
#     --screenshots       regenerate app screenshots
#     -h, --help          show this help
#
# Run automatically on "git flow release/hotfix start" with:
#   git config gitflow.path.hooks scripts/hooks
set -euo pipefail

cd "$(dirname "$0")/.."

die() { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }
usage() { sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }
# Replaces $1 with stdin in place, keeping its permissions.
rewrite() { local tmp; tmp=$(mktemp); cat > "$tmp"; cat "$tmp" > "$1"; rm -f "$tmp"; }

COMMIT=1
SCREENSHOTS=0
ARG=""
DESC=""
while [ $# -gt 0 ]; do
  case "$1" in
    -m | --message) [ $# -ge 2 ] || die "$1 needs a value"; DESC=$2; shift ;;
    --no-commit) COMMIT=0 ;;
    --screenshots) SCREENSHOTS=1 ;;
    -h | --help) usage ;;
    -*) die "unknown option '$1' (see --help)" ;;
    *) [ -z "$ARG" ] || die "more than one version given"; ARG=$1 ;;
  esac
  shift
done

SEMVER='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'
CURRENT=$(tr -d '[:space:]' < VERSION 2>/dev/null || true)
[ -n "$CURRENT" ] || CURRENT=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n1)
CURRENT=${CURRENT#v}

branch=$(git rev-parse --abbrev-ref HEAD)
release_prefix=$(git config gitflow.prefix.release || echo release/)
hotfix_prefix=$(git config gitflow.prefix.hotfix || echo hotfix/)
# Commit subject label, as in "[Hotfix] v0.1.2".
case "$branch" in
  "$hotfix_prefix"*) KIND=Hotfix ;;
  *) KIND=Release ;;
esac

if [ -z "$ARG" ]; then
  case "$branch" in
    "$release_prefix"* | "$hotfix_prefix"*) ARG=${branch#"$release_prefix"}; ARG=${ARG#"$hotfix_prefix"} ;;
    *) die "not on a git-flow release/hotfix branch; give a version (see --help)" ;;
  esac
  ARG=${ARG#v}
fi

case "$ARG" in
  major | minor | patch)
    [[ "$CURRENT" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+) ]] || die "current version '$CURRENT' is not X.Y.Z"
    maj=${BASH_REMATCH[1]} min=${BASH_REMATCH[2]} pat=${BASH_REMATCH[3]}
    case "$ARG" in
      major) NEW="$((maj + 1)).0.0" ;;
      minor) NEW="$maj.$((min + 1)).0" ;;
      patch) NEW="$maj.$min.$((pat + 1))" ;;
    esac ;;
  *) NEW=${ARG#v} ;;
esac
[[ "$NEW" =~ $SEMVER ]] || die "'$NEW' is not a semantic version (X.Y.Z)"
[ "$NEW" != "$CURRENT" ] || die "version is already $NEW"

TAG_PREFIX=$(git config gitflow.prefix.versiontag || true)
TAG_PREFIX=${TAG_PREFIX:-v} # empty (git flow init's default) still means "v": release.yml only runs on v* tags
MAIN_BRANCH=$(git config gitflow.branch.master || echo main)
TAG="$TAG_PREFIX$NEW"
if [ "$COMMIT" -eq 1 ] && [ "$branch" = "$MAIN_BRANCH" ] \
  && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  die "tag $TAG already exists"
fi
info "bumping $CURRENT -> $NEW"

if [ -z "$DESC" ]; then
  if [ -t 0 ]; then
    echo "Describe $TAG_PREFIX$NEW (Markdown, e.g. '- Fix zone resize'). Finish with an empty line:" >&2
    while IFS= read -r -p "> " line && [ -n "$line" ]; do
      DESC+="$line"$'\n'
    done
  else
    DESC=$(cat)
  fi
fi
# Trim leading and trailing blank lines.
DESC=$(printf '%s\n' "$DESC" | sed -e '/./,$!d' | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}')
[ -n "$DESC" ] || die "a version description is required"
export DESC

echo "$NEW" > VERSION

# Only the [package] version, not dependency versions.
awk -v v="$NEW" '
  /^\[/ { pkg = ($0 == "[package]") }
  pkg && !done && /^version *=/ { print "version = \"" v "\""; done = 1; next }
  { print }
' Cargo.toml | rewrite Cargo.toml

# Our own entry in Cargo.lock, edited directly so this works offline and without cargo.
if [ -f Cargo.lock ]; then
  name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' Cargo.toml | head -n1)
  awk -v n="$name" -v v="$NEW" '
    $0 == "name = \"" n "\"" { print; getline; if ($0 ~ /^version = /) $0 = "version = \"" v "\"" }
    { print }
  ' Cargo.lock | rewrite Cargo.lock
fi

# "## [Unreleased]" keeps its place; the new version heading and the description go
# below it, followed by the entries that were unreleased, and the compare links at the
# bottom follow. The description comes through ENVIRON since awk -v mangles backslashes.
if grep -qF "## [$NEW]" CHANGELOG.md; then
  info "CHANGELOG.md already has a $NEW section, left unchanged"
elif grep -q '^## \[Unreleased\]' CHANGELOG.md; then
  awk -v v="$NEW" -v d="$(date +%Y-%m-%d)" -v p="$TAG_PREFIX" '
    /^## \[Unreleased\]/ { print; print ""; print "## [" v "] - " d; print ""; print ENVIRON["DESC"]; next }
    /^\[Unreleased\]: / {
      base = $2; sub(/compare\/.*/, "", base)
      prev = $2; sub(/.*compare\//, "", prev); sub(/\.\.\.HEAD$/, "", prev)
      print "[Unreleased]: " base "compare/" p v "...HEAD"
      print "[" v "]: " base "compare/" prev "..." p v
      next
    }
    { print }
  ' CHANGELOG.md | rewrite CHANGELOG.md
else
  info "warning: no '## [Unreleased]' section in CHANGELOG.md, left unchanged"
fi

if [ "$SCREENSHOTS" -eq 1 ]; then
  info "regenerating app screenshots"
  ./scripts/screenshots.sh
fi

if [ $COMMIT -eq 1 ]; then
  git add VERSION Cargo.toml
  [ -f CHANGELOG.md ] && git add CHANGELOG.md
  [ -f Cargo.lock ] && git add Cargo.lock
  [ "$SCREENSHOTS" -eq 0 ] || git add docs/screenshots
  git commit -m "[$KIND] $TAG_PREFIX$NEW" -m "$DESC"
  if [ "$branch" = "$MAIN_BRANCH" ]; then
    git tag -a "$TAG" -m "$TAG"
    info "committed and created tag $TAG"
  elif [[ "$branch" == "$release_prefix"* || "$branch" == "$hotfix_prefix"* ]]; then
    info "committed; finish the release/hotfix with git flow to create tag $TAG"
  else
    info "committed; no tag created outside $MAIN_BRANCH or a git-flow release/hotfix"
  fi
else
  info "files updated (not committed)"
fi
