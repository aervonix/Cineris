# Answered in step 9, on 2026-11-12

Step 9 built `cortex-config`, and owner-only files on Windows in `cortex-base`. These are the choices it put to the owner, each accepted as recommended.

| Question | The owner's answer, 2026-11-12 | Recorded in |
|---|---|---|
| **Owner-only files on Windows.** Three options: rely on the per-user profile folder's rules, a helper in `cortex-sandbox`, or `cortex-base` itself with one module allowed `unsafe`. | **`cortex-base`,** in one Windows-only module, with `windows-sys`. The list is the current user only, like `0600`. | [Decision 0029](../../decisions/0029-owner-only-files-on-windows.md) |
| **`owner_only` over an existing file.** Step 4 kept an existing file's mode even with the option set. | **One meaning on both OSes:** with the option, the file the write leaves is owner-only, existing or not. It only ever tightens. | [Decision 0029](../../decisions/0029-owner-only-files-on-windows.md), [architecture/05-tools.md](../../architecture/05-tools.md), "Atomic writes" |
| **Where the credential layer ends.** `AuthMethod` is in `cortex-llm`, which `cortex-config` may not depend on. | **`cortex-config` owns the stores and the lookup order** and returns a `Secret` with its source. `cortex-core` hands it to the API-key `AuthMethod`. No new crate edge. | [Decision 0018](../../decisions/0018-credential-storage.md), [architecture/08-configuration.md](../../architecture/08-configuration.md) |
| **The Linux keychain.** The Secret Service backend cannot be run in the blocking CI. | **Not shipped untested.** The store selection, the feature name and a clear error are built. The backend is [outstanding](../../outstanding/secret-service-backend.md). | [Decision 0018](../../decisions/0018-credential-storage.md) |
| **The Credential Manager wrapper.** | **`windows-native-keyring-store` with `keyring-core`,** and local-machine persistence for stored keys. | [dependencies.md](../../dependencies.md) |

Added on 2026-11-13, with the fixes that followed the review of step 9:

| Question | The owner's answer, 2026-11-13 | Recorded in |
|---|---|---|
| **Which keys raise cost.** Only a speed tier other than `standard` waited for trust; a project could still choose a dearer model, more reasoning, more output or more attempts. | **An untrusted repository must not raise what its user spends.** The model, for the main role and each other role, with a role's provider; the reasoning level, for the main role and each other role; `model.max_output`; and `retry.attempts` are "raises cost", whatever their value. They apply only once the project is trusted, and until then the user's own values apply. | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Trust and what a project may set" |
| **Whose `.git` or `.cortex` counts for the workspace root on Windows.** An elevated process creates objects owned by the Administrators group, not the user. | **The current user's, or the Administrators group's** (`S-1-5-32-544`), read with the owner check `cortex-base` already has, so no new `unsafe`. Only an administrator can create an object the group owns. A link at `.git` is passed over on Windows and counts by its own owner on Linux. | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Layers" |
| **The check for changed files and a link that appears after the load.** The check asks the file system, which follows links. | **The project's files are found again through the path resolver before each check**, and on any difference the configuration is reloaded through the resolver. The moment between that look and the check's own is a residual, listed for the whole-repo review. | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Reloading without a restart" |
| **What is in force when trust is lowered while a file is broken.** Dropping the project's files would drop its `deny` rules and protected paths too. | **What an untrusted load would give:** the files in force are merged again with the project untrusted, so its restrictions and preferences stay and what needs trust waits. Losing a restriction would loosen what is in force, the opposite of what taking trust away is for. | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Reloading without a restart" |
| **Secrets in a reserved section.** A section a later step declares holds anything until then, an MCP server's token included. | **Redacted by name until it is declared** (`env`, `headers`, and names with `token`, `secret`, `password`, `auth`, `credential` or the word `key`), **and the user part of any web address everywhere.** The step that declares a section gives its secret keys the redacted kind. | [architecture/08-configuration.md](../../architecture/08-configuration.md), "Credentials" |

