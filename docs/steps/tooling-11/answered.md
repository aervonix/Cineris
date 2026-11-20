# Answered in the eleventh tooling step, on 2026-11-19

This step is not a numbered step of the build plan. It brought the Windows pull-request test job back under five minutes with room to spare, and rooted `xtask` at the checkout it runs in. Its records are here, in a folder named for the job; the CI times are in [ci.md](ci.md).

## The Windows test job

The aim: the Windows pull-request test job under 4 minutes on a slow runner (8 to 9 ms a process start), without moving a test out of the blocking job, loosening a tier budget, or raising the 5-minute limit.

### Where the time went

The table is in [ci.md](ci.md), "Before".

- **On a slow runner the job took 293 to 313 s:**
  - setup, the dependency cache, the toolchain and nextest, about 35 s;
  - xtask's own `dev` build, 22 to 25 s;
  - the build of the workspace's crates, 130 to 158 s;
  - the tiers, 95 to 110 s;
  - the report's upload, 1 to 2 s.
- **Every pull request built every workspace crate**, whatever it changed: rust-cache keeps the third-party crates only.
- **The `--timings` of a full build** (experiment run 44431433363, a 6 ms runner): 59 of its 68 units are the workspace's.
  - The longest are xtask's own test binary (25 s, last on the critical path), `cineris-core`'s integration tests (22 s), `cineris-tui`'s unit tests (19 s) and `cineris-agent`'s integration tests (16 s).
  - The runner has 2 cores and 4 threads (an AMD EPYC 9V45). Real-time antivirus scanning is off (`RealTimeProtectionEnabled: False`), as step 2 found.

### What was tried

Each of these was measured in one job on one runner, in turn: experiment run 44431433363, on a scratch branch whose `ci.yml` was the experiment.

| Candidate | Measured | Kept |
|---|---|---|
| Cache the workspace crates' build | Restored, with the checkout dated against its commit: nothing changed, 0.2 s of build. A change to `cineris-cli` 4.0 s; to `cineris-tui` 15.0 s; to `cineris-tools` 32.1 s; to `cineris-testkit` 52.3 s; to `cineris-base` 72.2 s. A full build of the workspace's crates was 81.6 s there. 2.1 GB of build, 380 MB packed with zstd; packed in 3.8 s and unpacked in 1.6 s | **Yes** |
| `rust-lld` in place of `link.exe` | Full build 122.3 s against 81.6 s (the noise of one job), a rebuild after `cineris-base` 71.6 s against 72.2 s, after `cineris-tools` 31.9 s against 32.1 s; its tests pass | No: no gain, as step 2 found |
| No debug info for the workspace's crates (Windows CI only) | A full build 104.2 s with `link.exe` against 81.6 s, and 65.1 s with `rust-lld` against 122.3 s: noise larger than any difference. The build shrank from 2.1 to 1.2 GB | No: no gain shown, and a failing test's backtrace would lose its line numbers |
| One build for all tiers (a nextest archive) | The tiers' second and third runs find the build fresh in 0.1 to 0.2 s | No: nothing to save |
| The report's upload | 0 to 2 s, 125 KB | Unchanged; a notice now says every test passed before it (below) |
| The toolchain and nextest | 8 to 11 s and 2 to 5 s, in every job. The toolchain installs `clippy` and `rustfmt`, which the test job does not use, as `rust-toolchain.toml` asks | Unchanged: a few seconds at most, for a toolchain set up differently in one job |
| xtask's own `dev` build | 22 to 25 s, in every job | Served by the same cache: 1 s |

### What changed

The details are in [testing-and-ci.md](../../testing-and-ci.md), "The workspace's own build, cached".

- **The main branch saves its whole build of the workspace's own crates** after its tests pass (`actions/cache/save`, GitHub's own action, new to this repository, pinned by hash at v6.1.0).
  - The key is `workspace-build-<OS>-<toolchain hash>-<checkout settings hash>-<commit>`.
  - The paths (`WORKSPACE_BUILD` in `ci.yml`) are every file cargo made for a `cineris-*` crate or xtask, in the `ci` profile and in xtask's `dev` build with the crates it depends on.
  - The main branch never restores it, so each saved build is one commit's whole build.
- **Every other run on Windows restores the newest one** (`actions/cache/restore`) and runs `.github/scripts/date-the-checkout.sh`.
  - The script gives every tracked file the date 2000-01-01, and every file that differs between the build's commit and the checkout the present. Cargo then rebuilds what changed and what depends on it.
  - It leaves every date as the checkout made it, and so everything is rebuilt, in each of these cases:
    - nothing was restored;
    - the key has no commit;
    - the commit cannot be fetched;
    - a file on disk is not the commit's (new, changed or ignored, outside `target/`);
    - an attributes file changed;
    - the diff fails.
