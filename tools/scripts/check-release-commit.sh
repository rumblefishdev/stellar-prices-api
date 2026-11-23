#!/usr/bin/env bash
#
# Refuse to change production from anything but the release commit: a clean
# working tree whose HEAD is the freshly fetched tip of `deploy/production`.
# Run first by every `make deploy-production*`, `destroy-production*` and
# `sync-portal-explorer` target (task 0343, step 13).
#
# WHY
# ---
# `master` is the record of what runs in production, and it only moves to a
# commit that was deployed from `deploy/production`. A deploy from anywhere
# else makes that record false without anyone noticing: Compute went out from
# `develop` by hand on 2026-11-18 and 2026-11-19, and only the release that
# followed brought the branches back to what was live.
#
# There is deliberately no override (decided 2026-11-20). An urgent fix is a
# PR to `deploy/production`; once merged, its tip deploys through CI or
# through these targets.
#
# Usage:  tools/scripts/check-release-commit.sh [repo-root]
#
# The remote and the branch are constants on purpose: an environment variable
# naming them would be an override by another name (`RELEASE_BRANCH=develop`).

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${1:-$(cd "${here}/../.." && pwd)}"
readonly remote=origin
readonly branch=deploy/production

refuse() {
  echo "::error::release guard: $1" >&2
  shift
  for line in "$@"; do echo "  → $line" >&2; done
  exit 1
}

git -C "$root" rev-parse --git-dir >/dev/null 2>&1 \
  || refuse "'${root}' is not a git checkout"

# Fetched now, not trusted from the last fetch: a stale remote-tracking ref is
# exactly how a laptop ends up deploying a release that has since moved on.
git -C "$root" fetch --quiet "$remote" "refs/heads/${branch}:refs/remotes/${remote}/${branch}" \
  || refuse "could not fetch ${remote}/${branch}, so the release commit is unknown"

# Untracked files count: an uncommitted source file is built into the deploy
# just like a modified one. Ignored files (target/, dist/, node_modules/) do not.
dirty="$(git -C "$root" status --porcelain)"
if [[ -n "$dirty" ]]; then
  refuse "the working tree is not clean" \
    "$(printf '%s\n' "$dirty" | head -n 10 | paste -sd ';' -)" \
    "commit, stash or remove these, then check out the release commit"
fi

head="$(git -C "$root" rev-parse HEAD)"
release="$(git -C "$root" rev-parse "refs/remotes/${remote}/${branch}")"
if [[ "$head" != "$release" ]]; then
  refuse "HEAD is not the tip of ${remote}/${branch}" \
    "HEAD:    ${head} ($(git -C "$root" log -1 --format=%s "$head"))" \
    "release: ${release} ($(git -C "$root" log -1 --format=%s "$release"))" \
    "git checkout --detach ${remote}/${branch}" \
    "to ship something else, merge it into ${branch} by PR first"
fi

echo "release guard: HEAD is ${remote}/${branch} @ ${head:0:8}, tree clean"
