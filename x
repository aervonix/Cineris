#!/bin/sh
# ./x <command> [args]: the developer commands, from any directory.
# It is `cargo xtask <command>`. Run ./x with no arguments for the list.
#
# `replace` and `links` still work when xtask does not build (in the middle of
# a change to xtask itself): the build is tried first, and when it fails the
# last binary that ran a command to its end is run instead, which says how old
# it is. CONTRIBUTING.md, "Editing from a script".
cd "$(dirname "$0")" || exit 1
cargo="${CARGO:-cargo}"
case "${1-}" in
    replace | links)
        # Only a build that ran and failed, not a cargo that is not there.
        if command -v "$cargo" >/dev/null 2>&1 && ! "$cargo" build --quiet --package xtask; then
            copy="target/xtask-last-good/xtask"
            if [ ! -x "$copy" ]; then
                echo "x: xtask does not build, and no earlier build was kept in $copy." >&2
                exit 1
            fi
            echo "x: xtask does not build: running the last good copy of it." >&2
            exec "$copy" "$@"
        fi
        ;;
esac
exec "$cargo" run --quiet --package xtask -- "$@"
