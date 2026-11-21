# Step 28, its parts: extensions found and pinned

The first part of step 28's second pull request ([answered.md](../answered.md), "The design, before building", point 10, "Extensions in a session"): the extensions a session finds itself, each checked against its pin, what each registered taken from the registration cache, the project's extensions in the trust question, `cineris.state`, and a change seen at the next turn. Skills and sections 6 and 7 of the system prompt are planned for the part after it, and hooks for the one after that.

## What was built

- **The user's own pins** (`cineris-policy`'s `trust::PinStore`).
  `pins.toml` in the state directory, `version = 1`, one `[[pin]]` of kind, name, hash and time each, written owner-only and atomically under `pins.toml.lock` and read strictly, a kind this build does not know kept and written back.
  It is a file of its own, so that a build that reads `trust.toml` up to version 2 is not made to refuse it ([06-permissions-and-sandbox.md](../../../architecture/06-permissions-and-sandbox.md), "Project trust").
  `TrustStore::pinned_hashes` gives every trusted project's pins of a kind, for pruning the cache.
  Beside it: `cineris_ext::source::Source`, the user's or the project's, which keys an extension's pin and its values; and `[extensions.<name>] enabled`, the user's alone, taking effect at the next session.
- **Discovery and the registration cache** (`cineris-ext`'s `discover` and `cache`).
  Each folder directly under the user's `<config dir>/extensions/` and the project's `.cineris/extensions/` read whole, its manifest giving the folder's own name; roots judged where they lead, a project's inside the workspace; no link followed into an extension's folder; a root that is not there ruled out with a look or two, before the path resolver is asked; names that differ only in case refused within one source.
  The cache, `<state dir>/extensions/registered.json`, by the hash of each folder, read strictly and set aside whole when it cannot be read in full, written owner-only and atomically, pruned of the hashes no pin names ([12-extensions.md](../../../architecture/12-extensions.md), "Tier 4: script extensions").
- **`cineris.state`** (the core's `ext::state`).
  `session` values as the log's `ext` records, which a rewind leaves and a fork copies as they were at its point (`cineris_session::Log::fork`); `project` values in `projects/<project key>/extensions/<source>-<name>.json`, read strictly; 64 KiB a scope, a key of 1 to 256 bytes, a hundred sets a call and 16 MiB of `session` records a session ([decision 0041](../../../decisions/0041-an-extension-s-state-is-kept-in-the-session-s-records-and-a-project-file-read-strictly-within-fixed-bounds.md)).
  `CoreOptions::ext_session_budget` is a test's, which only tightens the 16 MiB.
- **The core's pipeline and the trust question** (`ext::found`, `assemble::look`).
  A task each session starts once its loop runs: the folders found, every standing decided against `pins.toml` and `trust.toml`, approved extensions' tools taken from the cache, one the cache misses evaluated once and the first turn held for it, the cache written and pruned after.
  The trust question lists a project's extensions in words from their manifests and pins them as it read them; `cineris permissions trust` shows them as the question does and pins them on a yes at a terminal, or with `--with-extensions` where there is none.
  The interactive application's dialog names a changed extension apart from changed rules, and says what a keep of one does; the protocol's doc comments say the same.
- **Hot-reload** (`ext::watched`).
  What the extensions were found in, stamped without following a link and looked at in order at each turn's start; a change has every folder found and judged again, an extension that no longer stands approved withdrawn and one that now does loaded before the turn ([12-extensions.md](../../../architecture/12-extensions.md), "Hot-reload").
- **The break lists:** [mutations/step-28-pins.breaks](../../../../mutations/step-28-pins.breaks), [step-28-found.breaks](../../../../mutations/step-28-found.breaks), [step-28-state.breaks](../../../../mutations/step-28-state.breaks) and [step-28-pinned.breaks](../../../../mutations/step-28-pinned.breaks).

**Settled by the maintainers during the build:**

1. `cineris permissions trust` pins a project's extensions.
   It prints each with what it may do, in the question's words, and pins them only on a yes at a terminal, no being the default; at a terminal `--with-extensions` is ignored, so that nobody scripting at a terminal expects the flag to answer.
   Where there is no terminal to ask at, it pins them only with `--with-extensions`, and otherwise trusts the rules and says the extensions were left unapproved.
2. *Keep trusting it, without what is new* holds for an extension for the process alone, with no field of `trust.toml`: the next start asks again.
3. `--trust-project` approves no extension that is not pinned, and the session's notice names `cineris permissions trust`.
4. A yes the trust store could not keep approves the extensions the question listed, by the hashes shown, for that session alone (`ForTheRun::extensions`), the early review's recommendation.
5. The finding's task ends with its session and with the core; a session closed while its registration waits neither blocks nor leaks into the next session of the core (`extensions_trust::a_session_closed_while_its_registration_waits_neither_blocks_nor_leaks`).
6. Within one source, extensions whose names differ only in case are all refused, on every system (a Windows test on a case-sensitive folder, on a runner).
7. `cineris.state`'s bounds and where its values live: [decision 0041](../../../decisions/0041-an-extension-s-state-is-kept-in-the-session-s-records-and-a-project-file-read-strictly-within-fixed-bounds.md).
8. Extensions are found off the path that opens the session, and the start-up tests count what it reads, with no extension and with some (below).

## What was measured

| What | Measured | Where |
|---|---|---|
| Discovery with no extension | Neither root there: 1.22 µs. A configuration directory and a `.cineris` with no `extensions` in it: 1.74 µs. Both roots resolved through the path resolver instead: 20.7 µs. The user's root there and empty: 23.2 µs. One small extension: 32.2 µs. A release build, on an otherwise idle reference machine, best of 5 | `cineris_ext::discover`, measured during the build; no `perf_` test keeps it |
| Opening a session with no extension | Nothing of an extension's read, no root resolved, no runtime made, and neither the pins nor the cache written. The session's finding, after the open: one look at the user's folder. The trust question's look, for a client that answers it: one at the project's, two with a `.cineris` and no `extensions` in it | `cineris-core::it extensions_reload::with_no_extension_a_session_looks_once_a_folder_and_reads_nothing` |
| Opening a session with extensions | No extension's folder read on the path that opens the session but a project's, which the trust question reads once for a client that answers it and the session takes on; each folder read once | `extensions_reload::with_an_extension_nothing_of_it_is_read_until_the_session_is_open`, `extensions_trust::each_folder_is_read_once_when_a_session_opens` |
| The first turn's wait on a cache miss: one evaluation of the extension | Medians 0.34 ms for an extension of one tool, 0.50 ms of 16 and 0.98 ms of 64; 95th 0.41, 0.58 and 1.04 ms. A release build, on an otherwise idle reference machine, 40 runs. Misses are registered one after another | Measured during the build; no `perf_` test keeps it |
| A turn's start | One look (`lstat`) at each place stamped: three with no extension (`pins.toml`, `trust.toml` and the user's folder), more with the way to the project's folder and each extension's folders and files | `ext::watched` |

## The early review

A reviewer who had not seen the author's reasoning read the trust question's entries for a project's extensions and the check of each extension against its pin, at their first complete state. The bar the maintainers set held: each finding was of medium weight or lower.

**What held:** one snapshot for the hash, the manifest, the evaluation and the cache's key; only an approved extension that the cache misses evaluated; a cache that cannot be planted, since each entry is keyed by the hash of what registered it; the rules of names and sources; the order of the checks; a yes pinning what was listed as it was read; escaping through `visible`; pruning by every trusted project's pins, and by nothing when a store cannot be read; a wait for the extensions that cannot miss their settling; paths compared by place on Windows.

| Finding | Fix |
|---|---|
| **The trust store was read twice** in the look that shapes the question: trust taken away between the two readings could make the question list no setting held back, while a yes turned them on. | The question is shaped by the one reading the session's trust was worked out from (`Trust::stored`); `extensions_trust::a_trust_taken_away_while_the_question_is_shaped_turns_on_nothing_unshown`. |
| **An author's description could pose as the harness's sentences** about what the extension may do, and a backtick in a manifest's entry closed the code it was shown as. | The detail says what the manifest lets the extension do first, and the description last, labelled as its author's and in quotes it cannot close; the manifest refuses a backtick in a program, a glob or the version, and a program or a glob of more than 256 bytes. |
| **A user's extension that was left out freed its name** for a project's, to which the user's rules about the name (`allow ext tracker/*`) would then apply. | A name is the user's whenever a folder of the user's has it, used or not, with ASCII's case folded, and every name is while the user's folder cannot be listed whole (`Discovered::names`, `cut`). |
| **The first turn waited for the cache** to be written and pruned. | It waits for the tools alone; `extensions_trust::a_turn_does_not_wait_for_the_cache_to_be_written`. |
| **Folders were read whole twice** on the path that opens a session. | The question only lists the user's folder for the names it takes, and the session takes on what the question read of the project's; `extensions_trust::each_folder_is_read_once_when_a_session_opens`. |
| **Rules that guard had no test:** a project marked never, a second session after not now, an unreadable `pins.toml`, escaping beyond the description, *keep*, `--trust-project`, a project's `[extensions]` entry. | A test each, in `cineris-core::it extensions_trust`. |
| **Lower points:** the dialog called a changed extension changed rules; notices named the wrong command; notices showed the state directory's files as absolute paths; a cache miss held the turn with nothing said. | The dialog names each by its kind; the notices name the trust question, or `cineris permissions trust` under `--trust-project`; the files are shown under `~`; a miss says "Registering the extension …". |

Its open call, a yes the trust store could not keep, was settled by the maintainers as it recommended: the extensions listed are approved for that session, by the hashes shown. The rest of its last point is for hooks, below.

## What is left open, and where

- **For the hooks part:** the wait for a session's extensions has no failed state; a changed extension's pinned registration is not kept, which its gating hooks need in order to fail closed ([answered.md](../answered.md), point 5); misses are registered one after another, each given up to the 10 s an evaluation is; a hook's host calls must reach `cineris.state` through the same store as a tool's, with the extension's source; in a hook, a host call the engine would ask about becomes a refusal.
- **`cineris ext add` is the third pull request's.** Until then nothing in the harness writes `pins.toml`, and the notice that a user's extension is not approved names that command.
- **The stamp gap.** What the extensions were found in is stamped after it was read, so a write between the read and the stamp is not seen until the next change. It is widest when the session takes on what the trust question read, which may have been open for a while: an extension changed while the question was open is loaded at the hash that was pinned, and its tools are offered. It holds because the runtime reads the folder again at the extension's first call and refuses another hash, so what changed is never run; the next session finds it changed.
- **A folder swapped for a link while `Snapshot::read` reads it** is followed, a narrow race. It is benign here, since what is read is used only if its hash equals a pin; the third pull request's `cineris ext add`, which pins the snapshot it shows, reads the folder through handles that follow no link.
- **The words for a `*` in `net`.** The trust question says "any host", where [a-fetch-reaches-the-machines-own-and-its-networks-public-addresses](../../../outstanding/a-fetch-reaches-the-machines-own-and-its-networks-public-addresses.md) recommends words that say it reaches addresses on the user's own network: for a later part of the step, with `cineris ext add`'s.
- **Lower points:** the folders discovery left out are said again in every session; the command a notice names is not quoted for a folder whose name has a space; no test has two writers change one project's file of `cineris.state` at once, nor checks that a `CoreOptions::ext_session_budget` over 16 MiB is held to it; a configuration directory on a share the path resolver does not allow leaves the user's folder out, with the resolver's reason.

## The full review

(to come)
