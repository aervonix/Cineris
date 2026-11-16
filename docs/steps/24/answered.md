# Answered in step 24, on 2026-11-16

Step 24 builds the panels and live views: the panel mechanism and the first-release panels (plan, with editing; review; usage; project), `/context`'s breakdown, hot-reload of themes, key bindings and settings wired end to end through `config/reload`, and the `/usage`, `/plan`, `/project`, `/profile`, `/verify` and `/reload` commands. The coordinator's answers, and the owner's, are added here as they come.

## The design, before building

Written against `main` at `7f82023`. The specification is the build plan's step 24 with its "From step 23" bullet, the "What later steps inherit" bullets that name step 24 (under steps 16, 17, 19, 21, 22 and 23), [09-tui.md](../../architecture/09-tui.md) and screen 4 of [tui-look/](../../architecture/tui-look/README.md), [08-configuration.md](../../architecture/08-configuration.md) ("Reloading without a restart", "Trust and what a project may set"), [14-adaptation.md](../../architecture/14-adaptation.md), [18-usage.md](../../architecture/18-usage.md) and [15-efficiency.md](../../architecture/15-efficiency.md). Every choice below is built as recommended while the coordinator and the owner answer, and each one marked **(reversible)** can be changed without undoing the rest. A choice that hangs on a number says **to confirm by measurement**.

### 1. `config/reload` through the core

**What a reload does,** for the core's own configuration (the user's, with no project: what `config/get` answers without a session and what `usage/*` reads) and for each open session, in this order:

1. **The files are checked again,** through `cortex-config`'s `LiveConfig` (one for the core, one for each session, made from the same `LoadOptions` the session was opened with). A file that does not load leaves the configuration in force as it was, and its error is reported once, not on every check (`LiveConfig`'s own rule).
2. **Trust is judged again,** as at a start, with the code a start uses (`assemble::trust`, `pinned_rules`, `held_back_settings`): the trust store is read again (on the blocking pool), a trusted project whose rules changed has its settings and allow rules set aside until the change is approved, a project trusted since (in another process, or by `cortex permissions trust`) has them applied, and a project's first limit still waits for trust. **Lowering trust takes effect even when a file is broken** (`LiveConfig::update`, `Reload::TrustLowered`). `--trust-project` holds for the life of the core, as it does today.
3. **What the person chose in the session outranks the files, as the command line does:** the level, the models, the reasoning levels and the speeds set with `session/set` (`/mode`, `/model`, `/effort`, fast mode) are kept in the session's `LoadOptions` as overrides from the moment they are set, so a reload never undoes them. Today `session/set` changes the running session only; it will also record what it set there.
4. **The changed keys are sorted by when they take effect** (`Loaded::changes_since`, each key's declared effect), and applied as the next table says.
5. **The person is told** (below), and the result lists every key: `applied`, `pending` and `errors`, with sentences in a new `notes`.

**Where each key lands, and when a running turn sees it.** A running turn sees only `Now` keys, and only at a boundary: a request in flight, a tool call already decided and a verification round under way keep what they started with.

| Keys (effect `Now`) | Applied by | A running turn sees it |
|---|---|---|
| `permissions.mode`, `permissions.rules` | A new engine from the session's template with the new rules and level, its session approvals and learned rules carried over, as `set_level` makes one, with the lease recorded again in the log. The sandbox's grant in the template stays as the session was opened (next row but one). | At the next permission decision. |
| `limits.*` | The ledger's budgets changed in place, what was spent kept: the session's, the turn's (the running turn's included) and the day's account (one made from the statistics where there was none; a day's limit with statistics off is refused as at open, and the old configuration stays). The standing lease is recorded again with the new budget. | At the next request: a limit already passed stops the turn there, as a limit reached does. |
| `retry.*`, `providers.*.key_env`, `headers`, `idle_timeout`, `first_event_timeout`, `model.speed`, `speed_fallback`, `speed_wait`, `speed_cooldown`, `speed_cooldown_min`, `model.roles.*.speed` | Each role's model built again with the same catalog entry, reasoning level and output limit and the new settings (`Providers::role_model`, as `session/set` builds one), then `set_role_model`, `set_speed`, `set_role_speed`. Same provider, protocol, model and effort: no epoch begins and the cache is kept. | At the role's next request. |
| `context.auto_compact`, `background`, `compact_at`, `prepare_at`; `verify.rounds` | A new `SessionHandle::reconfigure`, which the loop takes from its inbox when it prepares a request, as it takes a change of speed today. | At the next request; `verify.rounds` at the next round. |
| `verify.mode`, `commands`, `round_time`, `turn_time` | The core's verifier, whose settings (and whether a project's file lowered the mode) move behind a lock it reads when a round begins: the seam step 19 left. | At the next verification round. |
| `log.level` | The process's log filter (`LogHandle::set_filter`, given to the core in `CoreOptions`), unless `--log-level` or `CORTEX_LOG` set it. | At once. |
| `usage.week_start`, `usage.ranges`, `usage.refresh`, `stats.retention` | The core's own configuration, which `usage/report`, `usage/refresh` and the statistics' writer read when they run. **Their effect changes from `restart` to `at once`** in the schema and the reference, as steps 16 and 23 asked. | Not a turn's: the next report, refresh or pruning. |
| `tui.*`, `keys.*` | The interface (point 5). The core only reports them changed. | — |

