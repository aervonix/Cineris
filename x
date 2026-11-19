#!/bin/sh
# ./x <command> [args]: the developer commands, from any directory.
# It is `cargo xtask <command>`. Run ./x with no arguments for the list.
#
# `replace` and `links` still work when xtask does not build (in the middle of
# a change to xtask itself): the build is tried first, and when it fails the
# last binary that ran a command to its end is run instead, which says how old
# it is. CONTRIBUTING.md, "Editing from a script".
#
# `./x --root DIR <command>` works on the checkout DIR names, read from the
# directory ./x was run in; without it, on the checkout ./x is in.
case "${1-}" in
    --root=*) first=${1#--root=}; shift; set -- --root "$first" "$@" ;;
esac
if [ "${1-}" = "--root" ] && [ $# -ge 2 ]; then
    root=$(cd "$2" && pwd) || exit 1
    shift 2
    set -- --root "$root" "$@"
fi
cd "$(dirname "$0")" || exit 1
cargo="${CARGO:-cargo}"
command="${1-}"
if [ "$command" = "--root" ]; then
    command="${3-}"
fi
case "$command" in
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
