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

`cargo xtask api-diff --against origin/main --features cortex-tui/test-kit` (`main` at `a2fb9d2`):

**cortex-agent**: 1 added, 0 removed, 0 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_agent::Environment::start` |  | `start: Option<String>` |


**cortex-core**: 3 added, 0 removed, 0 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_core::TrustState::GitNotTrusted` |  | `GitNotTrusted` |
| added | `cortex_core::TrustState::rules_trusted` |  | `fn rules_trusted(self) -> bool` |
| added | `cortex_core::Trusted::git` |  | `git: bool` |


**cortex-policy**: 6 added, 0 removed, 2 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_policy::trust::ProjectTrust::from_older_store` |  | `fn from_older_store(&self) -> bool` |
| added | `cortex_policy::trust::ProjectTrust::git_declined` |  | `fn git_declined(&self) -> bool` |
| added | `cortex_policy::trust::ProjectTrust::git_trusted` |  | `fn git_trusted(&self) -> bool` |
| added | `cortex_policy::trust::ProjectTrust::rules_declined` |  | `fn rules_declined(&self, rules: &ProjectRules) -> bool` |
| changed | `cortex_policy::trust::TrustStore::approve` | `fn approve(&self, key: &ProjectKey, definitions: &[Definition]) -> Result<ProjectTrust, TrustError>` | `fn approve(&self, key: &ProjectKey, definitions: &[Definition], git_shown: bool) -> Result<ProjectTrust, TrustError>` |
| added | `cortex_policy::trust::TrustStore::keep_without` |  | `fn keep_without(&self, key: &ProjectKey, rules: Option<&Definition>, git: bool) -> Result<ProjectTrust, TrustError>` |
| changed | `cortex_policy::trust::TrustStore::trust` | `fn trust(&self, key: &ProjectKey, root: &str, definitions: &[Definition]) -> Result<ProjectTrust, TrustError>` | `fn trust(&self, key: &ProjectKey, root: &str, definitions: &[Definition], git_shown: bool) -> Result<ProjectTrust, TrustError>` |
| added | `cortex_policy::trust::TrustStore::trust_unless_refused` |  | `fn trust_unless_refused(&self, key: &ProjectKey, root: &str, definitions: &[Definition], git_shown: bool) -> Result<ProjectTrust, TrustError>` |


**cortex-protocol**: 3 added, 0 removed, 0 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_protocol::methods::SessionNewParams::cwd` |  | `cwd: Option<String>` |
| added | `cortex_protocol::methods::TrustRequest::trusted` |  | `trusted: bool` |
| added | `cortex_protocol::methods::TrustResponse::keep` |  | `keep: bool` |


**cortex-session**: 2 added, 0 removed, 0 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_session::entry::Header::start` |  | `start: Option<String>` |
| added | `cortex_session::session::Session::create_starting_in` |  | `fn create_starting_in(project: &ProjectSessions, id: SessionId, workspace: &AbsPath, start: Option<String>, options: SessionOptions) -> Result<Self>` |


**cortex-tools**: 1 added, 0 removed, 0 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_tools::shell::ShellConfig::start` |  | `start: Option<String>` |


**cortex-tui**: 10 added, 0 removed, 3 changed

| | Item | Before | After |
|---|---|---|---|
| added | `cortex_tui::app::Start::cwd` |  | `cwd: Option<String>` |
| added | `cortex_tui::editor::Fragment::shown` |  | `fn shown(&self) -> String` |
| added | `cortex_tui::editor::Fragment::with_chips` |  | `fn with_chips(text: impl Into<String>, chips: Vec<Chip>) -> Option<Self>` |
| added | `cortex_tui::history::CHIP_KEEP` |  | `const CHIP_KEEP: usize` |
| added | `cortex_tui::history::CHIP_TEXT_KEEP` |  | `const CHIP_TEXT_KEEP: usize` |
| changed | `cortex_tui::history::HistoryStore::get` | `fn get(&self, index: usize) -> Option<&str>` | `fn get(&self, index: usize) -> Option<&Fragment>` |
| changed | `cortex_tui::history::HistoryStore::push` | `fn push(&mut self, prompt: &str)` | `fn push(&mut self, prompt: &Fragment)` |
| added | `cortex_tui::history::TEXT_KEEP` |  | `const TEXT_KEEP: usize` |
| added | `cortex_tui::history::is_blank` |  | `fn is_blank(prompt: &crate::editor::Fragment) -> bool` |
| added | `cortex_tui::history::kept` |  | `fn kept(prompt: &crate::editor::Fragment) -> crate::editor::Fragment` |
| changed | `cortex_tui::input::Input::search_found` | `fn search_found(&self) -> Option<&str>` | `fn search_found(&self) -> Option<&Fragment>` |
| added | `cortex_tui::view::dialog::TrustView::unsaid` |  | `unsaid: bool` |
| added | `cortex_tui::view::dialog::third_trust_answer` |  | `fn third_trust_answer(request: &cortex_protocol::methods::TrustRequest) -> &'static str` |