## Settings a repository could slow a session with (2026-11-15)

A fix to this step's crate. It settles the item a search of the schema left open after the review of step 18: preference keys with no range, which a cloned repository's own configuration file could set to slow or stall the session of whoever opened it, whether or not the user trusted it ([08-configuration.md](../../architecture/08-configuration.md), "Trust and what a project may set"). The review of step 18 had found the same mechanism in `index.map_wait`.

**The owner's decision:** each such key gets a range of what a person would set. The retry and timeout keys become tighten-only, so a project's file may shorten them and not lengthen them, and `context.background` becomes the user's alone. The two cooldowns stay preferences with a range: they slow a session without stalling it.

**The ranges.** A range binds every file, the user's own included: a value outside it is an error at load, and the file is refused. Since a project's file may lower a tighten-only key, the floor matters as much as the ceiling.

| Key | Default | Range | Class | Why the floor, why the ceiling | From |
|---|---|---|---|---|---|
| `retry.backoff_base` | 1 s | 100 ms to 1 min | tighten-only | Below 100 ms the attempts are spent before a brief outage can pass; `0s` retried at once. The first retry should come soon, and `backoff_cap` holds the later ones. | The owner's decision |
| `retry.backoff_cap` | 30 s | 1 s to 5 min | tighten-only | From a second, so that the attempts span more than a moment. Past five minutes a provider is down rather than busy. | The owner's decision |
| `retry.max_wait` | 2 min | 1 min to 1 h | tighten-only | From a minute, the window a provider counts its limits in, so that no file can make an ordinary rate limit end a turn. Past an hour the wait is a quota, not a rate limit. | The owner's decision |
| `retry.idle_timeout` | 90 s | 30 s to 1 h | tighten-only | A hosted provider that is still working sends a token or a ping within 30 s; at a millisecond every request failed. The ceiling is well past the five minutes the built-in entries give a local server. | The owner's decision |
| `retry.first_event_timeout` | 60 s | 30 s to 1 h | tighten-only | Below 30 s a long prompt would fail every time. The ceiling is well past the ten minutes the built-in entries give a local server. | The owner's decision |
| `providers.<name>.idle_timeout`, `first_event_timeout` | not set | 30 s to 1 h | user only, as before | The same ranges as the keys they override. The built-in entries' own five and ten minutes are inside them. | The owner's decision |
| `model.speed_wait` | 20 s | 0 to 2 min | tighten-only | `"0s"` is documented ("never waits"), and a lower value only falls back to `standard` sooner. Past two minutes `standard` answers sooner than the tier would. | The owner's decision |
| `model.speed_cooldown` | 10 min | 1 min to 1 h | preference | From a minute, so that a session does not change speed, and perhaps write its cache again, every few requests. Past an hour a tier that keeps refusing is better turned off with `model.speed`. | The owner's decision |
| `model.speed_cooldown_min` | 1 min | 0 to 1 h | preference | `"0s"` is documented ("takes the provider's time as given"). The cost that leaves open is an outstanding item: [a-projects-shortest-cooldown](../../outstanding/a-projects-shortest-cooldown.md). | The owner's decision; the coordinator chose to keep the floor |
| `context.background` | on | | user only | Off, every compaction waits at the trigger for its summary. | The owner's decision |
| `shell.timeout` | 2 min | 1 s to 30 min | tighten-only (was a preference) | Below a second every command that asks for no limit would fail. Past half an hour a command that hangs holds up its turn too long. It is held to `shell.max_timeout` where it is read (`cortex-tools`' shell: the default and a command's own limit alike, `.min(max_timeout)`). | The twin search; the coordinator approved it |
| `shell.max_timeout` | 30 min | 10 s to 1 day | tighten-only, as before | It had no floor: a project's `"1ms"` capped every command at a millisecond. Below ten seconds no build or test run could finish. | The twin search; the coordinator approved it |

