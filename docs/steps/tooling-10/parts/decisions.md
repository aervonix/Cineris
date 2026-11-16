# The tenth tooling step: the owner's decisions of 2026-11-16, put into the repository

A pull request of its own, after the step's tools, made at the coordinator's request. One sentence a line. Each change is a commit of its own.

## A managed `ask` holds back the user's own check

- **The owner's choice** (the item `a-managed-ask-and-the-users-own-check`, now settled): an `ask` rule of the managed configuration holds back a line of the user's own `verify.commands`, as a managed `deny` does; a user's own `ask` rule, or any other `ask`, still lets the user's line run, as step 19 built it.
- **How:** the engine marks an `ask` that an `ask` rule decided, with whether the rule is the managed configuration's (`Finding::asked_by`), and the question carries it (`Question::managed`: whether a managed `ask` rule is among what asks).
  The verifier asks the engine for the question only when the decision is `ask`, and refuses the user's line when it is managed, saying that an administrator's rule asks and a check never asks.
  The asks a rule makes for a path a command only names, or for a pattern or a search that may reach one it holds back, are marked too when the rule is the managed configuration's, whether it is an `ask` or a `deny` (which asks of a path only named): so a managed `deny` holds the user's line back there as well.
  Of two rules of one strength on one call, the managed configuration's is the hit (`RuleSet::strongest`), so a managed `ask` beside a user's `ask exec *` is still the one that decides.
- **Tested:** `cortex-core::it verify::a_users_check_a_managed_ask_rule_asks_about_does_not_run_and_a_users_own_ask_does` (the managed file's `ask` holds the line back with its note; the same rule in the user's file lets it run; nothing is asked), and `cortex-policy::it managed_ask` (the mark through an exec rule, beside another reason, on a path read, removed or only named; a managed `ask` beside another layer's in either order; no mark for the user's or the project's rules). Breaks: `mutations/tooling-10-decisions.breaks`.
- **The review** (independent, of the whole pull request) found that a managed `ask` lost to another layer's `ask` on the same call (the last of rules of one strength was the hit), and that the asks for a path a command only names were never marked: both fixed, with the engine's tests above and a break each. It found the step 19 record's Q7 and a line of the build plan still saying `verify/run` answers `not_available`, and the seventh tooling step's record with no word of the reversal: corrected. Left: the verifier asks the engine twice for a line that would ask (a decision, then the question), which runs the policy's hook again; harmless, and a single call returning both would be cleaner.
- **Documented** in 15-efficiency ("Automatic verification"), `security.md`, and step 19's record.

## Records of the owner's decisions

Each decision of 2026-11-16 below was already built; the records that still called it pending, reversible or with the owner now say "decided by the owner on 2026-11-16, as built", and keep what was measured and asked.

- **Step 17's load of a project's knowledge at its caps:** at most 5 ms on the desktop and 15 ms on a hosted runner, with the caps kept.
  `performance-budgets.md`, "Data paths", loses the row's pending mark; step 17's `answered.md` and `ci.md` say it was decided.
  The item `the-load-of-a-full-profile-on-a-hosted-runner` is settled and its file deleted; step 17's link to it is plain text now.
- **The eighth tooling step's budgets, set as proposed, per byte of the stream:** three new rows in `performance-budgets.md`, "Data paths".
  Search of the pinned corpus of real code: at the median, at most 100 ms on the desktop, 400 ms on a Linux runner and 500 ms on a Windows runner (Windows measured 316 to 382 ms).
  A re-read's diff: at most 2 ms at the median and 5 ms at the 95th percentile on the desktop, 10 ms on a runner.
  Decoding: at least 100 MB/s of the stream on the desktop and 80 MB/s on a runner.
  The item `perf-budgets-for-search-diff-and-decoding` is settled and its file deleted; the eighth tooling step's link to it is plain text now.
- **The `perf_` tests are held to the runner's figures,** which a desk also meets, since a test cannot tell a desk from a runner.
  `cortex-tools`' `perf_search_real_code` fails when the rare literal's, the regular expression's or the miss's median is over 400 ms on Linux; on Windows it reports them against 500 ms and does not hold them (the review: a runner measured 305 to 320 ms, and Windows runners differ twofold).
  The literal on a line in nearly every file takes about a third longer than the other three, which the budget's figures were measured from, and is reported, not held, so that it does not fail a Windows runner near the line.
  `perf_diffing_a_reread_of_real_code` fails when a changed re-read's median or 95th percentile is over 10 ms.
  `cortex-llm`'s `perf_decoding_a_large_tool_call_of_real_code` fails under 80 MB/s of the stream, and reports the rate as `llm.decode_large_call_real_code_rate`.
- **Measured on the desktop on 2026-11-16,** a release build through `rx`, all three passing: search 19.7 ms (rare literal), 19.9 ms (regular expression), 18.8 ms (miss) and 25.6 ms (common literal), at the median.
  A changed re-read 0.38 ms at the median and 0.88 ms at the 95th percentile, 0.20 ms unchanged; decoding 7.84 ms, 278 MB/s of the 2.18 MB stream.
  The run: `rx -- cargo xtask test -p cortex-tools -p cortex-llm --cargo-profile release --run-ignored only -E 'test(/perf_search_real_code|perf_diffing_a_reread_of_real_code|perf_decoding_a_large_tool_call_of_real_code/)' --no-capture`.
- **On the runners** (`try` 43943290230, `perf -p cortex-tools -p cortex-llm`, both systems, all passing): search 85 to 92 ms on Linux and 305 to 320 ms on Windows; a changed re-read 0.47 ms (95th 1.33 ms) on Linux and 0.86 ms (1.91 ms) on Windows; decoding 166 MB/s on Linux and 161 MB/s on Windows.
- **Step 19's first two questions,** decided as built: `verify.commands` the user's alone (a departure from 08-configuration's row), and the user's own check running unconfined where nothing confines commands.
  Step 19's `answered.md` drops "reversible" and "to the owner" from both, and `security.md` section 1 no longer asks the owner to confirm the trade-off.
- **The re-decide step's tightening,** deciding every call again, reads included, decided as built in its `answered.md`.
- **The step 23 fixes' answers,** decided as built: a command line's prompt that is a slash command does what typing it does, every `file:` link is refused, screens 2a and 2b stay as composed, and a project's `shell.env_remove` keeps its class, *stops commands*.
  Step 23's `answered.md` drops "reversible" and "pending the owner's word" from them.
- **Step 17's observed-command trade-off,** an observed command reaching the prompt before anyone reviews it, decided as built in step 17's `answered.md`.
- **Left as they were:** the item `caught-by-names-in-a-blocking-job`, which stays open; step 18's reversible coordinator decision; step 27's records; and 14-adaptation's "Reversible", a design principle.