- **The checkout settings in the key** are git's `core.autocrlf` and `core.eol`, every `.gitattributes` file's blob, and `.git/info/attributes`: what decides the bytes a checkout writes without being any file's content. A build made under other settings is not restored.
- **A notice that every test passed** (`::notice title=test-<os>::…`), written before the report is uploaded. A job the time limit cuts off after it is then told apart from one whose tests failed, on the run's page and in its annotations.
- **`cargo xtask rules` refuses a workflow step that could write a cache from any run but the main branch's**, in every workflow and every action of `.github/actions/`; [testing-and-ci.md](../../testing-and-ci.md) has the rule. It held already in practice; now it is checked.

### Each case tried on a runner

These ran on a scratch branch (`exp-wci-3`) whose `ci.yml` was this branch's, with one change: a dispatch input chose the main branch's role (build and save) or a pull request's (restore and date), because a branch's caches can be read only by that branch's runs. A case's change was one commit on top of the branch's base, and each run's restored build was the base's.

| Case | Run | What the job did | Result |
|---|---|---|---|
| The main branch's role | 44434153486, 44435419033, 44438435795 | Built everything and saved 353 to 363 MB, in 8 s | Passed |
| A change to `cineris-tools` | 44436126823 (8 ms) | Rebuilt `cineris-tools` and what depends on it (54 s); xtask's build 1 s | Passed, 197 s |
| A test in `cineris-stats` that fails on purpose | 44434797121 | Rebuilt `cineris-stats` and its dependents | **Failed** on `failing_on_purpose::fails_on_purpose` |
| An `include_str!` input changed (`cineris-base`'s Unicode table, its version line) | 44436146191 | Rebuilt `cineris-base`'s test binary alone (4.6 s): the file is read by test code only | **Failed** on `the_table_of_characters_that_render_as_nothing_is_unicodes` |
| A build script's `rerun-if-changed` input changed (`catalog/models.toml`, made invalid) | 44436166187 | Ran `cineris-llm`'s build script again | **Failed**: the script's TOML error |
| An `include_str!` input deleted | 44436607813 | Rebuilt `cineris-base` | **Failed**: the file cannot be read |
| An `include_str!` input renamed | 44436628001 | Rebuilt `cineris-base` and `cineris-testkit`, both of which read it | **Failed**: the old name cannot be read |
| `Cargo.lock` alone changed (`itoa` 1.0.19 to 1.0.18) | 44436680815 | Rebuilt `itoa`, every third-party crate above it (`serde_json`, `http`, `hyper`, …) and every workspace crate | Cut off at 5 minutes in tier 3, as today's job would be for that change: the build was 151 s and xtask's 27 s |
| `.gitattributes` changed | 44439011586 (8 ms) | Restored nothing (the settings' hash in the key), built everything | Passed, 298 s |
| A test written into the tree after the checkout, not committed | 44440563206 (5 ms, a 12-minute limit) | Refused to date ("files that are not its commit's"), rebuilt everything | **Failed** on the test written after the checkout |
| The toolchain changed (1.99.0) | 44440580488 (7 ms, a 12-minute limit) | Restored nothing (the toolchain's hash in the key), built every crate, third-party ones included | Passed, 388 s: a toolchain bump is a cold build, as it is today |

- **Every "Failed" above is the job failing as it must:** the change was rebuilt, and the stale build was not used.
- **The first runs** (44434778911, 44434797121) found xtask's `dev` build of `cineris-protocol` missing from the paths: xtask and it were rebuilt, 20 s. The paths now hold the `dev` build of every workspace crate.

### What it came to

The A/B, five rounds of each on one runner, is in [ci.md](ci.md), "One runner, both ways: the A/B". Each figure is the job, with the parts both sides pay alike:

| The pull request changes | Runner | Today | Cached |
|---|---|---|---|
| `cineris-tools` | 8 ms | about 300 s | **about 201 s** |
| `cineris-cli` | 6 ms | about 236 s | **about 127 s** |
| `cineris-testkit` | 6 ms | about 227 s | **about 184 s** |

- **On a slow runner, a change to a crate in the middle runs about 3 min 21 s**, under the 4 minutes aimed at. The three runs in a row on the scratch branch (8 ms, 6 ms and 8 ms runners) took 199, 177 and 203 s; the second failed on the known flake `a-verify-test-waits-on-a-timer-nothing-advances`.
- **The tiers are unchanged,** within a second, on one runner.

### The known slow cases

- **A change to `cineris-base` or `cineris-testkit`** rebuilds nearly every crate: on one runner, 72 s and 52 s of an 82 s full build.
  - In the A/B ([ci.md](ci.md)) a `cineris-testkit` change took 145.9 s of command against 193.4 s today, on a 6 ms runner.
  - That is about 240 s of job on an 8 ms runner, and about 4.3 minutes for `cineris-base`.
  - The coordinator accepted it as a known case.
- **A lockfile change that reaches a crate low in the graph** rebuilds third-party crates and every workspace crate. That is today's cost too, and above five minutes on a slow runner (44436680815).
- **A fallback** (no build restored, or the dating refused) pays for the restore, 5 to 9 s for 337 to 363 MB, on top of today's full build.

### The cache's cost

On 2026-11-19, before this change, the repository held 10.90 GB in 60 entries against its 10 GB, every one written by `main`. Most of it was rust-cache entries for earlier `Cargo.toml` hashes, which nothing reads any more:

| Key | Entries | In all | The newest |
|---|---|---|---|
| `v0-rust-test-Windows_NT` | 13 | 3.43 GB | 146 MB |
| `v0-rust-test-Linux` | 14 | 2.89 GB | 142 MB |
| `v0-rust-lint-Windows_NT` | 13 | 1.70 GB | 89 MB |
| `v0-rust-lint-Linux` | 14 | 1.49 GB | 83 MB |
| `workspace-build-Windows` (new) | one a push to `main` | — | 353 to 363 MB |

- **What a key costs:** the four rust-cache keys in use, about 460 MB together. A `workspace-build-Windows` entry is 353 to 363 MB, one for each push to `main`; Windows alone, since the Linux test job is well inside its five minutes.
- **How old entries go:** GitHub evicts the least recently used first, when a save takes the repository over its 10 GB, and drops what nobody has read for 7 days.
  - A `workspace-build` entry is read only while it is the newest. Once the next push to `main` saves another, it is never read again, and it goes before anything that is read.
  - The rust-cache entries in use are read at the start of every run, on every branch, so they are never the least recently used while the repository is active. The scheduled run twice a week reads them when it is not.
  - The stale rust-cache entries (about 9 GB now) go first of all.
- **Deleting superseded entries from the job** would need `actions: write` in a job that builds a pull request's code, which no job here has. LRU is enough, and keeps the permissions as they are.

### The review

An independent reviewer (a sub-agent on Opus, which had not seen the reasoning) read the whole diff against the specification before the first push. Its findings, and what was done:

- **The cache rule missed steps written in other valid layouts:** items level with `steps:`, a comment after the key, a dash on a line of its own, action names in another case, a condition read from an input's text.
  - Fixed: the steps are read as YAML lays out a block list, and steps on one line are refused.
  - Names are compared without case; `if:` is the step's own key.
  - Any other cache action, and a setup action's `cache:`, are refused.
  - The repository's actions (`.github/actions/**/action.yml`) are held to the pin and cache rules too.
  - Tests: `a_cache_written_is_found_however_the_steps_are_laid_out`, more cases in `only_the_main_branch_writes_a_cache`. Breaks `cache-names-by-case`, `cache-condition-from-an-input`, `cache-steps-level-with-their-key`, `cache-actions-not-read`.
- **`pull_request_target`, `workflow_run` and `issue_comment` run a pull request's code with `github.ref` the main branch's**, so a main-only condition holds there.
  - Fixed: a workflow they start writes no cache at all.
  - Test: `no_cache_is_written_where_a_pull_requests_code_runs_as_main_nor_by_an_action`. Break `cache-written-where-a-pull-request-runs-as-main`.
- **A change to `.gitattributes` alone changes the bytes a checkout writes, not any file's content in git** (confirmed by the reviewer on a scratch repository), so the dating could take a file a build reads as unchanged.
  - Fixed: git's line-ending settings and every attributes file are in the key, and the script also refuses when an attributes file changed.
  - Tried on a runner (44439011586, above). Break `dates-after-an-attributes-change`.
- **Nothing tested the dating script.** Fixed by two system tests in `xtask/src/checkout_dates.rs`, which run it with the bash CI runs it with (Git's on Windows, found beside git: `bash` on the path there is WSL's, which the A/B harness fell into once).
  - One test checks what is dated now and what long before.
  - The other checks each case that must leave every date alone.
  - Five breaks: `dates-a-stray-file-with-the-rest`, `dates-nothing-now`, `dates-only-modified-files`, `dates-the-wrong-part-of-the-key`, `dates-after-an-attributes-change`.
- **The records were unfinished:** finished here.
- **The fallback's cost was not measured:** it is now, in the cases above (298 s for a full build after the restore on an 8 ms runner) and under "The known slow cases".
- **On Windows the current directory may be `c:\…` where cargo wrote `C:\…`**: the note said "built elsewhere" and no last good copy was kept.
  - Fixed: `workspace::same_tree` compares as NTFS does (case folded, `\\?\` taken off).
  - Test: `two_spellings_of_one_checkout_are_one_tree`.
- **`./x --root ../other` was read from `./x`'s own folder**, which it changes into. Fixed: a relative `--root` is read from where `./x` was run (tried by hand with `CARGO=/bin/echo`).

The breaks, run after the fixes, found one more way round the rule: a condition that took the main-branch test inside parentheses or a function, such as `(a && github.ref == 'refs/heads/main') == false`.
- The conditions are now read at the top level only, which makes the separate refusal of `!` unneeded; `||` is still refused, since `main && a || true` holds on any branch.
- Break `cache-condition-inside-parentheses`.
- `cache-the-combined-action` survives by design: `actions/cache` is refused by the unknown-action check as well, and the break changes only the message.
- `mutate tooling-11`: 20 breaks, 20 as the list expects.

### Left open

- **The three real pull-request runs** read a cache only the main branch writes, so they came after the merge, from the next pull requests, by the coordinator's decision: 222 s (8 ms), 176 s (7 ms) and 231 s (6 ms, a change to `cineris-base`), all passed. They are in [ci.md](ci.md), "After the merge".
- **A flake found on the way, outside this step:** `cineris-sandbox`'s `protect::tests::a_cargo_made_as_a_link_is_found` depends on the watch's 250 ms of real time. Opened as `a-sandbox-watch-test-depends-on-real-time`, for the maintainers; settled by the [small fixes](../small-fixes/answered.md), whose tests give the watch a time no machine reaches.

## xtask's root

Item `xtask-is-rooted-where-it-was-built`, found in step 25, is settled here, and its file deleted.

- **What was wrong:** `workspace::root()` was the parent of `env!("CARGO_MANIFEST_DIR")`, the folder xtask was compiled from. With one `CARGO_TARGET_DIR` shared by several checkouts, cargo ran the first checkout's build of xtask from every one of them, and every command that works from the root (`links`, `rules`, `outstanding`, `rename --check`, `mutate --check`, `branch`, and the rest: 70 calls) checked, or wrote, the first checkout's tree. A binary built in one tree and run in a copy of it did the same to the original.
- **What it is now:** `main` chooses the root once, before any command runs, and `workspace::root()` gives it.
  It is the folder `--root DIR` names (`cargo xtask --root DIR <command>`, before the command's name), or else the nearest folder at or above the current directory that holds a `Cargo.toml` with a `[workspace]` table beside `xtask/Cargo.toml` (`workspace::find_root`).
  That is git's top level in a checkout, and it needs no `.git`, so `rx`'s copy and a `mutate` copy are their own roots. Another project's workspace inside the tree (the fuzz targets') and a crate with an `xtask/` folder by chance are not roots.
  Outside every checkout xtask refuses, and says to run it from inside one or to give `--root`: falling back to where it was built, as the item suggested, would bring back the silent case.
  When the root is not the checkout the binary was built from, xtask says so on standard error (`working on …; this xtask was built from the source in …`): the code it runs is still that build's.
- **The last good copy** (`target/xtask-last-good/`) is kept only by a checkout's own build: a binary built from another checkout's source is not this checkout's last good xtask.
- **`ab`'s `normalised`** moved to `workspace`, which needs it too; `ab`, `perf-ab` and `like-runner` use it from there, and the break `tooling-3/ab-base-compared-with-its-dots` names it there.
- **Tests:** `workspace::tests::a_leading_root_option_is_taken_and_nothing_else`, `the_root_is_the_nearest_checkout_at_or_above_the_current_directory` (a copy under `target/`, a fuzz workspace, a crate with `xtask/`, no checkout), `a_root_that_is_named_is_taken_from_where_xtask_runs_and_must_be_a_checkout`, and the system test `sys_a_build_from_one_checkout_works_on_the_checkout_it_runs_in`.
  The system test runs this test binary, built in this checkout, as xtask (`main_of`, through the ignored `as_xtask`), in a checkout of its own made in the system's temporary folder. `outstanding` from a folder inside it writes that checkout's index and leaves this one's as it was, and says whose source it was built from. From outside every checkout it is refused, and `--root other` from there passes.
- **Breaks:** `mutations/tooling-11.breaks`. All four were caught: the root taken from where xtask was built (two tests), any workspace taken as a checkout (one), `--root` ignored (two), and outside every checkout falling back to the build's checkout (one).
- **Documented** in [CONTRIBUTING.md](../../../CONTRIBUTING.md) ("The commands") and in `cargo xtask help`.
