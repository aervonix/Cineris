# Answered in step 24, on 2026-11-16

Step 24 builds the panels and live views: the panel mechanism and the first-release panels (plan, with editing; review; usage; project), `/context`'s breakdown, hot-reload of themes, key bindings and settings wired end to end through `config/reload`, and the `/usage`, `/plan`, `/project`, `/profile`, `/verify` and `/reload` commands. The coordinator's answers, and the owner's, are added here as they come.

## The design, before building

Written against `main` at `7f82023`, and revised after its independent review (the findings and what each changed are at the end). The specification is the build plan's step 24 with its "From step 23" bullet, the "What later steps inherit" bullets that name step 24 (under steps 16, 17, 19, 21, 22 and 23), the features table's rows for step 24 (prefetch, the plan, `/usage`, `/context`), [09-tui.md](../../architecture/09-tui.md) and screen 4 of [tui-look/](../../architecture/tui-look/README.md), [08-configuration.md](../../architecture/08-configuration.md), [14-adaptation.md](../../architecture/14-adaptation.md), [18-usage.md](../../architecture/18-usage.md) and [15-efficiency.md](../../architecture/15-efficiency.md). Every choice below is built as recommended while the coordinator and the owner answer; each marked **(reversible)** can be changed without undoing the rest. A choice that hangs on a number says **to confirm by measurement**.

### 1. `config/reload` through the core

**The configuration in force is a mix.** Each open session keeps two configurations: the one **in force**, which its parts were made from, and the one **last loaded**. A new function of `cortex-config`, `Loaded::taking(&self, newer, keys)`, makes a configuration in force from the one before with the named keys taken from the newer, by the schema's own declarations (no field is copied by hand), and checks the result as a load is checked. A reload takes the `Now` keys at once; the `NextEpoch` keys when the session's next compaction begins; the keys of the new effect `next session` (below) and the `Restart` keys never, for that session. The core's own configuration (the user's, with no project: what `config/get` answers without a session, and what `usage/*`, the statistics' writer and the day's purse read) is a mix the same way, so a reload never moves the data directory, the sessions' root or `stats.enabled` under a running core.

**The order of a reload, for each open session and for the core's own:**

1. **Look.** The configuration's files are stamped (`LiveConfig`), **and so is the trust store's file**, so that `cortex permissions untrust` in another terminal is seen by the next check. With `changed_only`, a look that finds nothing changed stops here: a few `stat` calls (22 µs measured in step 9).
2. **Judge trust first,** with the code a start uses (`assemble::trust` and `pinned_rules`): the store read again on the blocking pool; for a project the store trusts, a look-ahead load with the project trusted and its rules compared with the pins. That answer decides the trust of the one load that follows. **Nothing of a look-ahead load is ever in force, or compared, or listed.** `--trust-project` holds for the core's life, as today.
3. **Load once** with that trust (`LiveConfig::update`). A file that does not load leaves everything as it was, and its error is reported once; `/reload` says a standing error again rather than "nothing changed". **Lowering trust takes effect even when a file is broken** (`Reload::TrustLowered`).
4. **Check the session's reload whole.** Everything the changes need is built before anything is applied: the role models with their credentials, the day's account, the engine, the launcher. A failure (a new `key_env` whose variable is not set, a day's limit with statistics off) refuses that session's reload with its error, and its configuration in force stays, as `session/set` checks first.
5. **Apply** (the table), then **tell** (below).

**What the person chose in the session outranks the files,** as the command line does: the level, models, reasoning levels and speeds set with `session/set` (`/mode`, `/model`, `/effort`, fast mode) are recorded in the session's load options as overrides when they are set, so a reload never undoes them; and a role the person sets with `session/set` is dropped from what waits for the next compaction. `/profile` is the exception: switching profile starts from that profile, as `--profile` would, so it drops those choices, and the notice names each one it replaced **(reversible)**.

**Lowered trust takes everything trust gave,** at once: the engine's two answers (`trusted`, `repository_trusted`, in a new engine); the verifier's `trusted`; whether git may read the repository's own configuration, for the shell's look at what a command changed and for a base capture still under way (today a `bool` copied into each; it becomes one flag of the session's, which both read); the project's write roots and network in the sandbox's grant (next table); and the project's settings that need trust.

**Where each `Now` key lands, and when a running turn sees it.** A running turn sees only `Now` keys, each at a boundary: a request in flight, a tool call already decided, a command already started and a verification round under way keep what they began with.

| Keys | Applied by | A running turn sees it |
|---|---|---|
| `permissions.mode`, `permissions.rules` | A new engine from the session's template with the new rules, level and trust, its session approvals and learned rules carried over, the lease recorded again in the log, as `set_level` does. | At the next permission decision. |
| `sandbox.write`, `sandbox.network` | The new grant in the engine, and the session's launcher made again from it (the same temporary directory and protected paths), in place of the old one, for commands started from then on. | At the next command. |
| `limits.*` | The ledger's budgets changed in place, what was spent kept: the session's, the turn's (the running turn's included) and the day's (an account made from the statistics where there was none). | At the next request: a limit already passed stops the turn there, as a limit reached does. |
| `retry.*`, `providers.*.key_env`, `headers`, `idle_timeout`, `first_event_timeout`, `model.speed`, `speed_fallback`, `speed_wait`, `speed_cooldown`, `speed_cooldown_min`, `model.roles.*.speed` | Each role's model built again (`Providers::role_model`) **from the configuration in force**, so the `NextEpoch` keys it also reads (the variants, `adapt.variants.*.effort`, `model.reasoning`, a provider's `enabled`, `protocol` and `base_url`) keep their values; then `set_role_model`, `set_speed`, `set_role_speed`. The model, its provider, protocol and effort are unchanged, so no epoch begins and the cache is kept. **A variable a new `key_env` names is taken from every command at once:** the shell's list of variables to remove holds the providers' key variables as a live part. | At the role's next request. |
| `context.auto_compact`, `background`, `compact_at`, `prepare_at`; `verify.rounds` | A new `SessionHandle::reconfigure`, which the loop takes when it prepares a request, as it takes a change of speed today. | At the next request; `verify.rounds` at the next round. |
| `verify.mode`, `commands`, `round_time`, `turn_time` | The core's verifier, whose settings (with `trusted` and whether a project's file lowered the mode) move behind a lock it reads when a round begins: the seam step 19 left. | At the next round. |
| `log.level` | The process's log filter (`LogHandle::set_filter`, given to the core in `CoreOptions`), unless `--log-level` or `CORTEX_LOG` set it. | At once. |
| `usage.week_start`, `usage.ranges`, `usage.refresh`, `stats.retention` | The core's own configuration. **Their effect changes from `restart` to `at once`** in the schema and the reference, as steps 16 and 23 asked; the statistics' writer reads the retention when it prunes. | Not a turn's: the next report, refresh or pruning. |
| `tui.*`, `keys.*` | The interface (point 5). | — |