**`NextEpoch` keys** are listed as `pending` and given to the session's loop in a slot it takes when it next begins an epoch (`SessionHandle::at_next_epoch`): the role models built with the new `model.*`, `model.roles.*`, `providers.*.enabled`, `protocol`, `base_url` and `adapt.variants.*`; and the context's `keep_recent_tokens`, `instruction_names`, `instructions_cap`, `project.profile_tokens`, `index.map` and `index.map_tokens`. A compaction, a resume and a change of model begin an epoch, and nothing else does: a reload does not begin one to apply them (08's `/reload --now` is left, below). A second reload before that epoch replaces what waits.

**Keys a session is built with once** are departures from the reference, which calls them `next epoch` or `at once`: the shell (`shell.*`: the tool, its process runner and its line in the system prompt), the compatibility readers (`compat.*`), the map's wait (`index.map_wait`) and **the sandbox (`sandbox.*`)**: its launcher, its write roots and the continuity of its protected paths are made when the session opens, and making them again between two commands of a running session would let a change of protected paths read as a change from outside. I recommend a fourth effect, **`next session`** ("in sessions opened from then on"), declared for those keys in `sections.rs`, with the reference regenerated, so that what the reference says is what a reload does **(reversible; the coordinator's call, since it changes what 08 says of the sandbox's keys)**.

**What a reload can never do,** and the tests that hold it:
- **Apply a project's setting before trust.** Every load goes through `cortex-config` with the trust the store gives now; a session's `LoadOptions` never carry `Trusted` for a project the store does not trust. Test: a project file gains an `allow` rule and a higher `model.effort` while the session runs; after `config/reload` neither is in force and both are listed as held back; trusted afterwards, a second reload applies them.
- **Loosen a user-only or tighten-only key from a project's file.** The same load rules as a start: a project's `usage.refresh`, `sandbox.enabled` or a higher `limits.turn.cost` is refused with its warning. Tests for each class (user-only, tighten-only, first limit, a rule whose project rules changed since trust).
- **Leave the session without a configuration.** A broken file leaves the old one, except a lowering of trust.
- **Undo a choice made in the session** (point 3 above).

**What the person is told.** One notice in each open session, after a reload that changed something there, of the source `harness`, written once a reload: what is in force now (the keys, by section when there are many), what waits for the next epoch, what waits for a new session or a restart, and the trust sentence when trust was judged differently. A reload that changed nothing says nothing, unless it was `/reload`, which says "Nothing changed." A file with an error: its error, once, and that the old configuration stays. The interface draws the notice through `sanitize`, since it holds values from files.

**Who asks, and when** (08's four moments): the core checks its own and each session's files at the start of every turn (stat calls, on the blocking pool, before the prompt is taken); the interface asks on a focus-in and on a key press at most once a second (`config/reload` with `changed_only`, point 5), and on `/reload` (a full reload). A check that finds nothing changed costs a few `stat` calls (22 µs measured in step 9) and no notice.

**Protocol changes** (each an addition with a default; no new protocol version; the schema and golden values regenerated):
- `config/reload`'s params, `Empty` until now, become `ConfigReloadParams { session_id, profile, changed_only }`: `{}` still reads. `profile` (with `session_id`) loads that session with another profile, for `/profile`; a profile no layer defines is refused with the ones that exist (`unknown_profile`), and a project's profile is held to trust as at a start. `changed_only` reloads only when a file changed.
- `ConfigReloadResult.notes`: the sentences above.
- `ConfigGetResult.origins` (point 7, provenance).
- `SessionUsage.context` (point 7, `/context`).

### 2. The panel mechanism

**Where:** a module `cortex_tui::panel` with the four panels as one enum, each with its state, its view and its keys; `App` holds the open panels in the order they opened. The views take the `Look`, the width (one less than the window's) and the rows they may use, draw from tokens (`Token::Accent` for the active plan item's glyph, `Token::Selected` for the focused row) and never name a colour. Step 22's view functions are used and not changed: a panel's rows are made with `layout::cut` and `spread`, its text through `sanitize`.

**Where they sit,** as 09 and screen 4 have it: under the transcript's live rows (the streaming tail, the running round), above the input box, as one block followed by a blank row. **While a dialog is up, no panel is drawn** and the focus goes back to the input: a dialog takes the input box's place and fits the window whole (step 23's rule), and it keeps all the room it has today **(reversible)**. A menu replaces the hint row as before, and the panels stay.

**The live region's limit with several panels.** The region stays at most half the window and at least the bottom (the input box with its menu or hint row), as now. Panels may take the rows left after the bottom and up to four rows of the transcript's live rows (the tail or the round's box, so that what streams never vanishes under panels). When the panels need more:
1. the panels without the focus collapse, the oldest first, each to its collapsed form (1 or 2 rows);
2. the focused panel scrolls its list to keep the focused row in view, with a dim `… N more` row at a cut end;
3. if the collapsed panels alone still do not fit, the oldest of them fold into one dim row, `+2 panels · ctrl+p`.

The transcript's live rows then take what is left, cut from their top with the mark, as today. Test: every window height from 4 to 60 rows, every set of the four panels, each focused or not, with and without a running round: the region is never taller than `max(rows / 2, bottom)`, the focused row is always drawn, and no row is wider than the width.

**Focus with `Ctrl+P`,** shared with the live tool boxes: `Ring::update(boxes, panels)` each frame with the panels' ids in the order drawn, so the ring goes input, each live box, each panel, input. A panel that closes gives the focus back to the input (the ring's own rule). A focused panel is drawn expanded; it collapses again when the focus leaves it. `Up` and `Down` move inside it (`Outcome::Move`), `Enter` acts on the focused row (`Outcome::Activate`), `Esc` returns to the input, and the hint row shows the panel's keys (screen 4: `space skips · e edits · a adds · esc back to the input`). Each panel's own keys are new actions of the `focus` context in `ACTIONS`, so `[keys]` can rebind them, and a new outcome, `Outcome::Panel(Target, Action)`, carries them to the application. The names and keys:

| Action | Default | Plan | Review | Usage | Project |
|---|---|---|---|---|---|
| `focus.skip` | `space` | skip or restore | | | |
| `focus.edit` | `e` | reword | | | edit |
| `focus.add` | `a` | add after | | | |
| `focus.delete` | `d` | delete | | | delete |
| `focus.move_up`, `focus.move_down` | `alt+up`, `alt+down` | reorder | | | |
| `focus.accept` | `y` | | | | accept |
| `focus.disable` | `n` | | | | disable |
| `focus.promote` | `p` | | | | promote |
| `focus.refresh` | `r` | | | refresh | |
| `focus.range` | `tab` | | | next range | |
| `focus.group` | `g` | | | next grouping | |
| `focus.close` | `x` | | close | close | close |

A key a focused panel does not use, that types a character, goes to the editor as step 21 types it, and the focus goes back to the input with it, so no keystroke is lost or taken by a panel the person was not looking at. A panel that opens by itself (a plan, a review, a limit near, entries waiting) never takes the focus.

**Rewording, adding and editing** use the prompt's own editor, not a second text field: `e` puts the item's text in the input box with the hint `enter saves the item · esc cancels`, `Enter` sends the edit (`plan/update` or `project/update`) and not a prompt, and `Esc` gives the editor back as it was. Every rule of the editor (grapheme layout, paste, undo, the external editor) holds there, and no new text input is laid out **(reversible)**.

**What a panel draws at three widths** (snapshots at 60, 80 and 120 columns, both glyph sets, with and without colour, light and dark):
- **Plan:** `◇ Plan  1 of 4 done`; collapsed, the active item after it; focused, one item a row, two columns in, its text cut with `…`.
- **Review:** collapsed, `◇ Changed 3 files  +12 −4 · checks passed`; focused, one file a row, the path shortened from its start with `…` so the counts stay, then the verification's line.
- **Usage:** the session and history table, whose columns drop as the window narrows, in this order: cache write, cache read, input (output and cost stay); under 60 columns, the model, output and cost alone. Limits lines wrap at the prose width.
- **Project:** one entry a row (its mark, kind, text cut with `…`, age); the focused entry's detail wraps at the width and is never cut (point 3).

**How a panel's data arrives without blocking a frame.** A panel is drawn from the application's state and nothing else; the state is changed by events and results the link gives (`Link::next` never waits). The reads the panels make go beside the lane: `plan/get` and `usage/report` are already in `reads()` (`cortex-cli`'s `interactive.rs`), and **`project/list` is added there**. **`usage/refresh` runs beside the lane too**, since it may wait up to 20 s on a provider and must not hold a prompt behind it; it changes no session. `plan/update`, `project/update` and `config/reload` change sessions and run in the lane. Each result for a session carries that session's id in `Pending` and is dropped if the session was left meanwhile (step 23's rule). A panel waiting for its data says so (`reading…`) in the key's own frame.

**Hostile text.** Every field from outside (a plan item, a path, an entry's text, command, purpose, evidence and check, a provider's and a model's name, a project key, a balance's currency, a notice's sentence) goes through `sanitize` before it is laid out, and each panel has a hostile-text test with `testing::problems`: controls, bidirectional overrides, zero-width characters, and an escape sequence in each field.

### 3. The project panel

**Opens** when entries wait for this user's review (an observed entry that became active by itself, a proposed one, a shared one not yet accepted here), collapsed to one row (`◇ Project  2 entries wait for review`) and without the focus; or with `/project`, focused, with every entry in use and waiting (`project/list`). **Closes** when nothing waits and it was not opened by `/project`, or with `x`. `project.updated` events change its rows as they come.

**Accept, edit, disable, delete and promote** are each one `project/update` with one change, in the lane; the result's entries replace the rows, and a refusal is told with the core's sentence (the codes of 14-adaptation's table: `shared_entry_read_only`, `not_promotable`, `project_store_full`, `shared_profile_full` and the rest). Editing puts the entry's text in the input box (point 2). Deleting asks nothing more: the private store keeps no deleted entry, so `d` asks `delete this entry? y deletes · any other key keeps it` in the panel's own row first **(reversible)**.

**What the person sees before accepting a command:** the focused entry's detail, drawn whole: the whole command line, wrapped and never shortened, as an approval shows what will run; its purpose; its folder; whether it only reads; where it came from (`observed 3 times, learned in session s_… on 2026-11-14`, `proposed by the model in session … on …`, `the repository's file`, `you wrote it`); its expiry; and its `check`, with "not run in this build" (step 43 runs them).

**Accepting is held to the dialog's rule** (step 23, 09-tui's "The approval dialog"), since accepting a command makes it run unasked (`runs_unasked`, step 19): `y` is taken only when the focused entry's whole detail was drawn in the frame on screen, and only once that entry has been on screen 300 ms with no input the panel did not take; until then `y` types, as a key a dialog does not take yet types. An entry whose detail does not fit the room offers no `y`, and says `the whole entry does not fit: make the window taller`. So a key typed for the editor, a key held down, or a panel that opened under the typing never accepts an entry the person did not choose.

**Promote** asks the core to write the repository's file, which it already does safely (`knowledge::apply`: the store asked first, the file written once, never through a link at either name, every byte a person wrote kept, never past 256 KiB). The panel adds no write of its own. **It does not offer promote while its session's turn runs** (the hint row says why): the file is a protected path, and a confined command's looks around it would read the write as a change from outside ([a-promote-while-a-confined-command-runs](../../outstanding/a-promote-while-a-confined-command-runs.md)). That closes the panel's half of the item; a promote from another client or `cortex project promote` while a command runs is the half left (point 8).

### 4. The usage panel

**Opens** with `/usage`, focused, or by itself as one line in the warning colour when a provider's limit is near (less than a fifth left), as screen 4 draws it (`fast tier: 18% of the hourly output limit left, resets 15:00`). **Closes** with `x`, or by itself when the limit that opened it is no longer near.

**What it shows:** the session side from the session's live usage (`usage.updated`, which the application already holds: tokens by class, cache hit rate, cost by model, speed, time to first token and tokens per second), and the history side from `usage/report` for the range chosen (`tab`: today, week, month, all) and the grouping (`g`: provider and model, project, speed). Each cost with its basis: a metered amount plain, an estimate marked `est.` with the legend once, a total that mixes them apart (`$44.40 metered + $2.10 est.`), an unknown price as `—`. Limits: each provider's, and a tier's own line beside it, with its age (`as of 2 min ago`), `reset since; unknown until the next request` past its reset, a refreshed figure with its source, and **`not available`** for a provider that reports none. Balances as the provider's currency, exact.

**What `cortex stats` says, the panel says** ([statistics-a-build-cannot-use-are-said-only-in-the-log](../../outstanding/statistics-a-build-cannot-use-are-said-only-in-the-log.md)): `stats_newer`, statistics turned off, a file set aside. No notice at the start (the item's recommendation made it optional).

**The refresh key asks only the providers in `usage.refresh`.** `r` sends `usage/refresh` with no list, which the core answers from `usage.refresh` as it reads it now (re-read on reload, point 1). With the list empty, the panel says so and sends nothing (`usage.refresh is empty: no provider is asked`). Tested against a local scripted server: a provider listed gets one request, one not listed gets none, and a project's file cannot add one.

**A year of history within one frame (to confirm by measurement).** The panel opens in the key's own frame, its history rows `reading…`; `usage/report` reads the aggregates beside the lane and the next frame draws them. The budget ([performance-budgets.md](../../performance-budgets.md), "Open the usage view, any range, with a year of history: ≤ 16 ms from the aggregates") is measured from the key to the frame with the rows, with a generated year of aggregates (365 days, five providers, twelve models, twenty projects, both speeds): a `perf_` test, release build, at the desk and on a hosted Windows runner, and a tier-1 test of the same path with a small year that fails on a regression of shape (a raw file read, a second round trip). If it is over, I say so with a recommendation; I do not narrow the year.

### 5. Hot-reload of themes and key bindings

**One watcher, the core's.** The interface keeps no copy of the files: on a focus-in, and on a key press at most once a second, it sends `config/reload` with `changed_only`, after the key's own frame is drawn; on `/reload`, a full one. When the result lists any `tui.*` or `keys.*` key as applied, it asks `config/get` for those two sections (a read) and, with its answer:
- builds the `Look` again (theme, glyphs, density and the other `tui` settings), in force from the next frame;
- builds the key map again with `KeyMap::with_table` and gives it to the input with a new setter, `Input::set_keymap`: a key already pressed was taken with the old map, and the next key uses the new one. An entry the map cannot use keeps its action's defaults and is told, as at the start.

**An invalid file leaves the old one in force and tells the person once:** the core keeps the old configuration and reports the error once (`LiveConfig` remembers the failure until the file changes again), so the interface shows one notice and keeps its `Look` and key map. A theme name not known is said once and `auto` used, as at the start.

**"A theme file"** is the configuration file that holds `tui.theme`: there are no user theme files yet. 09-tui's "Themes are TOML tables that assign a colour to each token" is built by no step; I add `user-theme-files` to the open items rather than build a new file format here **(reversible)**.

So the theme or a binding changed in the file takes effect on the next focus event or key press, without a restart: a test edits the file under a scripted core, sends a focus event, drains the link, and checks the next frame's colours, then a key's action. The `cortex attach` client (step 27) gets the same through its own link: nothing here reaches the core but operations.

### 6. The cut: three pull requests of about 4,000 lines

| PR | Branch | What | Builds on |
|---|---|---|---|
| **1** | `worktree-step-24` | `config/reload` through the core (point 1), with `session/set` recording its choices, the loop's `reconfigure` and `at_next_epoch`, the verifier's seam, the log level, `usage.*` and `stats.retention` at once; `project/list` in `reads()` and `usage/refresh` beside the lane; `config/get`'s origins; `SessionUsage.context`; the `next session` effect; the protocol schema; the epoch read without its variants (point 8). | `main` |
| **2** | `worktree-step-24-panels` | The panel mechanism (point 2), the plan panel with editing, the review panel, the focus actions in `ACTIONS`, `/plan`, their snapshots at three widths and hostile-text tests. `cortex-tui` only. | `main`, in parallel with PR 1 |
| **3** | `worktree-step-24-views` | The usage and project panels, `/usage`, `/project`, `/profile`, `/verify`, `/reload`, `/context`, `/config`'s origins, theme and key reload, screen 4's rows, the measurements and the records. | PRs 1 and 2 |

PR 2 touches nothing PR 1 does, so the two are built and reviewed side by side. If the coordinator prefers it, PR 2 is given to a sub-agent with its own breaks file (`mutations/step-24-panels.breaks`, merged with `mutate --merge`).

### 7. The commands, `/context` and `/config`

| Command | Does |
|---|---|
| `/usage` | Opens the usage panel, focused. |
| `/plan` | Focuses the plan panel; with no plan, says that the model makes one with its plan tool. |
| `/project` | Opens the project panel, focused. |
| `/profile [name]` | With a name, `config/reload` with that profile for the session; the notice says what is in force and what waits. Without one, lists the profiles the session's layers define and the one in force. |
| `/verify` | Sends `verify/run` and shows the core's answer, whatever it is: today the core's `not_available` and its sentence; with the owner's yes to the turn that only verifies, its events, which the review panel shows. |
| `/reload` | A full `config/reload`, and its notice. |
| `/context` | The last request's context by category, from `SessionUsage.context` (new, optional): system prompt, tool definitions, summary, your messages, answers, tool results, notices, each in tokens and as a share, against the request's limit and the model's window, with the estimate marked as one. Printed into the transcript, as `/config` is. Before any request: says there is none yet. |
| `/config` | As step 23's, each value followed by its layer and file (`ConfigGetResult.origins`, new: `layer`, the file as a person reads it, and its line), which closes [config-provenance-in-the-application](../../outstanding/config-provenance-in-the-application.md). |

Each moves from `NOT_YET` into `COMMANDS` with its test.

### 8. The owed items

| Item | This step | How |
|---|---|---|
| [config-reload-through-the-core](../../outstanding/config-reload-through-the-core.md) | **Closes** (PR 1) | Point 1, with the `next session` departure for the keys a session is built with once. |
| [config-provenance-in-the-application](../../outstanding/config-provenance-in-the-application.md) | **Closes** (PRs 1 and 3) | `config/get`'s origins; `/config` shows them. |
| [statistics-a-build-cannot-use-are-said-only-in-the-log](../../outstanding/statistics-a-build-cannot-use-are-said-only-in-the-log.md) | **Closes** (PR 3) | The usage panel and `/usage` say what `cortex stats` says. No notice at the start. |
| [verify-run-a-turn-that-only-verifies](../../outstanding/verify-run-a-turn-that-only-verifies.md) | **Left** for the owner | `/verify` shows the core's answer, whatever it is, and needs no change when the branch merges. |
| [a-promote-while-a-confined-command-runs](../../outstanding/a-promote-while-a-confined-command-runs.md) | **Its half** (PR 3) | The panel offers no promote while its session's turn runs. Another client's or the command line's promote during a command stays open, with its recommendation. |
| [the-item-an-entry-was-learned-from](../../outstanding/the-item-an-entry-was-learned-from.md) | **Left** | The panel shows the session and the date and offers no way to open the item, so it needs no change to the tools' contract. |
| [an-epoch-read-without-its-variants](../../outstanding/an-epoch-read-without-its-variants.md) | **Closes** (PR 1) | When a session is resumed, an epoch with no variants is fixed to the variants in force then, recorded in the session's state, so the next request and the next `epoch` entry carry them; a reload's rebuilt model whose variants differ waits for the next epoch, as a variant's change does. |
| [the-workspace-root-is-the-working-directory](../../outstanding/the-workspace-root-is-the-working-directory.md) | **Left** for the owner | A reload uses the root the session was opened with. |
| [a-managed-ask-and-the-users-own-check](../../outstanding/a-managed-ask-and-the-users-own-check.md) | **Not built**, waiting for the owner | |

**Left by this step, added to `docs/outstanding/`:** `user-theme-files` (point 5) and `reload-now-begins-an-epoch` (08's `/reload --now`, which needs the loop to begin an epoch when asked; the `NextEpoch` keys wait for the next natural epoch meanwhile).

### What changes outside `cortex-tui` and `cortex-cli`

| Crate | Change | Kind |
|---|---|---|
| `cortex-protocol` | `ConfigReloadParams`, `ConfigReloadResult.notes`, `ConfigGetResult.origins`, `SessionUsage.context` | Additions with defaults, schema diff |
| `cortex-core` | `config/reload`; `session/set` records its choices as overrides; the verifier's settings behind a lock; `CoreOptions::log`; the core's own configuration live; `config/get`'s origins | Behaviour and API additions |
| `cortex-agent` | `SessionHandle::reconfigure` and `at_next_epoch`, taken at a request's preparation and at an epoch's beginning; `Ledger` budgets changed in place; the context breakdown in the live usage; an epoch read without variants fixed at resume | API additions; the loop's other behaviour unchanged |
| `cortex-config` | `Effect::NextSession`; the effect of `usage.*`, `stats.retention`, and the keys a session is built with once | Schema and reference regenerated |
| `cortex-stats` | The writer's retention read when it prunes, not only at start | Behaviour |

### Departures from `docs/`, for the coordinator

1. **The `next session` effect** for `sandbox.*`, `shell.*`, `compat.*` and `index.map_wait` (point 1). 08 calls the sandbox's keys "at once".
2. **No panel while a dialog is up** (point 2): 09 lists panels and the dialog in one region without saying which gives way.
3. **`/reload --now` is not built** (point 8): the `NextEpoch` keys wait for the next natural epoch.
4. **The review panel lists the files the file tools changed, with their counts,** and says when a command ran that may have changed others (`commands ran too: /view shows them`): which files a command changed is not in the protocol, only in the checkpoints. Adding it would be a protocol addition I do not make here.
5. **Four protocol additions** (point 1), which I ask before building, as every addition to the protocol is asked.
