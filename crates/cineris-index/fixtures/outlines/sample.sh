#!/usr/bin/env bash
set -euo pipefail

# Prints usage.
usage() {
  cat <<EOF
usage() {
  not a function
}
EOF
}

function build {
  local n=${#targets[@]} # count
  echo "build() { fake"
}

function deploy() {
  helper() {
    echo inner
  }
}

clean_up () { rm -rf "$tmp"; }

case "$1" in
  start) usage ;;
esac

decode() {
  base64 -d <<<SGVsbG8=
}

after_decode() {
  :
}
