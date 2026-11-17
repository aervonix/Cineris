Nothing blocking builds the fuzz package or checks its seeds: a change that breaks a target, or a method its seeds miss, shows only in the nightly.

- Area: tooling
- Owner: a tooling step

| Item | Who | When |
|---|---|---|
| **The fuzz package checked only at night.** `fuzz/` is outside the workspace and built only by the nightly workflow's `fuzz` job and `cargo xtask fuzz` ([testing-and-ci.md](../testing-and-ci.md), "Not blocking"). Since step 27 its `rpc_decoder` target compiles `cortex-rpc`'s `wire.rs` from its file, so a `use crate::…` added there breaks only the nightly; and its seed of requests is a copy of the golden transcript's client lines, so a method added later leaves the seed short with nothing failing. **To choose:** a blocking check that `wire.rs` names nothing of its crate (a repo rule), and seeds taken from `crates/cortex-protocol/tests/golden/v1-session.jsonl` when the fuzzer runs rather than copied; or a cheap build of the fuzz package's stable tests in a job that blocks nothing. | A tooling step | When convenient |
