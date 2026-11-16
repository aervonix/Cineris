# Step 23, after the review: the follow-ups

Five open items, found by the review of step 23 and of step 17's third pull request, built by the owner's decisions of 2026-11-16 (relayed by the coordinator, with its answers to the design's five readings, A to E). Each has a test that fails without the fix and a break the test catches (`mutations/step-23-followups.breaks`).

| Item | The owner's decision | What was built | Recorded in |
|---|---|---|---|
| `json-outputs-that-escape-what-json-does-not` | Print them through `cortex project list --json`'s function, and look for the same mechanism in every `--json` output | `cortex-cli`'s `json` module (`line`, `record`), moved out of `project_view`: `usage --json`, `stats show --json`, `sessions list --json` and `project list --json` print through it; so do headless mode's `--output json` and `jsonl`, the same mechanism's twin, which printed a bidirectional control or a C1 control as the character itself. `serve` speaks JSON-RPC to a client, not to a terminal, and is left as it is | This record |
| `the-workspace-root-is-the-working-directory` | Walk up to the root, as 08 says; show the root in the header; the working directory is the commands' first `cd` | Every binary finds the root with `find_workspace_root` (`cortex-cli`'s `workspace::Here`) and says what it passed over. `session/new` takes `cwd` below the root, kept in the session log's header as `start`; the shell runs a command with no `cwd` there; the environment and the shell's description say so only for a start below the root (a root start's prompt is the same, to the byte); instruction files are read from the root down to it | [08-configuration.md](../../../architecture/08-configuration.md), "Layers"; [07-sessions-and-context.md](../../../architecture/07-sessions-and-context.md); [11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md) |
| `trust-question-corners` (1) | On a rules-changed question, "Keep trusting it, without the new rules" in place of "Never"; never stays on a project's first question; the dialog says what each answer does | `TrustRequest.trusted` and `TrustResponse.keep`; keep is stored (`declined_rules`, `git = "declined"`) and not asked again while what is new is what it is; a line under the answers says what each does; reference screen 2c drawn again, two rows taller | [06-permissions-and-sandbox.md](../../../architecture/06-permissions-and-sandbox.md), "Project trust"; [09-tui.md](../../../architecture/09-tui.md) |
| `trust-question-corners` (2) | Pin whether git was shown; ask again when a trusted folder becomes a repository; a new store version, an old one read with git shown only where the folder was a repository when trusted (or say what was chosen) | `trust.toml` version 2 (`git`); a version 1 store is read with git never shown (the coordinator's answer C, below) | [06-permissions-and-sandbox.md](../../../architecture/06-permissions-and-sandbox.md); [../../../security.md](../../../security.md) |
| `history-has-no-size-limit` | A large paste is stored as its chip, its text up to a cap; the file is compacted by bytes as well as lines | `cortex_tui::history::kept`; `HistoryStore` holds `Fragment`s; the file's bounds below | [09-tui.md](../../../architecture/09-tui.md), "History"; [build-plan.md](../../../build-plan.md), step 25 (this part of its item is done) |

## The trust store's format

- **Version 1** (step 12 to here): `[[project]]` with `key`, `root`, `trusted_at` and `[[project.pinned]]`; `[[refused]]` since step 23.
- **Version 2** adds two keys to a project: `git = "trusted"` (git was in what the person was shown, and trusted with it) or `git = "declined"` (they kept trusting the project without git), absent when git was never shown; and `declined_rules = "<hash>"`, the project's rules as they were when the person kept trusting it without them.
- **How a version 1 store is read** (the coordinator's answer C): every project with git **never shown**. Each trusted repository is asked about git once more, its rules unchanged, and the session says that the store was older. The alternative, "shown where the folder was a repository when trusted", could only be told from `.git`'s creation time, which on Windows the folder's owner can set and archive tools restore, and which would make the two systems differ; asking once is what cannot trust what was not shown. The file is written as version 2 at its next change; a version 1 file that holds `git` or `declined_rules` is refused, since no build wrote one.
- **What an older build does with version 2:** it reads only version 1, so it refuses the file whole by its version (`TrustError::NotUnderstood`), and trusts nothing, writing nothing over it, until it is updated: decision 0028's rule, a file a build cannot read in full is not read in part. (Its `Project` also refuses an unknown key, a second refusal of the same file.)
- **The write** is as it was: under the store's lock from the read to the write, atomic, owner-only (`private` on both systems, checked by the store's tests on Linux and on the Windows runner).

## The workspace root, before and after

| Case | Before | After |
|---|---|---|
| Started in `repo/src` | `repo/src` is the workspace: `repo/.cortex/config.toml` not read, the root's `AGENTS.md` not read as the root's, trust said at `repo/` not applied, sessions, approvals, knowledge and history under `src`'s key | `repo/` is the workspace, every one of those is `repo/`'s; commands start in `src`, which the environment and the shell's description say; `repo/AGENTS.md` and `repo/src/AGENTS.md` are both read |
| Started at `repo/` | `repo/` | `repo/`, the system prompt the same to the byte |
| A session stored under `src`'s key before | Listed and continued from `repo/src` | Not listed by `sessions list` or taken by `--continue` from `repo/src`; `--resume <id>` and `sessions list --all` find it, and it opens with the workspace it was made in |
| Trust, a never or approvals given at `repo/src` before | In force from `repo/src` | Filed under `src`'s key and no longer consulted from below `repo/`; inert, not lost (`cortex permissions` lists the projects trusted, by root). A folder with nothing above it is its own root, as before |

## Signature changes

To be filled in with `cargo xtask api-diff` before the pull request.

## Measurements

To be filled in.

## The independent review

To be filled in.

## Left open

To be filled in.
