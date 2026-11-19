#!/usr/bin/env bash
# Dates the checkout's files so that cargo rebuilds exactly what changed since
# the build the workspace cache restored (docs/testing-and-ci.md, "The
# workspace's own build, cached").
#
#   date-the-checkout.sh KEY
#
# KEY is the cache key that was restored, which ends in the commit the main
# branch built it from; empty when nothing was restored.
#
# Cargo takes a workspace crate's build as fresh when no file it was built
# from is newer than the build. A checkout dates every file at the moment of
# the checkout, so a restored build looks older than all of its sources and is
# rebuilt whole. This gives every tracked file one date long before any build
# (2000-01-01), and then every file that differs between that commit and the
# checkout (added, changed or renamed, by git's comparison of the two trees)
# the present, which is after the build: what changed is rebuilt, with
# everything that depends on it, and nothing else.
#
# Whenever the comparison cannot be made (no key, a key without a commit, a
# commit that cannot be fetched, files on disk that are not the checkout's
# commit, a diff that fails), the dates are left as the checkout made them,
# and everything is rebuilt, as it was before the cache.
#
# What else a crate's build depends on, cargo does not judge by dates: the
# compiler's version (and the toolchain is in the cache's key), the profile,
# the features, the environment variables a crate reads, and the third-party
# crates (Cargo.lock). Files outside the checkout that a build reads are the
# registry's sources, which never change for a version, and the toolchain's.
set -euo pipefail

key=${1-}
if [ -z "$key" ]; then
    echo "No build of the workspace's crates was restored: they are all built."
    exit 0
fi
commit=${key##*-}
if ! [[ "$commit" =~ ^[0-9a-f]{40}$ ]]; then
    echo "The restored key ($key) names no commit: every crate is rebuilt."
    exit 0
fi
# The checkout is shallow: the commit is fetched by its hash, which GitHub
# serves for any commit of the repository.
if ! git cat-file -e "$commit^{commit}" 2>/dev/null &&
    ! git fetch --no-tags --quiet --depth=1 origin "$commit"; then
    echo "::warning::Could not fetch $commit, which the restored build was made from: every crate is rebuilt."
    exit 0
fi
# The comparison is of two commits, so the files on disk must be exactly the
# checkout's commit: a file written after the checkout (by an earlier step,
# a script, a build that writes into the tree), new or changed or ignored,
# would be dated with the rest and could be taken as built. Anything but the
# build folder itself, and the dates are left alone.
stray=$(git status --porcelain --untracked-files=all --ignored=matching | grep -v '^!! target/$' || true)
if [ -n "$stray" ]; then
    echo "::warning::The checkout has files that are not its commit's, so every crate is rebuilt:"
    printf '%s\n' "$stray" | sed -n '1,20p' | sed 's/^/  /'
    exit 0
fi
changed=$(mktemp)
if ! git diff --name-only --no-renames -z "$commit" HEAD >"$changed"; then
    echo "::warning::Could not compare $commit with the checkout: every crate is rebuilt."
    exit 0
fi
# Attributes change the bytes a checkout writes (line endings, filters)
# without changing any file's content in git, so a file a build read could
# differ on disk while its blob does not. The cache's key holds every
# attributes file and the checkout's settings, so this is a second guard.
if tr '\0' '\n' <"$changed" | grep -q '\(^\|/\)\.gitattributes$'; then
    echo "::warning::A .gitattributes file changed since $commit: every crate is rebuilt."
    exit 0
fi

# Every tracked file: long before any build.
git ls-files -z | xargs -0 -r touch -c -m --date=@946684800
# What changed since the build: now, after it. `-c`: a file the change
# deleted is not made again.
xargs -0 -r touch -c -m <"$changed"

count=$(tr -cd '\0' <"$changed" | wc -c)
echo "The restored build was made from $commit; $count file(s) differ from it, and are rebuilt with what depends on them:"
tr '\0' '\n' <"$changed" | sed -n '1,40p' | sed 's/^/  /'
if [ "$count" -gt 40 ]; then
    echo "  ..."
fi