No public API changes: cortex-base, cortex-config, cortex-ext, cortex-index, cortex-knowledge, cortex-llm, cortex-mcp, cortex-rpc, cortex-sandbox, cortex-stats, cortex-term, cortex-testkit.

## The history's bounds, and the measurement behind them

| Bound | Value | Why |
|---|---|---|
| A chip's text kept (`CHIP_KEEP`) | 64 KiB | The owner's figure. A recalled chip says it was cut (`[pasted 2,345 lines, first 64 KiB kept]`) and sends what was kept |
| A prompt's chips together (`CHIP_TEXT_KEEP`) | 256 KiB | So that a prompt of many pastes is still one entry of a bounded size; a chip past it keeps its label |
| A prompt's own text (`TEXT_KEEP`) | 64 KiB, with a line saying how many characters were left out | The coordinator's answer E: text from the external editor is kept, cut, rather than not kept |
| What a project holds in memory (`KEEP`, `KEEP_BYTES`) | 1,000 prompts and 2 MiB, those sent in the run included | The idle budget is 25 MB and the shipped build idles at 12.3 MB in a workspace of 20,000 files ([performance-budgets.md](../../../performance-budgets.md)); 2 MiB keeps every glibc and musl figure there within it |
| When the file is written again | Past 20,000 lines or 8 MiB (`COMPACT_BYTES`) | |
| What a file written again keeps | Each project's newest within 1,000 and 2 MiB, every project's together within 4 MiB (`FILE_KEEP_BYTES`) | Half the trigger, so that a file is not written again at every start |
| What a start reads | At most the file's last 16 MiB (`READ_LIMIT`) | An older build's file of pasted logs, of any size, costs one read of its end and is then written within the bounds |

A size in these bounds is the size of the line this build writes for what is kept: an older build's line of a pasted log counts as the chip or cut text it is kept as (since the full review, finding 1). The reader goes through the file newest first, parsing only the lines of its project and stopping at its bounds, and a compaction holds only what it keeps, so the reader's peak is the bytes read and at most 4 MiB beside them.

**Measured** with `perf_history_read_at_its_caps` (`crates/cortex-cli/src/history.rs`, ignored, run by hand: `cargo test --release -p cortex-cli --bin cortex perf_history -- --ignored --nocapture`), on the desktop through `rx`, release build, 15 runs each, a file of twenty projects whose every eighth prompt carries a 64 KiB chip:

| Case | Median | Slowest | What the project holds after | The file after |
|---|---|---|---|---|
| A file at 8 MiB, read (no compaction) | 5.7 ms | 7.5 ms | 1,603 KiB | 8,185 KiB, as it was |
| An older build's file of 64 MiB: its last 16 MiB read, and written again | 30.2 ms | 36.7 ms | 1,988 KiB | 4,061 KiB |

The read runs on the history's own thread, beside the start-up, and the interface never waits for it ([09-tui.md](../../../architecture/09-tui.md), "History"): the start-up budgets are not on its path. What it holds after is within 2 MiB of the idle budget's room.

## The independent reviews

**The early review of the trust store's format** (asked for as soon as it existed). Its findings, each fixed with a test:

| Finding | Fix | Test |
|---|---|---|
| A yes the store could not write gave git to a session where git was declined or never shown | `git_after_an_unkept_yes`: git only when shown or trusted before; a reload keeps it (`ForTheRun { pin, git }`) | `assemble::trust_and_start_tests::an_unkept_yes_gives_git_only_what_was_shown_or_trusted_before` |
| A yes to git alone cleared rules kept out before | `approve` clears `declined_rules` only when it approves rules | `cortex-policy`'s `trust::rules_kept_out_are_declined_while_they_are_those_rules` |
| No test showed git kept out of a repository whose git was not shown | A system test of the four places git trust reaches | `cortex-core`'s `trust::sys_a_repository_whose_git_was_not_shown_gets_nothing_of_git` |
| A never said by another client while a project's first question was open was lifted by a yes (it predates this change) | `TrustStore::trust_unless_refused`, under the store's lock | `trusting::a_first_yes_after_a_never_said_meanwhile_is_not_used` |
| A trusted repository's notice said "became a repository" after an older store was written again | One sentence that names both causes | (text) |

**Step 24's reload, met on the rebase:** its judgement gave git to every trusted project on each reload. It now reads the pin as an open does (`judge_pins`), tested by `reload::a_reload_reads_the_git_pin_as_an_open_does`.

**The full review of the whole change.** Its findings, each fixed with a test:

| Finding | Fix | Test |
|---|---|---|
| 1. An older build's long line was counted as written: at a start it could hide a project's prompts, and a compaction could drop every line before it | A line's size is that of what is kept of it | `history::tests::an_older_builds_long_line_counts_as_what_is_kept` |
| 2. The prompts sent in a run were held without a bound | The whole a project holds is kept within 1,000 and 2 MiB, the oldest dropped first | `history::tests::the_prompts_sent_in_a_run_are_held_within_the_bounds` |
| 3. On Windows, a working directory reached through a junction out of the workspace failed the session's open (`invalid_cwd`) | The binary starts commands at the root when the working directory leads out of it, and says so | `workspace::tests::a_start_through_a_link_out_starts_at_the_root_and_says_why` (Linux, a link) |
| 4. No test ran a command with no `cwd` where the session started, nor a binary below the root, nor the passed-over notice, nor keep of rules and git together | Tests added | `shell::a_command_with_no_cwd_runs_where_the_session_started`; `commands::sys_a_project_trusted_with_permissions_trust_is_trusted_on_the_next_start` (headless from `src`); the workspace test; `trusting::keep_of_new_rules_and_new_git_declines_both`; the context test checks the shell's text |
| 5. A client from before that answers never to a trusted project's question is refused where it was taken | Kept, and said in [11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md): a never there is what the owner ruled out | (documented) |
| 6. A start whose name is not Unicode was told to the model with replacement characters | Such a start starts at the root, in the binary and in the core | (the core's `start_below`; the binary's `leads_inside`) |

## CI

The pull request's first green run (43970151315, at `67a9a49`), against `main`'s latest run that kept its reports (`327cfaa`; `main`'s own commit, `a2fb9d2`, had none):

| Job | Main, `327cfaa` | The PR at `67a9a49` |
|---|---|---|
| `lint-linux` | 55 s | 57 s |
| `test-linux` | 104 s | 114 s |
| `lint-windows` | 88 s | 86 s |
| `test-windows` | 283 s | 281 s |
| `pr-size-linux` (blocks nothing) | — | 214 s |
| `pr-size-windows` (blocks nothing) | — | 367 s |

| Time above process start, against the budget | Linux, main (4 ms) | Linux, `67a9a49` (3 ms) | Windows, main (8 ms) | Windows, `67a9a49` (8 ms) |
|---|---|---|---|---|
| Tier 1, unit (20 s) | 12.6 s | 10.2 s | 14.6 s | 15.5 s |
| Tier 2, integration (40 s) | 34.2 s | 16.8 s | 38.2 s | 41.5 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 21.6 s | 22.9 s | 81.6 s | 83.1 s |

Windows' tier 2 was 41.5 s, over its budget of 40 s (the warning; it fails at 60 s), on a runner other than `main`'s. **A one-runner A/B** (`try` with `against=main`, run 43969493125, five rounds each at 6.0 ms a process start) shows the change's cost apart from the runner's speed:

| Tier | The PR | Main | Change | Budget |
|---|---|---|---|---|
| 1, unit | 11.7 s (2,526 tests) | 11.7 s (2,504) | +0.0 s | 20 s |
| 2, integration | 28.7 s (1,480 tests) | 29.7 s (1,469) | −0.9 s | 40 s |
| 3, system | 64.1 s (186 tests) | 62.9 s (185) | +1.3 s | 90 s |

What moved in tier 1 is `cortex-cli`'s new history tests (+0.92 s together: files of 8 to 16 MiB written and read); tier 3 gains the two system tests (`trust::sys_a_repository_whose_git_was_not_shown_gets_nothing_of_git`, and a headless run from a folder in `commands`). The Windows `try` of the changed crates alone passed (43969451213).

## Stress and breaks

- **Breaks:** `cargo xtask mutate step-23-followups --jobs 2`: 43 breaks, every one caught by the tests its list names (one list corrected: `version-1-kept-on-write` named a test that starts from no file, where a version 1 store cannot show; the two that read one catch it). The sixteen breaks of steps 13a, 17, 23 and 24 that named code this change rewrote were moved to the code as it is (`mutate --check`: every break applies).

- **Stress, plain:** `cargo xtask stress -p cortex-cli -p cortex-core -p cortex-config --count 50` (the side slot): 652 tests, 50 times each, seeds 0 to 49: every test passed every time.
- **Stress, loaded** (`--cores 2 --load 6`): 49 of 50 rounds passed. Once, under seed 33, `cortex-config`'s `discovery::in_the_workspace_a_link_is_looked_at_before_it_is_followed_on_unix` was stopped at 43.9 s. It is in a crate this change does not touch, it runs in milliseconds, and it was stopped while the disk stalled (every task waited on I/O 90.9% of one 10 s window). Run again under seed 33 it passed, and 100 loaded rounds of `cortex-config`'s discovery tests (seeds 0 to 99) passed every time. It is not a listed flake, and it is taken for the machine.

## Left open

- **Untested on Windows:** the start through a junction out of the workspace (finding 3) is tested with a link on Linux only; the Windows test runner cannot make a junction without privileges the test kit does not ask for. The code compares the canonical forms with `cortex_base::location_within`, as the Windows facts in `AGENTS.md` require.
- **A client from before that answers never to a trusted project's question** is refused, as above: a narrowing documented in 11, with no version of its own.
- Nothing of the five items is left open.
