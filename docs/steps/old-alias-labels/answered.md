# Answered in the old alias's labels fix, on 2026-11-19

A small fix outside the numbered steps. It settles the item the rename's record left open: "Names built on the old short form, left on purpose" ([the rename's record](../rename-cineris/answered.md), "Left open"), which said a later fix session might rename them once no branch was open over their files. Test and doc text only; no product behaviour changes.

## What was renamed

The pattern is the one step 25 used for the labels in the two files it touched (`cxtui-signals=` became `cintui-signals=`, `CXTUI_SIGNALS_REPORT` became `CINTUI_SIGNALS_REPORT`, `cxcli-tui=` became `cincli-tui=`): the two letters of the old alias become `cin`, the alias the rename gave the command, in the same case and with the rest of the name as it was. Each name was changed where it is set and where it is read.

| Old | New | Where |
|---|---|---|
| `cxterm-demo=`, `cxterm-shell=`, `cxterm-shell-over-raw=`, `cxterm-leaves-raw=`, `cxterm-signals=`, `cxterm-handoff=` | `cinterm-…` | `cineris-term`'s tests (`main.rs`, `signals.rs`, `handoff.rs`) |
| `CXTERM_HANDOFF_REPORT` | `CINTERM_HANDOFF_REPORT` | `cineris-term`'s handoff test |
| `cxtui-demo=`, `cxtui-shell=`, `cxtui-app=`, `cxtui-editor=` | `cintui-…` | `cineris-tui`'s tests (`main.rs`, `children.rs`) |
| `CXTUI_SCRATCH` | `CINTUI_SCRATCH` | `cineris-tui`'s children |
| `cxstats-writer=` | `cinstats-writer=` | `cineris-stats`'s day test |
| `CX_PIPE`, `cxpipe…` (a throwaway Windows user's name) | `CIN_PIPE`, `cinpipe…` | `cineris-rpc`'s pipe tests |
| `CX_EMPTY`, `CX_SYSTEM` | `CIN_EMPTY`, `CIN_SYSTEM` | `cineris-tools`'s shell search test |
| `CX_RERUN_SET`, `CX_RERUN_REMOVED` | `CIN_RERUN_SET`, `CIN_RERUN_REMOVED` | the test kit's rerun tests |
| `cxterm-demo=` in the docs | `cinterm-demo=` | the test kit's `role.rs`, `docs/testing-and-ci.md` |
| `/tmp/cxs-…` | `/tmp/cins-…` | `cineris-sandbox`'s confined test, and the tenth tooling step's record of it |
| `as_cx` (a variable meaning "run as the alias") | `as_cin` | `cineris-cli`'s name tests and help test |

Left as they were, on purpose: `cx` as a tool's context (`cx: &RunCx`, `cx.paths`, `|cx|`), the made-up binary `cx::it` in xtask's rerun tests (a path into something, which the rename's alias rule leaves alone), the character class `[a-cx-z]` in the glob tests (data), `cxx` (the C++ extension), the rename tool and its tests, and the rename's own records.

The Windows throwaway user's name is one character longer (15 of the 20 characters Windows allows). The "fourteen characters" in its comment is about the password, which did not change.

## The check

`cargo xtask rename --check` (and so `cargo xtask rules`) now fails on a label built on the old alias with no separator: the two lower-case letters and a lower-case letter after them at the start of a word, or the two capitals and an underscore or another capital, and the variable `as_cx`. Not labels: `cxx` and `CXX` (C++), the two letters inside a longer word (`s_01JCX4T9Q2`, `SM_CXSCREEN`), bare `cx` and `CX`, a camel-case field, and the snake-case `cx_…`, which the rename's own test names are built on. The files and lines the rename keeps are kept for this too. `apply` does not rename these (they are a test's own words), so the message says to write them by hand with `cin` in the alias's place.

The alias rule of the check already found `cx-` followed by a letter (`cx-tree-guard`) and the alias quoted or named as a command; that is unchanged.

## The review

One round, by an independent reviewer that read the whole diff against the spec. It confirmed:

1. **The check missed a label with nothing after it that the first version looked for** (`Scratch::new("cxpipe")`, `CXPIPE`, `cxterm_handoff`). **Fixed:** the lower-case form needs only a lower-case letter after the two, the upper-case form an underscore or a capital. Tested with each of those, and with the words that must stay (`cxx`, `CXX_FLAGS`, `cxScreen`, `SM_CXSCREEN`, `cx_…`).
2. **The break list's own name, `cx-labels`, was a name the alias rule renames,** so any record that named it would fail the check. **Fixed:** the list is `mutations/old-alias-labels.breaks`.
3. **Two docs were stale:** CONTRIBUTING's summary of the rules did not mention labels, and nothing recorded the settled item. **Fixed:** the summary says it, and this record settles the item. The rename's record is not edited: it describes what was true then.

It found no variable set under one name and read under another, no collision between marks, nothing wrong on Windows and no change of behaviour.

## What was measured

- `cargo xtask check`: passes. `mutate --check --tests`: every break applies and every `caught-by` names a test that exists.
- The tests of every crate changed (`xtask`, `cineris-term`, `cineris-tui`, `cineris-rpc`, `cineris-tools`, `cineris-testkit`, `cineris-stats`, `cineris-cli`, `cineris-sandbox`): 2,617 passed, none failed, run at the desk.
- `cargo xtask mutate old-alias-labels`: 7 breaks, all caught by the tests the list names.