**The check for the mechanism.** `waits::every_number_a_project_can_set_has_a_range` fails for any key that a project's file can set (any class but user-only, so tighten-only keys too) and that holds a length of time or a number with no range, unless a short list gives the reason. The list also fails when an entry gets a range or is removed, so it cannot go stale. The brief asked for preferences only. The check covers every class a project can apply, because a tighten-only key with no floor was the way `shell.max_timeout` broke every command. Run over the schema before this fix, it flagged these:

| Key | What was done | Why |
|---|---|---|
| The nine keys above, and `shell.timeout` | A range | Above. |
| `shell.max_timeout` | A range | Above. |
| `limits.turn.time`, `limits.session.time`, `limits.day.time` | Listed | A limit stops a turn or a session on purpose and says why, as `requests = 1` does. A project's lower one is a restriction the design lets it add, and a higher one is the user's alone. The coordinator approved the list. |
| `index.pass_time` | Listed | The index holds it to at least 250 ms where it reads it, and a pass that runs out carries on when the session is idle, so no request waits for it. The coordinator approved the list. |

**Tighten-only on a length of time.** The rule was already used by three duration keys (`limits.*.time`, `index.pass_time`, `shell.max_timeout`), so the brief's "no duration key uses it today" no longer held. It compares through `magnitude`, in milliseconds, so `"1m30s"` and `"90s"` are one value. It compares with the value in effect from the layers below, the user's included, not with the default, and the first limit where there was none still applies. Each of these is tested (`waits::a_length_of_time_is_tightened_against_the_one_in_effect_in_any_units`).

**Two changes of wording that go with it.**
- The warning for a project's longer value said "a project's configuration may not set this", which was wrong for a key it may lower. It now says so, and names the value in effect: `a project's configuration may lower this and may not raise it: "10m" is above the "2m" in effect, so it is ignored. Set it in your own configuration file if you want it`.
- A range of a length of time is written as lengths of time, in the error and in the reference: `expected 1m to 1h`, not `expected 60000 ms to 3600000 ms`. The comparison is still in milliseconds, and the JSON Schema still gives them (`x-cortex-milliseconds`).

**The readers, checked.** Every reader takes the merged `Settings`, so it gets the range and the class:
- `cortex-core`'s `retry_policy` maps the retry keys and the speed keys onto `cortex_llm::RetryPolicy` and `SpeedPolicy`, through `Settings::retry_for`, which puts a provider's own timeouts in place of the retry section's. Both are ranged.
- `cortex-llm`'s client prefers the provider's `ProviderInfo` timeouts, which come from the same entry or from the built-in local entries' constants (five and ten minutes, inside the range).
- `context_settings` in `cortex-core` hands `context.background` to `cortex-agent`.
- `-c` and the environment go through the same validation, so the range binds them too.
- No headless flag, environment variable or `session/set` setting carries any of these keys.

**Fixed: a supervisor's session overrides.** The one reader that bypassed the class was `session/new`'s `config` overrides. They rank as the command line, so the trust rules never see them, and a client that is not a `user` could set any key classed as a preference, every wait and timeout included: `retry.idle_timeout = "1h"`. The tighten-only keys are not in the set `only_preferences` lets such a client give, so it is now refused all of them, a shorter value too, as it already was `shell.max_timeout` and `[limits]`. The coordinator agreed for the owner: no supervisor has a real user before the task leases. Taking a lower value would mean comparing it with a second load. That is an outstanding item for step 37: [a-supervisor-lowering-a-wait](../../outstanding/a-supervisor-lowering-a-wait.md). Tested in `cortex-core`'s `settings::a_supervisor_cannot_start_a_session_above_the_level_it_could_set`, and recorded in [11-rpc-protocol.md](../../architecture/11-rpc-protocol.md).

**The tests that bite.** `mutations/step-09-settings.breaks` has 25 breaks: one for each key's range, one for each change of class, the comparison of a length of time and against the value in effect, the warning, the check over the schema, and the supervisor's overrides. Results are below.
