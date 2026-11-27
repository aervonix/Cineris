# Answered in the fixes to step 28, on 2026-11-27

Work outside the numbered steps. An independent review of step 28 (extensions I), on `main` at `5eda6204`, found 2 high, 8 medium, 3 low-to-medium and 6 low findings and a set of rules no test held. The maintainers chose which to fix before steps 29 (self-written extensions) and 30 (the MCP client) build on the code. The fixes come in three pull requests, each with a record of its own in [parts/](parts/):

1. **Variation selectors** ([parts/selectors.md](parts/selectors.md)): finding 6, first, since step 30's MCP descriptions come through `cineris_base::one_line`.
2. **The guards and the socket** ([parts/guards.md](parts/guards.md)): findings 1, 2, 3, 7, 8 and 17, and the tests of guard B's store-moving variables (19).
3. **The rest:** the gates and hooks (4, 5, 14: [parts/hooks.md](parts/hooks.md)), the approvals' words and the commands' front matter (11, 12, 15, 16, 18: [parts/approvals.md](parts/approvals.md)), the engine and `fetch` (9, 10, 13, and 19's untested rules: [parts/engine.md](parts/engine.md)), the build without the engine (19), finding 6's twins, and the lower-confidence notes (20), below.

The review's report and its tests are the maintainers'; each finding's test here is the review's test made a real one, or a better one, and fails on `main` at `5eda6204`.

## The findings and what became of each

| # | Finding | Outcome |
|---|---|---|
| 1 | `cineris project`'s changes have no guard B | Fixed, and `auth login`/`logout` and `stats reset` with them, with tests and breaks ([parts/guards.md](parts/guards.md)) |
| 2 | Windows: B's walk for a service's start falls open at its bound | Fixed, and its twin in the line of parents; shown on a Windows runner ([parts/guards.md](parts/guards.md)) |
| 3 | The RPC socket refuses only its own server's commands | Fixed: a peer under any harness is refused, the check passed in by `serve`; shown on both systems ([parts/guards.md](parts/guards.md)) |
| 4 | An approved project gate lapses when its entry changes, is removed or is turned off | Fixed: both gates held until the person approves it as it is, or takes its approval away where the trust question lists it as removed (`TrustStore::drop_pins`); the precise alternative, a cache of approved definitions, is an item ([parts/hooks.md](parts/hooks.md)) |
| 5 | A project's hook commands outlive its trust | Fixed: a live trust token; gates held, failing, while the store cannot be read ([parts/hooks.md](parts/hooks.md)) |
| 6 | A run of variation selectors survives `one_line` and `plain` into sections 6 and 7 | Fixed, with tests and breaks ([parts/selectors.md](parts/selectors.md)); its twins below |
| 7 | Linux: a harness that is process 1 is never seen | Fixed, and the stores' twin; shown on Linux 22.04 ([parts/guards.md](parts/guards.md)) |
| 8 | A pid namespace of the command's own hides every parent from guard B (Linux) | Not built: settled by the maintainers on the evidence below; stated in 06, security.md §3 and the guards' item ([parts/guards.md](parts/guards.md)) |
| 9 | The four-thread cap is not a cap | Fixed: a thread counts from when it is told to stop; the bound stated is four plus the calls and starts already running ([parts/engine.md](parts/engine.md)) |
| 10 | A fetch's lookup outlives it | Fixed: eight system lookups at a time in the process, a place freed when the lookup returns ([parts/engine.md](parts/engine.md)) |
| 11 | An extension's approval doesn't name its programs' environment | Fixed, on all four surfaces ([parts/approvals.md](parts/approvals.md)) |
| 12 | Indented front matter hides a command's `profile` | Fixed, and a key after a quoted value or a block ([parts/approvals.md](parts/approvals.md)) |
| 13 | A brace glob with an absolute path passes the manifest | Fixed: every alternative checked, braces that cannot be read refused ([parts/engine.md](parts/engine.md)) |
| 14 | `{path}` hands a user's hook a secret file before the question | Fixed as the owner chose: such a hook runs after the yes ([parts/hooks.md](parts/hooks.md)) |
| 15 | Skill-folder reads turned off can stay on | Fixed: the value in force is loaded and said ([parts/approvals.md](parts/approvals.md)) |
| 16 | A project extension's section 7 text isn't shown before approval | Fixed: shown whole, escaped ([parts/approvals.md](parts/approvals.md)) |
| 17 | Windows: `ProgramData` moves the managed configuration | Fixed, in A's and B's lists; shown on a Windows runner ([parts/guards.md](parts/guards.md)) |
| 18 | `ext remove`'s place check never refuses | The dead check removed and the name rules documented as the guard; the delete through a held handle stays with its item ([parts/approvals.md](parts/approvals.md)) |
| 19 | B's store-moving variables have no test | Tested, each variable on each system ([parts/guards.md](parts/guards.md)) |
| 19 | `fetch`'s user part and the 12 MiB of waiting host calls have no test | Tested ([parts/engine.md](parts/engine.md)) |
| 19 | The core's integration binary does not build without the engine | Built: three modules gated, a `no_engine` module only that build compiles, and the nightly `no-engine` job, outside the required checks, runs its tests and lints (below) |

## Settled before building

- **Finding 8: a pid or user namespace is not read as "not known".** A rule that refused under a pid namespace other than the system's first would refuse every person's own approvals under WSL 2, whose processes are all in such a namespace (checked on the reference machine: `/proc/self/ns/pid` is not the initial namespace's), and in every Docker or development container; one that refused under a user namespace other than the first would refuse every toolbox and distrobox user (rootless containers that share the system's pids). Nor can a parent of 0 tell the attack from a legitimate top: a namespace's own first process (WSL's `/init`, `bwrap`, a container's entry point) has a parent of 0 just as `unshare -Urpf` gives one. Ubuntu's restriction of unprivileged user namespaces does not stop the attack where `bwrap` is installed, since it has a profile of its own (Flatpak installs it): on the second machine `bwrap --unshare-user --unshare-pid` ran where `unshare -Ur` was refused, and a terminal or editor run under Flatpak is that shape for a person's own Cineris. So the gap is stated (06, [security.md](../../security.md) §3 and the guards' item), approvals signed with a key that needs a person close it by construction, and step 32's channel the kernel names closes the rest.

