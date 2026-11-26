#!/usr/bin/env bash
# A setup for `cargo xtask ab --job` (docs/testing-and-ci.md, "A whole job
# two ways on one runner"): makes the round's build rebuild every crate of
# the workspace, as a pull request that touches cineris-base does. With
# `--keep-build` the warm rounds then time that pull request's job, its
# third-party crates already built (docs/steps/ci-budgets-2/ci.md).
set -euo pipefail
touch crates/cineris-base/src/lib.rs