**`NextEpoch` keys wait for the next compaction.** They shape the prompt, the tools or the model, and only a compaction builds the prompt again within a session: a change of model keeps the prompt and the tools exactly ([07-sessions-and-context.md](../../architecture/07-sessions-and-context.md)), and taking them at a turn's start would make the prompt differ and begin an epoch at once. So they are listed `pending`, and handed to the loop in a slot (`SessionHandle::at_next_epoch`) that it takes **when a compaction begins, before it chooses what to keep** or uses a summary prepared in the background (a summary prepared with the old `keep_recent_tokens` is asked again): the role models built from the configuration in force with those keys taken, and the context's `keep_recent_tokens`, `instruction_names`, `instructions_cap`, `project.profile_tokens`, `index.map` and `map_tokens`. A session opened again takes them all, being a new load. A second reload before then replaces what waits. **The variants follow this rule too:** a `Now` rebuild never changes a model's variants (they are `NextEpoch` keys, held), so the case [an-epoch-read-without-its-variants](../../outstanding/an-epoch-read-without-its-variants.md) describes, a rebuilt model whose variants differ, can arise only at the compaction that takes them, which is the new epoch it asks for.

**`next session`, a new effect,** for the keys a session is made with once, which no epoch of a running session can take: the sandbox's switch and protected paths (`sandbox.enabled`, `sandbox.protect`: whether the sandbox enforces is decided as the session is put together, and a changed set of protected paths would break the continuity of their looks), the shell (`shell.*`: the tool, its runner and its line in the system prompt), the compatibility readers (`compat.*`) and the map's wait (`index.map_wait`). Declared in `sections.rs`, with the reference regenerated, so that what the reference says is what a reload does. They are listed `pending` with the sentence "in sessions opened from now on" **(reversible; the coordinator's call: 08 calls `sandbox.enabled` and `protect` "at once" and the others "next epoch")**.

**What a reload can never do,** and the tests that hold it, each against the real core:
- **Apply a project's setting before trust.** A trusted project's file gains an `allow` rule while a session runs: after `config/reload` the rule is not in the engine, not in the configuration in force, and not among the keys applied, and a hold point between the look-ahead and the load shows nothing of it in force there either. Untrusted, a higher `model.effort` and `sandbox.network = "full"` are held back and listed; trusted afterwards, the next reload applies them.
- **Loosen a user-only or tighten-only key from a project's file:** a project's `usage.refresh`, `sandbox.enabled`, a higher `limits.turn.cost`, a first limit untrusted, and a higher `shell.timeout`, each refused with its warning, as a start refuses them.
- **Keep what trust gave once trust is lowered:** `cortex permissions untrust` from outside, then a `changed_only` check: the project's allow rule, write root and network are gone from the next decision and the next command, git no longer reads the repository's configuration, and the verifier runs nothing; with a project file broken at the same moment, the lowering still holds.
- **Leave the session without a configuration,** or **half-apply** a reload (step 4).
- **Undo a choice made in the session** (above).

**Who may reload.** Any client may ask for a reload: it reads only the files of the user and the project, as they are. **`profile` only from a `user` client,** as only a `user` client may raise a session's level, which a profile can (11-rpc-protocol, `session/set`).

**What the person is told.** A new event, **`config.changed`**, to every client of each open session a reload changed: the keys applied, those pending, the errors and the sentences (`notes`): what waits for the next compaction, for a new session or for a restart, and why trust was judged differently. The interface draws it as a notice through `sanitize`, since it carries values from files. It is not given to the model. A reload that changed nothing sends no event; `/reload` then says "Nothing changed.", or the standing error of a broken file. The result of `config/reload` holds the same for the client that asked, its lists the union over the sessions.

**Who asks, and when** (08's four moments): the core looks before taking each turn's prompt; the interface asks on a focus-in and on a key press at most once a second (`config/reload` with `changed_only`, after the key's own frame is drawn); and `/reload` asks for a full reload. Whoever's reload finds a change, every client hears it through `config.changed`.

### 2. The panel mechanism

**Where:** a module `cortex_tui::panel` with the four panels as one enum, each with its state, its view and its keys; `App` holds the open panels in the order they opened. The views take the `Look`, the width (one less than the window's) and the rows they may use, draw from tokens (`Token::Accent` for the active plan item's glyph, `Token::Selected` for the focused row, as screen 4's colours show) and never name a colour. Step 22's view functions are used and not changed: a panel's rows are made with `layout::cut` and `spread`, and its text goes through `sanitize`.

**Where they sit,** as 09 and screen 4 have it: under the transcript's live rows (the streaming tail, the running round), above the input box, as one block followed by a blank row. **While a dialog is up, no panel is drawn** and the focus goes back to the input: a dialog takes the input box's place and fits the window whole (step 23), and keeps all the room it has today **(reversible)**. A menu replaces the hint row as before.

**The live region's limit with several panels.** The region stays at most half the window, and at least the bottom (the input box with its menu or hint row), as now. Panels may use what is left after the bottom and up to four of the transcript's live rows (the tail or the round's box, so what streams never vanishes under panels). When they need more:
1. the panels without the focus collapse, the oldest first, each to its collapsed form (one or two rows);
2. the focused panel scrolls its list to keep the focused row in view, with a dim `… N more` row at a cut end;
3. the oldest collapsed panels fold into one dim row, `+2 panels · ctrl+p`;
4. **when there is no room at all** (a window of eight rows or fewer, or a menu open), no panel is drawn.

**A panel that is not drawn cannot have the focus:** `Ring::update` is given only the panels drawn, so the focus goes back to the input when the focused panel no longer fits, and `Ctrl+P` passes over one that does not. The transcript's live rows take what is left, cut from their top with the mark, as today. Test: every window height from 4 to 60 rows, every set of the four panels, each focused or not, with and without a running round and a menu: the region is never taller than `max(rows / 2, bottom)`; a focused panel is drawn with its focused row, or the input has the focus; and no row is wider than the width.

**Focus with `Ctrl+P`,** shared with the live tool boxes: `Ring::update(boxes, panels)` each frame, the ring going input, each live box, each panel drawn, input. A focused panel is drawn expanded and collapses again when the focus leaves it. `Up` and `Down` move inside it (`Outcome::Move`), `Enter` acts on the focused row (`Outcome::Activate`), `Esc` returns to the input, and the hint row shows the panel's keys (screen 4: `space skips · e edits · a adds · esc back to the input`). Each panel's own keys are new actions of the `focus` context in `ACTIONS`, so `[keys]` can rebind them, reaching the application as a new `Outcome::Panel(Target, Action)`:

| Action | Default | Plan | Review | Usage | Project |
|---|---|---|---|---|---|
| `focus.skip` | `space` | skip or restore | | | |
| `focus.edit` | `e` | reword | | | edit |
| `focus.add` | `a` | add after | | | |
| `focus.delete` | `d` | delete | | | delete (asks) |
| `focus.move_up`, `focus.move_down` | `alt+up`, `alt+down` | reorder | | | |
| `focus.accept` | `y` | | | | accept; yes to a delete |
| `focus.disable` | `n` | | | | disable |
| `focus.promote` | `p` | | | | promote |
| `focus.refresh` | `r` | | | refresh | |
| `focus.range` | `tab` | | | next range | |
| `focus.group` | `g` | | | next grouping | |
| `focus.close` | `x` | | close | close | close |

**No keystroke is lost or taken by surprise.**
- **Every panel key that changes something is held to the dialog's rule** (step 23; 09-tui, "The approval dialog"), reused from `app::dialog` and not written again: it is taken only 300 ms after the panel's focused row was first drawn, and only after a second with no input the panel did not take; keys read together, a marked repeat and a paste never act; the grace starts again after the viewer, after `fg`, after the external editor, after a dialog hid the panel, when the focused row's content changes (a `project.updated` replacing the entry), and when a reload replaces the key map. Moving (`Up`, `Down`, `Ctrl+P`) and `Esc` are taken at once. `/project` and `/usage` open their panel focused, so typing that runs on past their `Enter` meets this rule and never acts.
- **A key that types and is not taken** (a panel or a live box has the focus but not the key, or the panel is not ready) **goes to the editor, and the focus with it.** Today a letter typed with a live box focused is dropped (`input.rs`, `Input::key`); it goes to the editor too, so the two kinds of target behave alike.
- A panel that opens by itself (a plan, a review, a limit near, entries waiting) never takes the focus.

**Rewording and adding** use the prompt's own editor, not a second text field: `e` (or `a`) sets aside what is being written, puts the item's text in the input box with the hint `enter saves the item · esc cancels`; `Enter` sends the edit (`plan/update` or `project/update`), never a prompt, and pushes nothing to the prompt history; `Esc` cancels; either way the text set aside comes back. Every rule of the editor holds there, and no new text input is laid out **(reversible)**.

**What a panel draws at three widths** (snapshots at 60, 80 and 120 columns, both glyph sets, with and without colour, light and dark):
- **Plan:** `◇ Plan  1 of 4 done`; collapsed, the active item after it; focused, one item a row, two columns in, cut with `…`.
- **Review:** collapsed, `◇ Changed 3 files  +12 −4 · checks passed`; focused, one file a row, the path shortened from its start with `…` so the counts stay, then the checks (point 4b).
- **Usage:** the session and history table, whose columns drop as the window narrows (cache write, then cache read, then input; output and cost stay; under 60 columns the name, output and cost alone). Limits lines wrap at the prose width.
- **Project:** one entry a row (its mark, kind, text cut with `…`, age); the focused entry's detail wraps at the width and is never cut (point 3).

**How a panel's data arrives without blocking a frame.** A panel is drawn from the application's state alone, which events and results change (`Link::next` never waits). `plan/get` and `usage/report` are already in `reads()` (`cortex-cli`'s `interactive.rs`); **`project/list` is added**. **`usage/refresh` always runs beside the lane**, as `workspace/files` does: it may wait 20 s on a provider and must never hold a prompt behind it, and it changes no session. `plan/update`, `project/update` and `config/reload` change sessions and run in the lane. Each result for a session carries its id in `Pending` and is dropped if the session was left (step 23's rule). A panel waiting for data says `reading…` in the key's own frame.

**Choices made for the run survive a reload:** a theme picked with `/theme` stays unless the file's `tui.theme` itself changed.

**Hostile text.** Every field from outside (a plan item; a path; an entry's text, command, purpose, folder, evidence and check; a provider's and a model's name; a project's label and key; a source; a balance's currency; a check's command and note; a notice's sentence) goes through `sanitize` before it is laid out, and each panel has a hostile-text test with `testing::problems`: controls, bidirectional overrides, zero-width characters, and an escape sequence in each field.

**Open and close, as state tests:** each of the four panels opens on its condition and closes on its own (09-tui's table), and the plan's "all done and the turn has ended" and the review's "the next prompt is sent" are tested at their edges.

### 3. The project panel

**Opens** when entries wait for this user's review (an observed entry that became active by itself, a proposed one, a shared one not yet accepted here), collapsed to one row (`◇ Project  2 entries wait for review`) without the focus; or with `/project`, focused, with every entry in use and waiting (`project/list`). **Closes** when nothing waits and it was not opened by `/project`, or with `x`. `project.updated` events change its rows.

**Accept, edit, disable, delete and promote** are each one `project/update` with one change, in the lane; the result's entries replace the rows, and a refusal is told with the core's sentence (14-adaptation's codes). **Every one of them waits for the panel to be ready** (point 2), and deleting, which the private store cannot undo, asks first in the panel's row (`delete this entry? y deletes · any other key keeps it`), its `y` held to the same rule.

**What the person sees before accepting a command:** the focused entry's detail, drawn whole: the whole command line, wrapped and never shortened, as an approval shows what will run; its purpose; its folder; whether it only reads; where it came from (`observed 3 times, learned in session s_… on 2026-11-14`, `proposed by the model in session … on …`, `the repository's file`, `you wrote it`); whether it **still asks** (an active entry not yet accepted) and whether it has **expired**, said as `cortex project` says them (14-adaptation: "it says no more than the engine will do"); its expiry; and its `check`, with "not run in this build". `y` is taken only when that whole detail was drawn in the frame on screen; an entry whose detail does not fit offers no `y` and says to make the window taller.

**Promote** asks the core to write the repository's file, which it already does safely (`knowledge::apply`: the store asked first, the file written once, never through a link, every byte a person wrote kept, never past 256 KiB); the panel writes nothing itself. **The core refuses a promote while any open session of that project has a turn running** (`turn_running`, added to 14's table), so neither this panel, another client of the core, nor a turn started between the panel's look and its key can write the file under a confined command; the panel says so beside `p` while its own session runs. A `cortex project promote` from another process during a command is the half left of [a-promote-while-a-confined-command-runs](../../outstanding/a-promote-while-a-confined-command-runs.md) (point 8).

### 4. The usage panel

**Opens** with `/usage`, focused, or by itself as one line in the warning colour when a provider's limit is near (less than a fifth left), as screen 4 draws it (`fast tier: 18% of the hourly output limit left, resets 15:00`). **Closes** with `x`, or by itself when the limit that opened it is no longer near.

**What it shows:** the session side from the session's live usage (`usage.updated`, which the application holds: tokens by class, cache hit rate, cost by model, speed, time to first token and tokens per second); the history side from `usage/report` for the range chosen (`tab`: today, week, month, all) and the grouping (`g`: provider then model, project, speed, and two at once, as 18-usage allows: provider within project), with each row's turns' time and tools where the report gives them and the zone its days are counted in. Each cost with its basis: metered plain, an estimate marked `est.` with the legend once, a total that mixes them shown apart (`$44.40 metered + $2.10 est.`), an unknown price as `—`. Limits: each provider's, and a tier's own line beside it, with its age (`as of 2 min ago`), `reset since; unknown until the next request` past its reset, a refreshed figure with its source, and **`not available`** for a provider that reports none. Balances in the provider's currency, exact.

**What `cortex stats` says, the panel says** ([statistics-a-build-cannot-use-are-said-only-in-the-log](../../outstanding/statistics-a-build-cannot-use-are-said-only-in-the-log.md)): `stats_newer`, statistics turned off, and a file set aside, which needs **`UsageReport.notes`** (a protocol addition: today only `cortex stats` reaches that, in process). No notice at the start (the item made it optional).

**The refresh key asks only the providers in `usage.refresh`.** `r` sends `usage/refresh` with no list; the core answers from `usage.refresh` as it is in force now (re-read on reload). With the list empty the panel says so and sends nothing. While one refresh is out, `r` does nothing more, so a held or doubled `r` makes one request. Tested against a local scripted server: a listed provider gets one request, one not listed none, a project's file cannot add one, and a provider added to the user's file is asked after a reload.

**A year of history within one frame (to confirm by measurement).** The panel opens in the key's own frame, its history rows `reading…`; `usage/report` runs beside the lane, through the link's own routing, and the next frame draws the rows. The budget ([performance-budgets.md](../../performance-budgets.md): ≤ 16 ms from the aggregates) is measured from the key to the frame with the rows, with a generated year of aggregates (365 days, five providers, twelve models, twenty projects, two speeds): a `perf_` test, release build, at the desk and on a hosted Windows runner, and a tier-1 test of the same path with a small year that fails on a change of shape (a raw file read, a second round trip). Over budget, I say so with a recommendation and do not narrow the year.

### 4b. Verification on screen, and the files named with `@`

**The checks** (step 19's bullet for step 24): the review panel shows the verification notices as they come: `started` (round 1 of 2, the checks by their command), `finished` (each check's line: its mark, command, folder, measure `exit 101 · 3.4s` and note, then the verdict), and `offered` (with `verify.mode = "ask"`: `checks not run: /verify runs them`). The turn's verdict from `turn.completed` is in the turn's summary line when `tui.turn_summary` is on, and in the review panel's collapsed row always. **A check's whole output a key away:** `Enter` on a check opens it in the viewer. Its output is a stored artifact (`CheckReport.artifact`), which no operation reads today: **`session/artifact`** (`session_id`, the artifact; the text, bounded at 1 MiB with what was cut said) is a new read, beside the lane, and the viewer draws it through `sanitize` (a protocol addition).

**`@` files warmed** (the features table's prefetch row; 15-efficiency, "Prefetch", step 3): when an `@` completion is accepted, the interface sends **`workspace/warm`** (`session_id`, the paths), a new read beside the lane: the core reads each named file's first 64 KiB on the blocking pool through the session's path resolver and gate (never a file the agent could not read unasked, never through a link the resolver refuses), at most 16 a prompt, and drops what it read, so that the model's first `read` finds it in the system's cache (a protocol addition) **(reversible: the core could instead warm what a prompt names when it arrives, with no new method, later than the doc places it)**.

### 5. Hot-reload of themes and key bindings

**One watcher, the core's.** On a focus-in and on a key press at most once a second, the interface sends `config/reload` with `changed_only`, after the key's own frame; `/reload` sends a full one. **When a `config.changed` event, or its own result, lists a `tui.*` or `keys.*` key as applied,** whichever client's reload or turn found it, the interface asks `config/get` for those two sections (a read) and, with the answer:
- builds the `Look` again (theme, glyphs, density and the other `tui` settings), in force from the next frame;
- builds the key map again with `KeyMap::with_table` and gives it to the input with a new setter, `Input::set_keymap`: a key already pressed was taken with the old map, the next uses the new one, and a dialog's or a panel's readiness starts again. An entry the map cannot use keeps its action's defaults and is told, as at the start.

**An invalid file leaves the old one in force and tells the person once:** the core keeps the configuration in force and reports the error once until the file changes again; the interface shows that one notice and keeps its `Look` and key map. A theme name not known is said once and `auto` used, as at the start.

**"A theme file"** is the configuration file that holds `tui.theme`: no user theme files exist. 09-tui's "Themes are TOML tables that assign a colour to each token" is built by no step; I add `user-theme-files` to the open items rather than make a new file format here **(reversible)**.

**Tested against the real core:** a `cortex-cli` test drives the application through the in-process link on a real core whose configuration is a test's files: the file's theme and a binding are changed, a focus event is given, the link drained, and the next frame's colours and the next key's action are the new ones; then the file is broken, and the old ones stay with one notice. A second test has the core's turn-start look find the change first, and the interface still takes it, from `config.changed`. The `cortex attach` client (step 27) gets the same through its own link: nothing here reaches the core but operations and events.

### 6. The cut: three pull requests of about 4,000 lines

| PR | Branch | What | Builds on |
|---|---|---|---|
| **1** | `worktree-step-24` | `config/reload` through the core (point 1): the mix in force, trust first, the lowered trust's reach, `session/set` recording its choices, the loop's `reconfigure` and `at_next_epoch`, the live launcher and key variables, the verifier's seam, the log level, `usage.*` and `stats.retention` at once, `next session`, `config.changed`; `project/list` in `reads()` and `usage/refresh` beside the lane; promote refused while a turn runs; `config/get`'s origins, `SessionUsage.context`, `UsageReport.notes`, `session/artifact`, `workspace/warm`; the protocol schema. | `main` |
| **2** | `worktree-step-24-panels` | The panel mechanism (point 2), the plan panel with editing, the review panel with the checks (point 4b, but the artifact's viewer), the focus actions in `ACTIONS`, the keys that type going to the editor, `/plan`, their snapshots at three widths, state and hostile-text tests. `cortex-tui` only, on the events `main` already has. | `main`, beside PR 1 |
| **3** | `worktree-step-24-views` | The usage and project panels, `/usage`, `/project`, `/profile`, `/verify`, `/reload`, `/context`, `/config`'s origins, a check's output in the viewer, `@` warming, theme and key reload, screen 4's rows, the measurements and the records. | PRs 1 and 2 |

PR 2 touches nothing PR 1 does, so the two are built and reviewed side by side; if the coordinator agrees, PR 2 goes to a sub-agent with its own branch and breaks file (`mutations/step-24-panels.breaks`, merged with `mutate --merge`).

### 7. The commands, `/context` and `/config`

| Command | Does |
|---|---|
| `/usage` | Opens the usage panel, focused. |
| `/plan` | Focuses the plan panel; with no plan, says the model makes one with its plan tool. |
| `/project` | Opens the project panel, focused. |
| `/profile [name]` | With a name, `config/reload` with that profile for the session (point 1). Without one, lists the profiles the session's layers define and the one in force. |
| `/verify` | Sends `verify/run` and shows the core's answer, whatever it is: today `not_available` with its sentence; after the owner's yes to the turn that only verifies, its events, which the review panel shows. |
| `/reload` | A full `config/reload`, and its notice. |
| `/context` | The last request's context by category, from **`SessionUsage.context`** (new, optional): system prompt, tool definitions, summary, your messages, answers, tool results and notices, each in tokens and as a share, against the request's limit and the model's window, marked as estimates. Printed into the transcript, as `/config` is; before any request it says there is none yet. |
| `/config` | As step 23's, each value followed by its layer and file, from **`ConfigGetResult.origins`** (new: the layer, the file as a person reads it, and the line). |

Each moves from `NOT_YET` into `COMMANDS` with its test.

### 8. The owed items

| Item | This step | How |
|---|---|---|
| [config-reload-through-the-core](../../outstanding/config-reload-through-the-core.md) | **Closes** (PR 1) | Point 1, with `next session` for the keys a session is made with once, and the `NextEpoch` keys at the next compaction. |
| [config-provenance-in-the-application](../../outstanding/config-provenance-in-the-application.md) | **Closes** (PRs 1, 3) | `config/get`'s origins; `/config` shows them. |
| [statistics-a-build-cannot-use-are-said-only-in-the-log](../../outstanding/statistics-a-build-cannot-use-are-said-only-in-the-log.md) | **Closes** (PRs 1, 3) | `UsageReport.notes`; the panel and `/usage` say what `cortex stats` says. No notice at the start. |
| [verify-run-a-turn-that-only-verifies](../../outstanding/verify-run-a-turn-that-only-verifies.md) | **Left** for the owner | `/verify` shows the core's answer, whatever it is, and needs no change when the branch merges. |
| [a-promote-while-a-confined-command-runs](../../outstanding/a-promote-while-a-confined-command-runs.md) | **Most of it** (PR 1) | The core refuses a promote while a session of the project runs a turn. Left: another process's promote during a command, with the item's recommendation. |
| [the-item-an-entry-was-learned-from](../../outstanding/the-item-an-entry-was-learned-from.md) | **Left** | The panel shows the session and the date, and offers no way to open the item. |
| [an-epoch-read-without-its-variants](../../outstanding/an-epoch-read-without-its-variants.md) | **Closes** (PR 1) | When a session is resumed, an epoch with no variants is fixed to the variants in force then and recorded in the session's state, so the next request and the next `epoch` entry carry them; a reload changes variants only at the compaction that takes them (point 1). |
| [the-workspace-root-is-the-working-directory](../../outstanding/the-workspace-root-is-the-working-directory.md) | **Left** for the owner | A reload uses the root the session was opened with. |
| [a-managed-ask-and-the-users-own-check](../../outstanding/a-managed-ask-and-the-users-own-check.md) | **Not built**, waiting for the owner | It was due before `/verify` because `/verify` runs the user's checks; `/verify` ships here only as the core's `not_available`, so nothing new runs a check. The checks at a turn's end (step 19) are as they were. |

**Left by this step, added to `docs/outstanding/`:** `user-theme-files` (point 5), `reload-now-begins-an-epoch` (08's `/reload --now`: the `NextEpoch` keys wait for the next compaction meanwhile), and `the-files-a-command-changed` (point 9, departure 4).

### 9. Departures from `docs/`, and questions for the coordinator

1. **`next session`,** a fourth effect, for `sandbox.enabled`, `sandbox.protect`, `shell.*`, `compat.*` and `index.map_wait` (point 1).
2. **`NextEpoch` keys at the next compaction,** not at a change of model, which keeps the prompt by design (point 1).
3. **`/reload --now` is not built** (point 8).
4. **The review panel lists the files the file tools changed, with their counts,** and says when a command ran that may have changed others (`commands ran too: /view shows them`): which files a command changed is in the checkpoints, not the protocol.
5. **No panel while a dialog is up** (point 2): 09 lists both in one region without saying which gives way.
6. **`/profile` drops the session's own choices** (point 1).
7. **Protocol additions,** asked before building, each with a default, no new protocol version, the schema and golden values regenerated:
   - `config/reload`'s params, `Empty` until now, become `ConfigReloadParams { session_id, profile, changed_only }` (`{}` still reads); `ConfigReloadResult.notes`; `ConfigReloadResult.pending`'s documentation says what it now holds (the next compaction, a new session, a restart);
   - a session event, `config.changed`;
   - `ConfigGetResult.origins`;
   - `SessionUsage.context`;
   - `UsageReport.notes`;
   - two read methods, `session/artifact` and `workspace/warm`;
   - the error `turn_running` for `project/update`'s promote.

### What changes outside `cortex-tui` and `cortex-cli`

| Crate | Change | Kind |
|---|---|---|
| `cortex-protocol` | Point 9's additions | Additions with defaults, schema diff |
| `cortex-core` | `config/reload`; a session's configuration in force and last loaded; trust judged first; `session/set` records its choices; the verifier's settings behind a lock; the session's launcher, key variables and repository trust live; `CoreOptions::log`; the core's own configuration a mix; `config/get`'s origins; `session/artifact`, `workspace/warm`; promote refused while a turn runs | Behaviour and API additions |
| `cortex-agent` | `SessionHandle::reconfigure` (taken as a request is prepared) and `at_next_epoch` (taken as a compaction begins); `Ledger` budgets changed in place; the context breakdown in the live usage; an epoch read without variants fixed at resume | API additions; the loop's other behaviour unchanged |
| `cortex-tools` | The shell's repository trust and key variables readable live | API addition |
| `cortex-config` | `Effect::NextSession`; `Loaded::taking`; the effects of `usage.*`, `stats.retention` and the `next session` keys | Schema and reference regenerated |
| `cortex-stats` | The writer reads the retention when it prunes | Behaviour |

### Tests for the "Done when"

| Done when | Test |
|---|---|
| Each panel opens and closes on its conditions | State tests for each panel's open and close (point 2), on the scripted link. |
| Plan edits from the panel reach the model | Through the in-process link on a real core with the scripted provider: `Space` on an item, then a prompt; the provider's next request holds the plan notice with the item skipped. |
| Editing a theme or key-binding file takes effect on the next focus event or key press; an invalid file leaves the old one | Point 5's two tests against the real core. |
| Several panels never exceed the live region's limit | Point 2's sweep of heights and panel sets. |
| Snapshots at three widths | Each panel at 60, 80 and 120, collapsed and focused, both glyph sets, colour and none, light and dark; screen 4's panel rows compared by `tui-screens`. |
| The usage panel within one frame with a year of history; "not available"; estimates marked | Point 4's measurement and tier-1 test; snapshot tests of a provider with no limit and of an estimate. |
| A key's frame with panels open | `perf` with the four panels open, against the key-to-frame budgets (≤ 2 ms idle, ≤ 4 ms while streaming). |

### The review of this note

An independent reviewer read the first version against the specification and the code. Its findings, and what each changed:

| Finding | Change |
|---|---|
| The trust store's file was not watched, so an untrust from outside waited for `/reload` | The trust file is stamped with the configuration's (point 1, step 1) |
| A reload with the old trust put a load with unapproved rules in force before trust was judged | Trust is judged first, from look-ahead loads never put in force; one load follows (steps 2, 3), with a hold-point test |
| Lowered trust left `repository_trusted` (the shell, the base capture, the verifier, the engine) and the sandbox's grant as they were | Lowered trust takes all of them (point 1), and `sandbox.write` and `network` apply at the next command |
| Rebuilding a model for a `Now` key applied `NextEpoch` keys (variants, effort, reasoning, a provider's switch) | Models are built from the configuration in force, a mix by key (`Loaded::taking`) |
| The slot for the next epoch was taken at a change of model, which keeps the prompt; it overrode a later `/model`; it departed from the variants item unsaid | Taken when a compaction begins, before what to keep is chosen; `session/set` drops the roles it sets; the variants item is answered |
| The core's own live configuration would move `Restart` keys (data directory, statistics) | The core's own configuration is a mix too |
| A theme or key change found by another reload (the turn's look, another client) was lost to this interface | `config.changed`, to every client of each session; the test runs against the real core |
| Only `y` was guarded in the project panel, and more weakly than a dialog | Every acting key of every panel is held to the dialog's whole rule, with its restarts |
| `usage/refresh` would wait in the lane behind a reload and hold prompts | Always beside the lane; one refresh at a time |
| Out of the note: `@` files warmed, and the verification notices with a check's output a key away | Point 4b, with two new read methods |
| A file set aside needed a protocol field | `UsageReport.notes` |
| `profile` could raise the level from a client that is not a `user` | `profile` only from a `user` client |
| The live-region test contradicted itself on small windows | What gives way, step 4, and the focus rule for a panel not drawn |
| `/reload` on a broken file said "nothing changed"; a reload could half-apply; the notice reached the model; `/profile`, `/theme` and the plan's editing against choices made in the session; tests missing for three criteria; promote racing a turn; `still asks` and `expired`; two groupings; hostile fields | Each answered in its point; promote is refused by the core while a turn runs |