## Finding 6's twins, found by these fixes

The same channel, a variation selector after each character carrying a byte no reader sees, in other text that reaches the model. Settled by the maintainers, each with a test and a break:

- **What a script registers** (a tool's or a command's description, a tool's `input`) is refused when it holds a selector, named as its escape; the registration cache checks an entry the same way. An emoji's selector is refused too: `"⚠️ Deletes files"` is refused, `"⚠ Deletes files"` is taken (12-extensions).
- **A project's command** has its selectors taken out of its body and its arguments' defaults, an emoji's too. This replaces step 28's settled point that kept an emoji's selector in a project's command ([../28/parts/pr3a-commands.md](../28/parts/pr3a-commands.md)). The user's own commands are sent as written.
- **A script command's text**, sent as the user's prompt, has what acts unseen and every selector written as escapes.
- **What a hook writes** (its reason and context, a `session.start` hook's context, a replacement's note) has every selector written as its escape.
- **A repository's instruction files** (section 4 and the notice of a change) are read as an extension's text is: line breaks made `\n`, and what acts unseen and every selector taken out. A file that held one, or Windows line endings, has a new hash once, so a resume over a log written before is told the file anew (07, "Project instruction files").

## The build without the engine (finding 19)

The core's integration binary did not build with `--no-default-features`: three modules that need the engine (`extensions`, `hook_commands`, `hook_wiring`) were not gated. They are now, beside the review's two end-to-end tests made a module of their own, `no_engine`, which only that build compiles: an approved extension's gates and a user's refused hook command hold without the engine. Four dead-code expectations in `ext::manage` are met in that build's tests too. The nightly workflow's `no-engine` job, outside the required checks, runs `cargo xtask test -p cineris-core --no-default-features --features cineris-core/mcp,cineris-core/sandbox` (564 tests in a local run) and lints that build ([testing-and-ci.md](../../testing-and-ci.md), "Not blocking"). The blocking Windows jobs are untouched.

## Settled during the fixes

- **Finding 4's way out** (the maintainers, on the hooks part's evidence): a pin keeps only a name and a hash, so a changed or removed project hook's approved event, tools and `observe` are not known, and its held gates are both gating events on every call. A removed one had no way out but `untrust` and `trust`, since `TrustStore::approve` only adds pins. The trust question and `cineris permissions trust` now list such pins as removed, saying what each was and that a yes takes its approval away, and the yes drops the pin through `TrustStore::drop_pins`, which drops only a pin still at the hash listed. The same lists a removed project extension, whose pin `approve` used to keep. A cache of approved hook definitions, which would hold only the approved gates, is the item `an-approved-hooks-gate-is-held-on-every-call`.
- **Finding 14's reach** (the maintainers, with the pre-push reviewer): every hook command gets the call's arguments, the file's name among them, on its standard input, and a user's own hook can read what the user can. So the deferral keeps `{path}` from pointing a program at a held-back file before the yes, and is not a confidentiality boundary; deferring every gate on such a call would lose the gate's deny before the question. security.md §3 and §12 and 12-extensions say so.

## Open items these fixes add or change

Added: `an-approved-hooks-gate-is-held-on-every-call`, `a-compaction-focus-does-not-pass-prompt-submit`, `threads-left-running-past-the-cap-by-calls-already-running`, `system-lookups-wait-in-turn-behind-names-that-never-answer`, `an-extensions-progress-is-shown-only-when-its-tool-call-ends`, `an-extensions-out-of-memory-is-told-by-its-text`, `an-extensions-tool-running-alone-has-no-end-to-end-test`, `extensions-status-without-a-session`, `rough-edges-of-step-28s-custom-commands`, `a-gone-users-extension-frees-its-name`, `the-trust-store-is-read-through-a-link-at-its-name`, `the-first-runs-mode-line-names-a-mode-a-managed-lock-may-override`. Changed: `a-commands-settings-profile-for-its-turn`, `a-file-is-opened-by-name-after-its-decision`, `a-folder-is-deleted-by-its-path-after-its-check`.

## Corrections to step 28's own record

Step 28's [answered.md](../28/answered.md) is its record and is left as written. Two of its statements are corrected by these fixes: the Threads budget row's "a cap of four" is four plus the calls and starts already running when the fourth was counted (12-extensions, "Limits"); and point 2's "200 characters, the latest kept" for `cineris.progress` is 200 bytes a line, the last 20 lines, shown when the call ends (12-extensions).
