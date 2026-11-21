# Permissions, approvals and sandboxing

Three layers, each with a different job. The distinction matters because only one of them is a security boundary.

| Layer | Job | A security boundary? |
|---|---|---|
| **Policy rules** | Decide allow, ask or deny for each planned tool effect. | No. Rules reason about what a command says, not what it does. |
| **Approvals** | Put a person in the loop when the policy says ask. | No. People approve things they have not read. |
| **OS sandbox** | Limit what a child process can touch, whatever it runs. | Yes, where it is enforcing. |

The sandbox enforces. Approvals decide when to step outside it. Rules exist to reduce how often a person is asked. The threat model is in [../security.md](../security.md).

The default behaviour follows the [vision](../vision.md)'s autonomy policy: reads run, edits inside the project apply with a snapshot to undo them, and shell commands ask first unless they are on a safe list. The owner accepted this posture, and how far the sandbox goes on each OS, on 2026-11-12 ([decision 0009](../decisions/0009-sandboxing.md)).

## What is checked

Every tool call is planned before it runs ([05-tools.md](05-tools.md)), and the plan lists its effects.

| Effect | Target | Examples |
|---|---|---|
| `read` | a path | `read`, `search`, `list` |
| `write` | a path | `write`, `edit` |
| `exec` | a command line and working directory | `shell` |
| `network` | a host, or any | `fetch`, MCP over HTTP |
| `ext` | an extension or MCP tool by name | extension and MCP tools, plus any of the effects above that an extension's script asks the host to perform |

Built-in tools, extension tools and MCP tools all go through the same check. There is no path around it.

**What an answer depends on.** The engine (`cineris_policy::Engine`) answers allow, ask or deny from these and nothing else, so a test that holds them holds the answer, and a review that changes one asks whether the answer should change with it:

- **The call:** the tool's name, its default for a call nothing else decides (`allow` for the built-in tools, `ask` for an extension's or an MCP server's), and every access its plan lists, with its kind and target: a path, or a command line with its working directory, read in the shell's language (POSIX, PowerShell or `cmd.exe`) by the command analysis ("Shell command analysis", below).
- **The lease:** its level and the scope of what a command may touch ("Leases", below).
- **The rules,** each with where it came from (the user's configuration, the project's, a flag, a learned command), and whether the project's allow rules apply: the user trusted the project and approved its rules as they are now ("Project trust", below). Whether the project is trusted at all decides, separately, whether the git commands that read the repository's own configuration count as commands that only read.
- **The approvals already given:** the ones stored for the project ("always allow in this project") and the ones held for the session.
- **The sandbox:** whether it is enforcing, and its write roots. A command that would be asked about unconfined can be allowed confined.
- **What no level relaxes:** the hard limits, the harness's own files (protected paths) and the files that look like secrets.
- **Where it is:** the workspace's root and the home directory, for `~` and for what is inside the workspace.
- **The file system, at the moment of the decision:** a path a call names is followed as the system follows it (links, junctions, case on Windows), so the same call can be answered differently once a link is made. `Lexical` reads paths as text and is for tests that grant nothing. **Within one response,** a call is decided when it is planned, before the calls the model wrote ahead of it have run; so the loop plans and decides again every call that has not started once a call that may have changed the file system has finished, or a rule has been taken back ([03-agent-loop.md](03-agent-loop.md), "A call is decided again"). `ln -s . d` and then `cp evil d/.bashrc` is asked about as a copy to `.bashrc`.

The engine's clock only dates an approval as it is given, and its store only keeps one: neither changes an answer, though the approval they record changes the next ones. A lease's budget and its end are held by the loop, not by the engine's answer. `cargo xtask decide` prints the answer for a command with the inputs set by its flags ([../testing-and-ci.md](../testing-and-ci.md), "Asking the permission engine").

## Autonomy: one model

How much the agent may do without asking is described by one model. It covers a single session, a time-limited piece of delegated work, and a Coordinator team ([17-coordinator.md](17-coordinator.md)). There is no separate "permission mode", "lease" and "team level": every session holds a **lease**, and every lease has a **level**. The owner asked for this on 2026-11-12 ([decision 0026](../decisions/0026-one-autonomy-model.md)).

### Levels

A level says who decides when no rule has already settled the question: the **holder** of the lease, which is the agent, or its **supervisor**. The supervisor of an ordinary session is the user. The supervisor of an Employee is its Manager, as far as the Manager's own lease reaches, and the user beyond that.

| Level | Reads | Edits inside the lease's scope | Commands on a safe list | Other commands | Anything beyond the lease's scope |
|---|---|---|---|---|---|
| `read-only` | holder | refused | holder, for read-only commands | supervisor | supervisor for reads; writes refused |
| `supervised` (default) | holder | holder, with a snapshot | holder | supervisor | supervisor |
| `guided` | holder | holder, with a snapshot | holder | holder, for the kinds of command the lease names, inside an enforcing sandbox; otherwise supervisor | supervisor |
| `autonomous` | holder | holder, with a snapshot | holder | holder, inside an enforcing sandbox | supervisor |
| `unrestricted` | holder | holder | holder | holder, with no sandbox | holder |

- **`supervised` is the vision's default**, on every OS: reads and in-project edits do not ask, and shell commands ask unless they are on a safe list. The command line and configuration call a session's level its *mode* (`--permission-mode supervised`).
- **The scope** of a lease is its rules and its sandbox policy: which paths may be edited, which commands may run, whether tools may use the network. For a session's ordinary lease the scope is the workspace. "Beyond the scope" covers leaving the sandbox, writing outside the workspace, and network access from tools.
- **The safe list** is three lists together: the built-in table of read-only commands (below), commands the user approved for the session or the project, and commands learned for the project and accepted by the user ([14-adaptation.md](14-adaptation.md)), such as its build and test commands.
- **The sandbox applies at every level except `unrestricted`.** A command that runs, whether it was asked about or not, runs inside the sandbox where one is enforcing. Asking and confining are separate protections.
- **`guided` and `autonomous` need an enforcing sandbox.** They are refused, with an explanation, where the sandbox is not enforcing: Windows until its sandbox step, or a Linux kernel without Landlock. There, `supervised` is the highest level short of `unrestricted`, for single sessions and for teams alike.
- **`unrestricted` must be chosen explicitly**, by flag or user configuration. A client of `cineris serve` may ask for it only of a server started at it (step 27). A project cannot select it, nor `guided` or `autonomous`. It cannot be given to a team or passed down to a child session. The hint row shows any level above `supervised` prominently.
- **The decisions a team adds** (the plan, starting Employees, merging, spending past a threshold) follow the same ladder. Their rows are in [17-coordinator.md](17-coordinator.md).
- A front end that cannot ask (headless without pre-approval) treats "supervisor" as "refused" and says so in the result.

**As built in step 12** (`cineris-policy`, `level`). The table is one function, `decider(level, matter)`, with a column for each kind of thing to decide, and a test holds it against the rows above.

| Level | Read | Read beyond the scope | Edit | Write beyond the scope | Read-only command | Safe command that may write | Other command | Network from a tool | Leave the sandbox |
|---|---|---|---|---|---|---|---|---|---|
| `read-only` | holder | supervisor | refused | refused | holder | supervisor | supervisor | supervisor | supervisor |
| `supervised` | holder | supervisor | holder | supervisor | holder | holder | supervisor | supervisor | supervisor |
| `guided` | holder | supervisor | holder | supervisor | holder | holder | supervisor | supervisor | supervisor |
| `autonomous` | holder | supervisor | holder | supervisor | holder | holder | holder | supervisor | supervisor |
| `unrestricted` | holder | holder | holder | holder | holder | holder | holder | holder | holder |

- **The scope of a standing lease is its writable roots:** the workspace and the session's private temporary directory, plus `sandbox.write`. A read or an edit under one of them is inside the scope. The sandbox's `read` set is not asked: it says what a child process may read, and a file tool's read outside the workspace asks ([05-tools.md](05-tools.md), "Paths").
- **Three things are beyond the scope wherever they are.** A path outside the writable roots. A write to a protected path ("Protected paths", below): whoever can write one can run code later. And a file that looks like a secret, read or written ("Files that look like secrets", below).
- **A level that refuses is final.** At `read-only` a write does not happen, whatever a rule allows. Rules settle what a level leaves open; they do not open what it closes.
- **A rule still holds at the levels that ask least.** A `deny` is a denial at `unrestricted`, and an `ask` is a question.
- **`guided` reads like `supervised` in this table** because the kinds of command a guided lease names are allow rules of that lease, and rules are settled before the level is asked. It cannot be chosen until task leases exist (step 37).
- **Where the sandbox does not enforce,** `autonomous` is refused when the session starts, with the reason and what to use instead (the error code `sandbox_required`), and every other level shows one line, "sandbox: off", that says what follows: commands on a safe list run without asking, every other command asks first, and one that is approved runs with the user's full rights. Since step 26 that is a session on Windows, on a Linux kernel without Landlock or with Landlock older than version 3, in a build without the `sandbox` feature, or with `sandbox.enabled = false` ([decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md)).
- **A level this build does not know decides nothing:** every matter is refused, and a session cannot be started at it.
- **Leaving the sandbox,** the last column, was added in step 26: a command the model asked to run outside the sandbox ("Leaving the sandbox", below). It is asked about at every level but `unrestricted`, where nothing is confined to leave. Where the sandbox does not enforce there is nothing to leave either, and it decides nothing.

### Leases

Everything a session is allowed to do is one value.

```rust
pub struct Lease {
    pub id: LeaseId,
    pub grant: Grant,                   // what is allowed
    pub stop_when: Vec<StopCondition>,  // when the holder must stop and report
    pub expires: Expiry,                // Never | At(time) | EndOfTask | EndOfSession
    pub granted_by: Grantor,            // User | Parent(LeaseId)
}

pub struct Grant {
    pub level: Level,
    pub rules: Vec<Rule>,               // allow / ask / deny
    pub sandbox: SandboxPolicy,         // writable roots, protected paths, network
    pub models: Vec<ModelId>,
    pub budget: Budget,                 // money, tokens, requests, wall-clock time
    pub limits: HardLimits,             // what no level relaxes
}

// `LeaseExt`, in `cineris-policy`: the lease type is the protocol's.
impl LeaseExt for Lease {
    /// The widest lease inside both. Never wider than `self`.
    fn narrow(&self, requested: &Lease, sandbox: SandboxState) -> Result<Lease, NarrowError>;
    /// True if everything `self` permits, `parent` permits too.
    fn is_within(&self, parent: &Lease) -> bool;
}
```

There are three kinds of lease, and they are the same type.

| Kind | Who grants it | Lasts | Used for |
|---|---|---|---|
| **Standing lease** | Built from configuration, flags and the user's stored approvals. | The session. | Every session. Its level is the configured mode. Its budget is the configured session and daily limits. |
| **Task lease** | The user, for one piece of work, confirmed in exact terms. | Until the work is done, the time or budget runs out, a stop condition fires, or the user revokes it. | "For the next 30 minutes or 2 dollars, edit under `src/` and `tests/`, run the build and the tests, and stop if the tests fail three times running." |
| **Team lease** | The user, to a Manager session. Each Employee holds a **sub-lease** narrowed from it. | As a task lease. | The Coordinator. |

- **Granting a task lease.** The user says what they want in a sentence, or picks a preset. The model may draft the terms, but a draft is only text. The harness shows the exact terms it will enforce (scope, level, budget, time, stop conditions) and the user confirms them. Only a client in the `user` role can create, extend or revoke a lease ([11-rpc-protocol.md](11-rpc-protocol.md)). Nothing a model writes can.
- **Inside a task lease** the holder does not ask for what the lease covers. **At its edge** it does not start asking question after question: it stops at the next safe boundary, falls back to the standing lease, and reports what it did, what remains of the task, and what it would need to continue. The user can extend the lease or carry on step by step.
- **Stop conditions** are checked by the harness: verification failed a set number of rounds in a row, an action outside the scope was needed, a spending threshold was crossed, the time ran out. They are data, not instructions to the model.
- **A parent can hand a child a subset, and only a subset.** A session that starts another session (a run, or an Employee) proposes a lease, and the harness gives the child `parent.narrow(proposed)`. Narrowing is defined field by field: the lower level; the parent's `deny` and `ask` rules always carried over; `allow` rules kept only where the parent allows the same thing; writable roots intersected; the network allowed only if both allow it; budgets capped by what the parent has left; an expiry no later than the parent's; the stop conditions of both.
- **What reads the lease.** The permission engine and the sandbox launcher read only the current lease's grant. The limits ledger ([03-agent-loop.md](03-agent-loop.md)) charges the lease's budget before each request.
- **A task lease cannot exceed what the user could configure.** Managed configuration caps it, and a project cannot grant one.
- The hint row shows the level when it is not the default and, for a task lease, what remains of its time and budget. `/lease` shows the full terms.
- Property tests check that `narrow` always returns a lease within both inputs, and that no sequence of narrowing widens anything.
- **A lease that is not understood in full is refused whole.** The wire form of a lease, a grant, a rule, a sandbox policy, a budget, the hard limits and a stop condition rejects any field and any value it does not know, with the error code `lease_not_understood`. Elsewhere the protocol ignores what it does not know; here that would let a build that predates a restriction drop it, and hold a wider lease than was granted ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)).

**As built in step 12** (`cineris-policy`, `lease`). The lease type is the protocol's, so `narrow` and `is_within` are an extension trait on it, `LeaseExt`, and the engine adds no other way to read a lease.

- **`narrow` returns a result.** One pair of leases has no lease inside both: a proposal that names models, under a parent that names models, with none in common. An empty list means "the configured models", which would be wider, so that pair is refused (`NarrowError::NoModelInCommon`) and nothing else is.
- **`unrestricted` is never handed on.** The child's level is the lower of the two and at most `autonomous`.
- **`narrow` checks the level it gives,** as a session's start checks its own: it is told whether the sandbox confines the child's commands, and refuses a level the child could not be started at (`NarrowError::Level`): `autonomous` where the sandbox does not enforce, `guided` before step 37. So no caller can hand a child a level a session could not hold (found by the review of step 12).
- **An allow rule is kept only when the parent has the same rule,** by its effect and its pattern as text. A child that asks for `cargo test *` under a parent that allows `cargo *` does not get the rule: deciding that one pattern lies inside another is where a mistake would widen a lease, so it is not attempted. A proposal names the parent's rules it wants.
- **Roots are compared exactly.** A writable root reaches only folders spelled as it is, so that on Windows it never reaches a folder that differs only in case.
- **An expiry that cannot be ordered becomes a stop condition.** A lease that ends "at the end of the session" and one that ends "at noon" cannot be put in order ahead of time. The child's expiry is the event, and the moment is added as a `time` stop condition, which the harness enforces the same way. A time nobody can read is taken to have passed.
- **Budgets are narrowed as written.** To cap a child by what the parent has left, the caller narrows a copy of the parent's lease whose budget is the remainder (`with_budget`).
- **Rule patterns are relative to each session's own workspace.** A parent's `deny write secrets/**`, carried to a child in another worktree, protects that worktree's `secrets/`. The Coordinator step checks that this is what a team wants.
- **The property tests check against an oracle of their own,** which reads roots and times by its own rules and calls none of the code it checks. They are given roots that share a prefix of their text (`/work` and `/work2`), that differ only in case, or are spelled with `.`, `..` and two slashes, and times that are one moment in several spellings (`10:00+01:00` and `09:00Z`) or a fraction of a second apart.

**In the first release:** the types, the standing lease, the levels `read-only`, `supervised`, `autonomous` and `unrestricted`, and `narrow` with its tests. Settings profiles, headless flags and the non-interactive policy of runs all compile to leases. **After it:** task leases and the `guided` level (one build step), then team leases with the Coordinator, which is built on that step. See the [build plan](../build-plan.md).

## Rules

Rules live in configuration ([08-configuration.md](08-configuration.md)).

```toml
[[permissions.rules]]
effect = "exec"
match  = "cargo test *"
action = "allow"

[[permissions.rules]]
effect = "write"
match  = "**/.env*"
action = "deny"

[[permissions.rules]]
effect = "read"
match  = "~/notes/**"
action = "allow"
```

- **Matching.** Path effects match with globs against the canonical path. Exec effects match against the analysed command (below). Extension and MCP tools match by `server/tool` name.
- **Evaluation.** All matching rules are collected. `deny` beats `ask` beats `allow`. If no rule matches, the level's default applies. Order in the file does not matter, so a broad allow can never hide a deny.
- **Layers.** Managed rules cannot be weakened by user rules, and user rules cannot be weakened by project rules. A project's rules can add `deny` and `ask`. A project's `allow` rules take effect only if the user has trusted the project and are shown at that time.
- **Built-in deny rules** protect the harness's own files and credentials: the configuration directory, credential storage, and session logs of other projects.

### Rules as built

Built in step 12 (`cineris-policy`, `rules` and `glob`).

**Patterns.**

| Effect | The pattern is | Examples |
|---|---|---|
| `read`, `write` | A path glob. | `**/.env*`, `src/**`, `~/notes/**`, `*.lock` |
| `exec` | Words, each a pattern for one word of the command. A last word that is `*` alone stands for any further words. | `cargo test *`, `git status`, `npm run test:*` |
| `network` | A host, compared in lower case. | `docs.rs`, `*.example.com` |
| `ext` | A tool's full name. | `docs/find`, `docs/*` |

In every pattern `*` is any run of characters, `?` any one, and `[abc]`, `[a-z]`, `[!a-z]` a set. In a path glob `*` and `?` stay inside one name, `**` as a whole name is any number of names, and `{a,b}` is either. There is no escape character: a special character is written as a set of one, `[*]`. The matcher is our own (about 300 lines), because only `cineris-tools` may link the search libraries ([01-workspace.md](01-workspace.md)); it is tested against the cases `globset` documents and against a reference matcher.

**Where a path glob is anchored.**

| Written | Read as |
|---|---|
| `/etc/**`, `C:\Tools\**`, `\\server\share\**` | An absolute path. On Windows `\` and `/` both separate. |
| `~/notes/**` | Under the home directory. |
| `**/.env` | In a rule that holds back (`deny`, `ask`): anywhere, in the workspace or outside it. In a rule that allows: anywhere in the workspace. |
| `src/**`, `docs/*.md` | With a separator: under the workspace root. |
| `*.lock`, `Cargo.toml` | With none: that name at any depth in the workspace. |

A pattern that ends in `/**`, or in a separator, matches the directory itself as well as what is in it, so that a rule on a tree covers a listing of its top.

**A rule that holds back is matched as widely as it can be, and a rule that allows as narrowly.**

- **Where `**` starts.** A `deny` or an `ask` for `**/*.pem` holds for every such file on the machine. An `allow` written the same way allows them in the workspace only: an `allow write **` that reached the home directory would hand over the shell start-up files and the SSH keys in it. An allow that is to reach further says where: `~/notes/**`, `/srv/data/**`, `/**`.
- **Case, on Windows.** A `deny` or an `ask` matches a path without regard to case, and also exactly. An `allow` matches exactly, against the resolved path, which is in the file system's own spelling. So no spelling gets round a deny. And an allow for `App` never grants `app`, which is another folder in a directory marked case-sensitive. Since every path is resolved before it is matched, an allow works however the *path* was typed; it is the *rule* that must spell the folder as it is stored, and one that does not simply does not match. A drive letter, and a share's server and name, have no case on disk and are always folded.
- **Commands.** An `allow` matches one whole simple command of a line the analysis models, from its first word: `git status` allows `git status` and not `git status --short`, and a rule for `git` does not allow `./git` or `/tmp/git`. In PowerShell, which finds commands without regard to case, the program's name is compared that way and `git.exe` is `git`. A `deny` or an `ask` matches wherever its words appear in the raw line, one after another: `rm *` matches `sudo rm -rf x`, `echo x; rm y`, `bash -c 'rm y'` and `/bin/rm y`. It matches too much on purpose, and it is the only match there is for a line that could not be analysed. The line is tried a second time with every quote, backslash, back-tick and caret taken out of its words, as the shells take them out, and a line break after one of them taken out with it: `"g"it push`, `g\it push`, `gi''t push` and `g\` then `it push` on the next line are `git push` (found by the review of step 12 and the independent review of its fixes). What no match catches is a name the line makes as it runs (`$(echo git) push`), or a pattern the shell expands to the program (`/usr/bin/gi? push`): **a `deny` on the raw line of a command the analysis does not model can be dodged**, and at `autonomous` and `unrestricted` such a line runs. Where the sandbox enforces, it is what holds it.
- **A call that cannot say which host it reaches** meets every rule that holds back, and is allowed only by an allow for every host (`*`).

**Who wrote a rule.** Each rule reaches the engine with its source: the managed configuration, the user's, the command line, the project's, the project's learned commands, a stored approval, a session approval, or a lease. An `allow` from the project or from its learned commands is set aside until the user has trusted the project, and the set-aside rules are kept for a front end to show. The configuration loader already holds such rules back ([08-configuration.md](08-configuration.md)); the engine applies the same rule again where rules are enforced, so that nothing upstream has to be right for it to hold. Layers need no rank beyond that: a managed `deny` stands whatever a user rule allows, because `deny` beats `allow` whoever wrote them.

**A user's own skill folder may be read unasked** (since step 28, **only while the user's own setting `permissions.read_skill_folders` is on, and it is off by default**: the owner's answer, [steps/28/answered.md](../steps/28/answered.md), "The owner's answers"; narrowed by the maintainers). Off, a read of a skill's file outside the workspace is asked about, as any read there is. On, each skill a session finds in the home directory's roots (`~/.cineris/skills/`, and `~/.agents/skills/` and `~/.claude/skills/` while `compat.skills` is on) gives the session a rule of its own, `allow read <folder>/**`, when its folder and the workspace are apart, neither within the other, and the folder is not the home directory or a folder that holds it, each judged by place (`cineris_base::location_within`). A project's skills give none, nor does a user's whose folder is in the workspace (its root a link there, or the workspace the home directory) or holds it (its author working in its own `scripts/`): the workspace's reads, and its look for files that look like secrets, stay exactly as they were. The folder is where the skill really is, as `skills::find` judged it, in the file system's own spelling, every `*`, `?`, `[`, `]`, `{` and `}` in it taken as itself; a pattern that would not compile gives no rule. It is a session approval, source `session`, made as "allow for this session" makes one (`Engine::record`, a finding's rules as one answer, so that the set is compiled again once) and taken back as one is (`Engine::revoke`): listed by `permissions/list` and revocable, kept over a change of level, never saved. No new kind of rule, and nothing in the engine changed, so it is matched as every allow is: a `deny` or an `ask` on a path in the folder still wins; a write there is asked as before; a path in it that leads out through a link is judged where it leads. **A file in the folder that looks like a secret is read unasked, and found by a search,** as under any allow of a folder: a `.env` or a `credentials.json` that a skill's scripts use, kept beside its `SKILL.md`, reaches the model without a question. That is why it is the user's choice, off unless they turn it on: some providers keep what they are sent. A project's configuration cannot turn it on (a project's file that sets it is ignored, and the session says so), and a change to it takes effect at the next session. **The user is told:** the first time a session makes such rules, once, "N of your skill folders may be read without asking, because `permissions.read_skill_folders` is on. Each is an approval of this session's, which /permissions lists and can revoke."; and when the setting is turned off while a session runs, after the note that it takes effect in sessions opened from now on, "This session still reads your skill folders without asking, including skills added to them later: revoke those approvals with /permissions, or start a new session with /new." Neither is given to the model. A skill whose folder is a link out of its root is no skill and has no rule. The rules follow the skills found ([12-extensions.md](12-extensions.md), "Hot-reload"): at a turn's start that finds the skills again, a skill gone, or moved where it may not have one (its root pointed into the workspace), loses its rule, and one added gains one. A rule the user revokes is never given again in the session, though its folder drops out of a finding and is found again. A rule some approval holds already is not added again, and is not the skills' to take back.

**A learned command's rule is one exact line** (since step 17). `exact_command_rule` writes the allow rule of an accepted command of the project's profile with the writer an approval's exact-line rule uses: the line word for word, and no rule at all for more than one command, a redirection, anything the shell expands, a home path, a word with a blank, or a word with `*`, `?`, `[` or `]`, which a pattern would read as a wildcard. (The rule is matched as every allow is: in PowerShell the program's name is compared without regard to case.) A running session is given its learned rules, and has them taken back, with `Engine::set_learned`, which replaces every rule of the source `Learned` and compiles the set again: an accepted command stops asking at once, and one the user turned off asks at once. A list with anything but `allow exec` in it, or one that does not compile, leaves no learned rule in force. The rule is the line and nothing else: the directory an entry was learned in is not part of it, so an accepted line is allowed wherever in the workspace it runs, and a line that names a program by a relative path (`./gradlew test`) names whichever file that is there. Nothing about evaluation changes: the allows wait for trust as above, `deny` and `ask` beat them, and the standing lease keeps the rules the session started with, as it does when an approval is given.

**A rule that cannot be compiled refuses the whole set,** and the session does not start: a rule that is skipped could be the one that denies.

**The built-in protection** is not a list of rules but a part of every rule set that nobody wrote and nobody can remove. It denies, so no allow and no level lifts it.

| Closed | To |
|---|---|
| The configuration directory (configuration, and credentials on Linux) | Reading and writing. |
| The user's configuration file, wherever `CINERIS_CONFIG` puts it | Reading and writing. |
| The credentials file and its lock, wherever they are kept, and on Windows their own folder (`%LOCALAPPDATA%\cineris\credentials`), with any temporary file a write left there | Reading and writing. |
| The state directory (trust, the user's own pins, stored approvals, what was learned) | Reading and writing. |
| The data directory | Writing. |
| The session directories of other projects | Reading. This project's own are outside the workspace, so reading one asks. |

Closing the state directory to writing is what makes trust and approvals mean something: a model that could write `trust.toml` could trust its own project, and one that could write `pins.toml` could approve its own change to an extension of the user's.

Where each of these is on each system is one table, beside what is closed in it: [08-configuration.md](08-configuration.md), "Where each system keeps the harness's files". The credentials file has a row of its own because on Windows it is not in the configuration directory, and before the review of step 13a nothing closed it to reading there.

The protection holds for a path a command only names, too: `cat` of the credentials file, or `cp` to it, is refused and not asked about. A command that searches a tree with one of these directories somewhere in it is asked about.

### Files that look like secrets

Reading or writing a file whose name says it is likely to hold a secret asks first, at every level short of `unrestricted`, wherever the file is.

| Looks like a secret | Does not |
|---|---|
| `.env`, `.env.local`, `.env.production` | `.env.example`, `.env.sample`, `.env.template`, `.env.dist`, `.env.defaults` |
| `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks` | `keys.rs`, `pem-files.md` |
| `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ecdsa_sk`, `id_ed25519`, `id_ed25519_sk` | `id_rsa.pub`, `src/id_map.rs` |
| `.netrc`, `_netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `.envrc` | |

- **It is a default, not a rule,** and that is deliberate: `ask` beats `allow`, so as a built-in `ask` rule nothing a user wrote could lift it. As the default for such files it is what happens when no rule has settled the question. A user's `allow` lifts it (`allow read **/.env.test`). A project's `allow` lifts it only after trust. A `deny` closes the file for good.
- **The same default decides what a session's base state may keep** ([07-sessions-and-context.md](07-sessions-and-context.md), "The base state"): the store keeps a file only if the engine would let the agent read it without asking. The session store has no list of names of its own.
- The list is by name and short on purpose. An earlier list had `id_*`, and so held back `src/id_map.rs`.
- **It is narrower than the list the session store had before step 12,** and that follows from the owner's decision in step 12 that the store keeps no list of its own ([steps/12/answered.md](../steps/12/answered.md)): `id_rsa_work`, `id_ed25519_github` and `.env-backup` are not on it, so a base state keeps them and a read of one is not asked about. A rule says otherwise where it matters: `deny read **/id_*`.

### Protected paths

Some files in a workspace decide what runs later, outside the sandbox and without anybody being asked. Whoever can write one can run code, or reach a secret, so writing one always goes to the user.

| Protected | Because |
|---|---|
| Everything in a `.git` directory, a file named `.git`, and a file named `HEAD`, at any depth | Git runs the programs its hooks and configuration name: a hook, a file-system monitor, a filter, a text converter, a pager. A nested repository, a submodule's or a worktree's configuration, and a bare repository made of plain files are read the same way. |
| `.cineris/` at the workspace root | The project's own configuration, hooks, extensions and instructions. |
| `.bashrc`, `.bash_profile`, `.bash_login`, `.profile`, `.zshrc`, `.zshenv` and `.zprofile` at the workspace root; `.envrc` at any depth | A shell, or `direnv`, runs them when it starts. `direnv` reads `.envrc` from the directory it is in and upward. |
| `.github/workflows`, `.github/actions`, `.gitlab-ci.yml`, `.circleci`, `.pre-commit-config.yaml`, `.husky` | A CI service runs them with the repository's secrets, and a commit runs the last two. |
| `.cargo/config.toml`, `.cargo/config`, `.npmrc`, `.yarnrc` and `.yarnrc.yml` at any depth, and `.pypirc` | They name a program to run for a build, or the registry a token is sent to. Cargo, npm and yarn read theirs from the directory a build starts in and upward, so one in a subdirectory applies to a build started below it as surely as one at the root does to every build (found by the review of step 12). |
| Whatever `sandbox.protect` lists | The user said so. An entry is an absolute path, a path under `~/`, or a path relative to the workspace. One that is none of these stops the session from starting: an entry that was skipped would not be protected. |
| Files that look like secrets | Above. |

**Who decides, at each level.**

| Level | A write of a protected path |
|---|---|
| `read-only` | Refused, like every write. |
| `supervised`, `guided`, `autonomous` | Asked about. `autonomous` lets a command run unasked and still asks about these. |
| `unrestricted` | Left to the holder: the level asks nothing. A user's `deny` and `ask` rules still hold there, and so does the built-in protection of the harness's own files. |

An extension's script's write of one (`cineris.fs.write`) is the exception: it is refused at every level, `unrestricted` included, before the engine is asked, since nothing looks at it afterwards as the look at the protected paths does after a command ([12-extensions.md](12-extensions.md), "Tier 4"). No rule and no answer lifts that refusal.

- **Every route asks.**
  - The `write` and `edit` tools.
  - A redirection: `echo x > .git/config`.
  - A command that names one: `cp evil .git/hooks/pre-commit`, `sed -i … .github/workflows/ci.yml`, `tee`, `rm`, `dd of=…`. This holds whatever allow rule or level lets the command itself run. The engine cannot know that a command only reads a file it names, unless the command is on the table of commands that only read.
  - `git config`, however it is written: it changes the configuration without naming the file.
- **An allow rule that names the path lifts the question,** for that path and by every route the engine decides: `allow write .github/workflows/**`. A project's own allow does so only after trust. It does not lift an extension's `cineris.fs.write` of one, which is refused before the engine is asked (above).
- **Names are compared without regard to case on Windows,** like every rule that holds back.
- **Reading one is not held back,** unless it also looks like a secret.
- **What each of these makes git or cargo run,** and what the harness's own git and the sandbox do about each, is in [security.md](../security.md), section 11.
- **What this does not cover** is a program that writes one of these without the command line naming it: `python3 fix.py`. At `supervised` such a command is asked about as a command. At `autonomous` it runs, and the sandbox is what holds the protected paths ("The sandbox", below). Two stated limits besides: **the directory above a protected path** is not itself protected, so `mv payload .cargo` or `cp -r staging/workflows .github` can make one (protecting every ancestor would ask about `cp x .`); and **a link that leads nowhere yet** is not followed to a protected path it names (`tee hook`, where a link `hook` to a workflow that does not exist yet was carried in by a repository).
- **After each confined command, the sandbox looks** (step 26, below): what a command wrote without naming it is found then, and stops the turn.
- **A link an earlier call of the same response made is followed** (since the re-decide step of step 12): a later call is planned and decided again when the earlier one has finished, so a copy, a `write`, an `edit` or a `read` through the link is judged where it leads, on every OS and at every level that asks. The sandbox's look only finds such a write afterwards, and only for a confined command.
- **The list in the lease, which the sandbox enforces, is narrower in two respects.** Of `.git` it has `hooks`, `config`, `config.worktree` and `info`, because `git commit` has to write the rest. The engine asks about every write a tool or a command line names anywhere in a `.git` directory. And it names the build configuration at the workspace root only: one at any other depth is not a fixed path, so the engine knows it by name, and the sandbox step looks for it by name, as it does a nested repository's configuration.

## Shell command analysis

Rules on commands are a convenience. They are useful only if they fail closed.

1. The command line is parsed into a sequence of simple commands. Pipelines and the `&&`, `||` and `;` operators are split, and each part is checked on its own.
2. If the command contains anything the analyser does not model, the rule result is "ask": command substitution, process substitution, backticks, `eval`, here-documents, redirection to a path outside the workspace, variable assignment prefixes, subshells, function definitions, or a parse error.
3. An `allow` rule applies only if every part is allowed.
4. A small built-in table marks commands and argument forms that are read-only (for example listing, printing, and status commands with fully enumerated safe flags). Only these are scheduled as read-only. Commands that can write through an option, such as `git`, `find`, `sed` and `tar`, are never in the table as a whole.
5. The analyser never runs anything. It never calls a shell, an interpreter or a program found in the workspace.

On Windows the same rules apply to PowerShell through a conservative tokenizer: one command, literal arguments, no script blocks, no invocation operators, no encoded commands. Anything else is "ask". `cmd.exe` syntax is not analysed at all.

### Command analysis as built

Built in step 12 (`cineris-policy`, `command`). **The analysis is two hand-written tokenizers that accept a small listed subset and refuse everything else**, not a parser of a shell's grammar ([decision 0031](../decisions/0031-command-analysis-by-allow-list.md), with the measurements). A parser returns a tree for everything a shell accepts and leaves the refusing to its user; a tokenizer that lists what it accepts refuses what nobody thought of.

**The POSIX subset** means the same in bash, dash, zsh and ksh.

| Modelled | Not modelled, and so asked about |
|---|---|
| Simple commands joined by `\|`, `&&`, `\|\|`, `;` and line breaks | `&` (a background job), `\|&`, `;;` |
| Words of letters, digits and `_ - . / : = + , @ %` | `$`, a back-tick, `( )`, `{ }` anywhere in a word, a backslash outside quotes, `#`, `!`, `^`, any other character |
| Single quotes; double quotes with no `$` and no back-tick | A quote left open |
| `~` and `~/…` for the home directory; a `~` inside a word (`HEAD~1`) | `~user`, `~+`, `~-`, a `~` after `=` or `:`, a word that begins with `=` (zsh's `=cmd`) |
| Globs `*`, `?`, `[…]`: kept, and flagged | A glob where a command is expected: `[`, `[[` |
| `< file`, `> file`, `>> file` with one digit before or none, the file a pattern too (kept, and flagged); `2>&1`, `>&2`, `>&-` | `<<`, `<<<`, `<>`, `>\|`, `&>`, `>&file`, `<(…)`, `>(…)`, several digits before a redirection (`20>f` is a descriptor to bash and an argument to dash) |
| | `NAME=value` where a command is expected |
| | A keyword (`if`, `for`, `time`, …) or a builtin that runs text as code or changes what later commands mean (`eval`, `exec`, `source`, `.`, `trap`, `command`, `alias`, `export`, `set`, `unset`, `shopt`, `pushd`, `exit`, …) as the command |

**zsh with `EXTENDED_GLOB`** reads a `~` inside a word as a glob operator, so `HEAD~1`, which the subset takes as plain text, is a pattern there. The option is off unless the user's `.zshenv` turns it on, and this is a stated limit.

**The PowerShell subset** is one command with literal arguments: words of ASCII letters, digits and `_ - . / \ : = +`; a whole word in single or double quotes, with no `$` and no back-tick in double quotes, and not empty, with no `"` in it, no `\` at its end and none of `& | < > ^ %`; `~` for the home directory; `*` and `?`, flagged. Everything else is asked about: `;`, `\|`, `&`, a line break, `$`, a back-tick, any bracket, `@`, `,`, `%`, `#`, `!`, `<`, `>`, a quote that is not a whole word, a parameter with a `.` or a `:` in it (PowerShell reads `-Path.5` as `-Path` and `.5`, and `-Path:x` as `-Path:` and `x`, which the comparison with its parser found on its first run in CI), any character outside ASCII that is not inside quotes, a typographic quote anywhere (PowerShell reads `‘ ’` as `'` and `“ ”` as `"`, and several dashes as `-`), a keyword, and `powershell`, `pwsh`, `cmd`, `Invoke-Expression`, `iex` and `.` as the command, which is where an encoded command would be.

**What a program receives is not always what PowerShell's parser read.** Windows PowerShell 5.1, which the shell tool runs where pwsh is missing, and pwsh 7.3 and later for a `.cmd` or `.bat` program (`npm.cmd`, `gradlew.bat`), build the program's command line the old way: a text with a space in it is put in double quotes, nothing inside it is escaped, and a text without one is passed bare. So a `"` in a quoted text ends that quoting early, and the rest becomes words of their own (`git diff --src-prefix 'a/ " --output=f'` reaches git with `--output=f`); an empty text is dropped and the words after it move up; a `\` at the end of a quoted text escapes the closing quote, and the next words join it; and `cmd.exe`, which runs a `.cmd` or `.bat` program, reads `& | < > ^ %` in a text passed bare (`npm test 'x&calc'` runs `calc`). Each of these is not modelled, in every PowerShell line, since the analysis cannot know which PowerShell runs it or what the program is (found by the review of step 12). A test on the Windows runner compares what a program receives through `powershell.exe`, through pwsh, and through pwsh passing arguments the old way, with the words the analysis gives.

**`cmd.exe` is not analysed, and neither is a shell this build does not know** (`fish`, `nu`): the shell tool starts any unknown program as if it read `-c <command>`, which is no reason to believe it reads the line like a POSIX shell. The engine is told the shell by its program's name (`ShellSyntax::of_program`).

**What "not modelled" means.** No allow rule applies to the line and no safe list does. The level then decides it as any other command: asked about at `read-only`, `supervised` and `guided`, and run at `autonomous`, where the sandbox confines it. A `deny` or `ask` rule is still matched against the raw line. This is rule 2 above, stated for every level: the analysis never turns an unmodelled line into "allow", and it does not ask where the level would not, **except for what its words could name.** Each word of the raw line, cut both ways a rule that holds back is matched, is checked as a path a command names: one of the harness's own files is refused, and a protected path, a file that looks like a secret or a path a rule holds back is asked about, at every level that asks about those (`cp evil .git/hooks/pre-commit #x` at `autonomous`). A line that merely mentions `.git/…` or `.env` is then asked about; that is accepted. Each word is checked once, and tried from the directory the line runs in, from the workspace root, and from every directory a `cd` among the words could lead to, taken from where the line started and from where the `cd` before it led (`cd .github && cp evil.yml workflows/ci.yml #`, `(cd a && x) && (cd b && y)`); a `cd -` or a `cd` to a `*` or `?` pattern among them could lead anywhere, and is asked about, and so is a line with more such places than are followed (32). The same holds for a word with a space or a shell's operator (`< > | ; &`, a back-tick, `$(`) in it in a modelled line, which may be a command line for another program (`sh -c 'cp evil .git/hooks/pre-commit'`, `bash -c 'cat</tmp/x>.git/hooks/pre-commit'`), tried from every place the part may run in. One of the harness's own files named in such a word is refused even where the word is only text, a commit message among them. The owner's decision on the second independent review of the fixes to step 12, widened by the final review.

**How a modelled line is decided.**

1. A `deny` or `ask` rule on the raw line or on any part is settled first.
2. Each part must be allowed: by the table of read-only commands, or by an allow rule. A line is on a safe list only if every part is.
3. **What a command names is checked as a read.** `cat` only reads, and `cat ~/.ssh/id_rsa` reads a private key. So every file a table command names, and the directory it runs in, is put to the same judgement a `read` call gets, and the command is safe only if each would be allowed unasked. Where a rule denies a named path, the command is asked about, not refused: an argument that reads like a protected file may be a search pattern (`grep .env README.md`), and nothing is lost by asking. The harness's own files are the exception: a command that names one is refused. For every other modelled command, an argument that names a path a rule denies or asks about, a protected path, or a file that looks like a secret makes the command one to ask about, whatever allows the command ("Protected paths", above). An option's value counts as a path it names: after a `=` (`--target-directory=.git/hooks`), and for an option of one dash every text from its third character on (`cp -t.git/hooks`, `cp -vt.git/hooks`, `sort -o.git/config`), since the analysis does not know every program's letters; and after the `:` of an option written with a `/`, as Windows programs take theirs (`/out:file`). Found by the review of step 12.

   **A pattern names every path it matches.** The shell replaces a word with `*`, `?` or `[…]` with the names of the files it matches: `cp evil .gi?/hooks/pre-commit` writes `.git/hooks/pre-commit`. This holds for an argument, an option's value, a redirection's file (which also takes the part off every safe list), a `cd` to a pattern (what comes after it is tried under the pattern), and in PowerShell a quoted pattern, which its own commands expand. Found during the fixes to step 12, and settled by the owner after their second independent review:
   - **The directory a pattern starts under** is judged as a path the command names: a pattern under `.git` is a pattern in a protected directory.
   - **The pattern is expanded against the directory as it is now**, as the shell will: the base is found through the resolver, listed, and each match is judged as the name written out, through any link: a rule that holds it back (`deny write **/*.lock` meets `rm *.lock`), a protected path, a file that looks like a secret (asked about as a read, for a command that only reads), and for a redirection, the scope. A name that begins with `.` is matched as the shells match it, only by a pattern that begins with a `.` written out. So `rm *` and `rm -rf dist/*` run at `autonomous` where nothing protected, secret or held back is there, and `cp evil.yml c?/workflows/x.yml` is asked about where `ci` is a link to `.github`. **The bound:** at most 1,000 names in one directory, and 64 directories for one pattern (`*/*/x` lists the base and each directory its first name matched); past it the pattern is asked about. A thousand names is a directory of sources or build output, and keeps the decision to a few milliseconds: on the reference machine a pattern over 1,000 files took 6.7 ms to decide, about 7 µs a name, nearly all of it matching the rules; a hundred files is under a millisecond. **The work of one command's decision is bounded too:** 10,000 units in all, a name listed or a path or pattern looked at (one unit for each 32 characters of it), so that a long word of option letters with a pattern in it cannot list the directory once a letter; past it the command is asked about.
   - **Its text is judged as well** where an earlier part of the line, or one beside it in a pipeline, may write (it could make names the listing would not see: `touch x && rm *`), and **instead** where the directory cannot be listed. What the directory holds is still judged after a part that writes, so `mkdir t && rm *.lock` meets `deny write **/*.lock` (found by the final review of the fixes to step 12). Then a pattern that could match `.git`, `HEAD`, a file that looks like a secret, build configuration read from where a build starts, or one of the protected paths, is asked about at every level that asks about protected paths, and so is one that starts above one of the harness's own directories. Read this way, `rm *` is asked about, since it could match `HEAD` or `id_rsa`, and `rm *.o` runs. A `[` is read as "anything to the end of the name", so that a class, a range or a quoted member is never misread. An allow rule does not lift a question the text asks.
   - **Patterns the listing does not answer alone.** A name that begins with `.` and could be `..` (`.?`, `.*`) matches more than the listing shows: dash, and bash before 5.2, match `.` and `..` with it, so `.?/.github` from `src` is the workspace's `.github`. Such a pattern is listed, so a rule meets what it matches at every level, and its text is judged as well, which asks about it at every level that asks about protected paths. A name `**` is listed as `*`, as bash reads it, so a link it matches is followed; zsh reads it as any number of directories, so it is also asked about where a rule or one of the harness's own directories holds back any path below where it starts, and its text is judged. (The last review of the fixes to step 12 found that skipping the listing for these lost a rule and a link.) A `]` with no `[` before it is no pattern, and the name is judged as written (found by the final review: `> .github/workflows/x].yml` was checked by nothing).
   - **After a part that makes a link** (`ln`, `link`, `mklink`, `cp -s`, `cp -l`), earlier in the line or beside it in a pipeline, every part that names anything, by an argument or a redirection, is asked about, a name written out included: the link could give any path another name, and no listing taken now can see it (`ln -s .github gh && cp evil.yml gh/workflows/x.yml`).
   - **Stated limits.** The directory can change between the listing and the run, and a link made earlier in the line by any other program (`tar x`, `git checkout`) is not seen; the sandbox holds what such a command writes: since step 26, where it enforces, a write outside the write roots is refused, and a protected path written in the workspace is found after the command, stops the turn and is kept for undo. An option the user sets (`dotglob`, `GLOB_DOTS`, `globstar`) that changes what a pattern matches is not known to the analysis. A link copied or moved as a link (`cp -a`, `mv`), or made through another program's name (`env ln`, `busybox ln`), is not seen as a link made.
4. **A search of a whole tree reads files nobody named.** The `git` commands that print what tracked files hold (`diff`, `show`, `grep`, and `log` with `-p` and the like) are asked about when a `deny` or `ask` rule on reading can match something in the tree they read, or when one of the harness's own directories is in it. The check is made on the rules' patterns, so it errs towards asking, and it holds at every level, as a rule does. The `search` tool makes the same search with every file put to the gate.
5. **`cd ./dir && …` is followed** where its effect is certain for every part after it: the parts after it are then judged from the new directory.
   - The `cd` has one plain argument and no redirection, past `--` and `-L`, which change nothing (`-P` takes `..` through links, and such a `cd` is not followed), and **the argument says where it starts:** `/`, `~`, `./` or `../`, or it is `.` or `..`. Any other name (`cd src`) is looked for in each directory of the shell's `CDPATH` first, which the environment can set and so can zsh's `cdpath` in a `.zshenv` that `zsh -c` reads: such a `cd` could lead anywhere, and is not followed. Write `cd ./src`.
   - **`..` is taken logically, as a POSIX shell's `cd` takes it:** it removes the name before it from the directory the shell believes it is in, whatever that name links to. Through a link `a` to `b/c`, `cd ./a/../..` goes up two from where the shell is, where the file system would go up one from `b/c` (found by the review of step 12, with bash, dash and sh). A path a later command names is found from the real directory, as the kernel finds it. **Where the logical directory is not there,** bash takes the target through links instead: if that leads elsewhere, the `cd` is not followed, and both are places (found by the final review of the fixes to step 12: `cd ./l/../workflows`, with a link `l` to `.github/x`). After a `cd` that is not followed, each later `cd` adds both readings.
   - It is not in a pipeline, where it runs in a shell of its own, and it does not follow `||`, where it runs only if what came before failed.
   - Everything after it is joined by `&&`, or is in a pipeline with something that is. Then nothing after the `cd` runs unless the `cd` ran and succeeded.
   - Any other `cd` that is not the last part takes the line off every safe list, and the level decides it. After `a || cd x && …` the rest runs whether the `cd` ran or not. After `cd x && a; b`, `b` runs in one directory if the `cd` succeeded and in another if it failed.
   - **What comes after a `cd` that is not followed is still checked for what it could reach from anywhere:** a path that starts at a root or at `~`, as it is; `git config`; and every other path a part names or redirects to, tried from the directory before the `cd`, from where each `cd` since would lead with no `CDPATH` (read past its options, `-P` through links too, and whatever it redirects), and from the workspace root, for a protected path, a file that looks like a secret, or a path a rule holds back. A `cd` to a pattern is not followed either, and a path after it is tried under the pattern (`cd .gi? && cp evil hooks/pre-commit`). A `cd -` before any other `cd` of the line goes to the directory the environment's `OLDPWD` names, which could be anywhere: what comes after it is asked about where the level asks about writing outside the workspace (step 13a takes `OLDPWD` out of the shell's environment). The independent review of the fixes to step 12 found `cd -- .git/hooks && cp evil pre-commit` running unasked. So `cd sub && git config core.fsmonitor ./run` and `cd .git && cp x hooks/pre-commit` are asked about at `autonomous`, where the rest of such a line runs.
   - A `cd` with a redirection is a command that writes the file, and is judged as one.
   - **A command that may write, run after a followed `cd`, is judged where it runs:** run outside the workspace, or in a protected directory, it is asked about, whatever rule allows it (`cd /tmp && make`, `cd ./.git/hooks && make`). **So is one the call's own working directory puts there** (since the re-decide step of step 12): before it, `make` with the `shell` tool's `cwd` in `.git/hooks` ran unasked at `autonomous`, while `cd ./.git/hooks && make` was asked about. A line the analysis does not model is judged the same way. (A working directory outside the workspace was asked about already, through the write of it the tool declares.)
6. **A redirection is a write of its file,** or a read, judged as one (a quoted `~` is a name, in a folder named `~`, there and in what a command that only reads names; `~\x` is a name outside Windows; in PowerShell a word that begins with `~` is judged both as the home directory and as such a name, quoted or not, since its own commands read a quoted one as home, PowerShell 7 passes a bare one to a native program as home, and Windows PowerShell passes a bare one on as it is): outside the scope it asks, to a protected path or a secret it asks, and where a rule denies it the line is refused, because a redirection is a write for certain. `/dev/null` is nothing, read or written.
7. **A line whose every part only reads is marked read-only,** which is how the loop comes to run it beside other calls ([03-agent-loop.md](03-agent-loop.md)).

The paths a command names are followed by the same resolver the file tools use, so a link in the workspace leads where the file system says. The analysis itself looks only at text.

**The table of commands that only read.** A command is on it with its options, never as a whole.

| Shell | On the table |
|---|---|
| POSIX | `ls`, `cat`, `head`, `tail`, `wc`, `grep`, `egrep`, `fgrep`, `fd`, `fdfind`, `find`, `sort`, `uniq`, `stat`, `file`, `du`, `df`, `tree`, `readlink`, `realpath`, `basename`, `dirname`, `pwd`, `whoami`, `id`, `uname`, `hostname`, `which`, `true`, `false`, `echo`, `date`, `sed` (only as `sed -n '<lines>p'`), and `git` with `status`, `diff`, `show`, `log`, `branch`, `tag`, `rev-parse`, `ls-files`, `blame`, `describe`, `grep`, `remote`, `stash list`, `version` |
| PowerShell | `Get-ChildItem`, `Get-Content`, `Select-String`, `Test-Path`, `Resolve-Path`, `Get-Item`, `Get-Location`, `Get-Date`, `Write-Output`, `Write-Host`, `Get-FileHash` and their aliases (`gci`, `ls`, `dir`, `gc`, `cat`, `type`, `sls`, `pwd`, `echo`, …), `whoami`, `hostname`, `fd`, and `git` as above |

- **Each entry lists the options that are safe, by their exact names.** An option that is not listed takes the command off the table: `sort -o`, `find -delete` and `-exec` and `-fprint`, `git diff --output`, `git -c …`, `fd -x`, `tail -f`, `date -s`. So does an abbreviation of a long option, which GNU programs accept and the table does not, and a parameter of a PowerShell command that is not written out in full.
- **An option that takes its value only attached is listed so,** and one is listed as taking the next word only where the real program takes the next word. Otherwise the table would skip that word as the value, and the program would read it as an option of its own: `git log --pretty --output=f` wrote `f` (found by the review of step 12). Each option with a value was checked by putting after it a word the program would act on as an option: against git 2.53, GNU coreutils 9.4, uutils coreutils 0.10, GNU grep 3.12, fd 10.3 and tree 2.3. Git's `-U`, `--unified`, `--relative`, `--pretty`, `--format`, `--min-parents` and `--max-parents` take theirs only attached; every other option listed with a value takes the next word, or the program refuses it. **And a `git` line with `--out` in any word is off the table,** whatever comes before it.
- **A subcommand that can write is listed only in the form that cannot:** `git branch` and `git tag` with no name, `git stash list`, `git remote` with no subcommand.
- **Programs that are languages are not on it:** `awk`, `perl`, `python`, `xargs`, `env`, `tee`, and `less` and `more`, whose keys run commands. `sed` is on it only to print lines, because its scripts can write files and run commands.
- **`printf` and `test` are not on it,** though they look harmless. In bash both evaluate an array subscript in their arguments (`printf -v 'a[$(cmd)]'`, `test -v 'a[$(cmd)]'`), which runs a command, quoted or not, and `printf -v` sets a variable such as `CDPATH`.
- **No option that follows links is listed** (`grep -R`, `find -L`, `ls -L`), and `diff` is not on the table, because it reads through the links inside the directories it compares. What a command names is checked name by name; what it finds by walking a tree must stay in the tree.
- **No search that reads the content of a whole tree is on it.**
  - `grep -r` reads every file under a directory, a `.env` included, and names none of them. It is off the table, and `grep` of named files is on it.
  - `rg` leaves out hidden files and what ignore files name, but the workspace can change both: a `.ignore` with `!.env` in it brings `.env` back, and so does `rg -t sh`, whose type names `.env` and lifts the rule on hidden files (found by the review of step 12). The owner took `rg` off the table altogether. Through the shell it is an ordinary command, decided by rules and the level; the `search` tool makes the same search with every file put to the read rules.
  - `git grep --untracked`, `git grep --no-index` and `git diff --no-index` read what git does not track, ignored and hidden files included, and are off the table. So is `git diff` with two operands or more unless the first two both read as revisions (`HEAD~1 HEAD`, `abc1234 main~2`): outside a repository, or with one of them outside it, `git diff a b` is `git diff --no-index a b`, and git 2.53 printed the hidden `.env` under `b` (found by the independent review of the fixes to step 12). `git diff main feature` is asked about for that reason.
  - **The limit that remains:** a file that git tracks, a file that looks like a secret included, when `git diff`, `git show`, `git log -p` or `git grep` prints it. A rule on reading closes this, by rule 4: with `deny read **/*.pem` the command is asked about.
  - Commands that print names and not content (`ls -R`, `find`, `tree`, `fd`, `du`) show the names of files a rule protects.
- **A glob takes a command off the table.** The names it expands to can begin with a `-`, which the program then reads as an option. In PowerShell a pattern inside quotes does too, and so does a path that is not a file's (`Env:PATH`, `HKLM:\Software`): PowerShell's commands expand `*`, `?` and `[…]` in a path themselves, so `Get-Content '.en?'` reads `.env`.
- **The program is named plainly:** `ls`, not `./ls` or `/tmp/ls`.
- **`git` is on the table only in a trusted project** (`Parts::repository_trusted`, below). A repository's own configuration can name programs that `git status` and `git diff` run ([../security.md](../security.md), section 2), and an untrusted repository can arrive with that configuration.
  - In a trusted project the `git` forms on the table are as safe as `.git/config` is. That is why nothing writes git's configuration unasked, in the workspace's repository or in one nested in it, and why `git config` is asked about ("Protected paths", above).
  - What is left is a program that runs unasked at `autonomous` and writes the configuration without the command line saying so. Only the sandbox holds that. From step 13 every shell command runs with git's file-system monitor turned off in its environment, which closes the shortest of those routes.

**The tokenizers are compared with the shells themselves.** A test generates lines, keeps the ones the analysis models, and checks that bash and dash pass exactly the words the analysis gave, and that PowerShell's own parser reads the line as one command with those words. The comparison found, on its first run, that dash and bash disagree about `20<file`.

Published bypasses of command allow-lists in other agent tools are summarised in the research notes. They are the reason for rule 2 and for treating this layer as convenience only.

## Approvals

When the answer is "ask", the core sends an approval request to the front end and the tool call waits.

**The request shows** the tool, the one-line summary from the plan, and the details: the full command and working directory, or the diff, or the path. Nothing is hidden or shortened in the detail view, and control characters are made visible.

**The choices.**

| Choice | Effect |
|---|---|
| Allow once | This call only. |
| Allow for this session | Adds a session rule, held in memory, for the matching effect. For commands, the rule covers the command's program and subcommand, not the whole line. |
| Always allow in this project | Writes a rule to the user's per-project state (not to the repo). |
| Deny | The call gets an error result. |
| Deny with a note | As deny, and the note goes to the model as the reason. |

An approval never outlives its scope. Session approvals are not saved. Stored approvals are listed and removed with `cineris permissions`.

**As built in step 12** (`cineris-policy`, `approvals` and `engine`).

- **What a choice would add is worked out before the question is put,** and travels with it, so a front end shows it in full: "always allow `cargo build *` in this project". For a command that is the program and its subcommand, with any further words: `cargo build *`, `npm run *`, `make *`. For a path, that one path. For a host, that host, written so that it matches nothing else: a host `*` is the rule `[*]`, not every host (found by the review of step 12). For an extension's tool, that tool.
- **For a program that works through subcommands, only a subcommand on a short list is approved with any further words:** one whose further words choose what is built, shown or changed in the project, and cannot name a program to run, a package to fetch and run, or a place to send the project (`cargo build`, `cargo test`, `npm run`, `git commit`, `git diff`, …). Any other subcommand, and the program with none, is approved as the exact line. Among those not on the list (found by the review of step 12): `npm exec`, `pnpm dlx` and `yarn dlx`, which fetch a package and run it; `git rebase` (`--exec`), `git bisect` (`run`), `git submodule` (`foreach`), `cargo watch` (`-x`) and `rustup run`, which run a command they are given; `git push`, `git fetch`, `git pull` and `git clone`, which reach the network and take a program to run there (`--receive-pack`, `--upload-pack`); `git grep` (`-O` starts a pager), `git init` (`--template` copies in hooks), and `go run`.
- **A program that runs its arguments is approved line by line.** For `sudo`, `env`, `xargs`, `timeout`, `nohup`, a shell, an interpreter, `java`, an editor with a command language (`vim -c`), `ssh`, `docker`, programs with a language of their own in their arguments (`find`, `sed`, `tar`, `rsync`, `npx`), **programs that reach the network** (`curl`, `wget`, `scp`, `nc`, cloud tools: a download can be written anywhere, `curl -o ~/.bashrc`), programs with an option that runs another (`rg --pre`, `fd -x`, `man -P`, a pager's `!`, `zip -TT`), and the like, the rule is the exact line, word for word. A version after a program's name (`python3.12`) is the same program: approving `sudo apt list` must not become "allow `sudo *`", nor approving one `find` "allow `find *`". The same holds for a program that works through subcommands when an option stands where the subcommand would be: `git -C sub commit -m x` is approved as that line, not as `git *`. Where the line cannot be written as a pattern (`bash -c 'rm -rf x'` has a word with a space), no rule is offered.
- **Where no rule could say the approval safely, only "allow once" and "deny" are offered:** a line the analysis does not model, a question an `ask` rule raised (an allow would not lift it), a call that reaches "any host". Rules are offered only when they would settle every reason the call was asked about.
- **"Always allow in this project" is offered only where it can be kept:** when the engine was given a store (`Question::storable`). Offered without one, it would allow the call once and save nothing, while the person believed it saved (found by the review of step 12).
- **An approval is only ever an allow, and only ever lifts a default.** A `deny` or an `ask` rule still beats it, and so does the built-in protection.
- **Stored approvals are the user's own:** they are kept outside the repository and apply whether or not the project is trusted. The file is `approvals/<project key>.toml` in the state directory, one `[[approval]]` table per rule with its id, the time it was given and the rule. It is written owner-only and atomically on every write, and read strictly: a key this build does not know, a newer `version`, or an entry that does anything but allow, refuses the whole file, which then grants nothing and is not overwritten.
- **An answer the server does not know is a denial,** as [decision 0028](../decisions/0028-enforced-values-fail-closed.md) says. A choice that was not on offer allows the one call and adds nothing.
- **A call decided again is not asked again what a person allowed** (since the re-decide step of step 12). The core keeps the question a person allowed, with the call's accesses as they were shown and the loop's own id for the call, for the turn; the same question about the same accesses is answered "allow" ([03-agent-loop.md](03-agent-loop.md), "A call is decided again").
- **A revoke, an answer and a change of level do not lose each other** (since the re-decide step of step 12). A change of level makes its engine, writes the new lease to the log, and then puts the engine in force; the new engine takes over the approvals of the one in force at that moment, and a revoke and the recording of an answer take the engine in force under the same lock (`EngineInForce` in `cineris-core`). Before, a revoke that came while the lease was written reached the old engine only, and the approval stood in the new one; an answer recorded then was lost, and the person asked again. The wait this lock can put on the core thread is [outstanding](../outstanding/the-core-thread-waits-on-a-revokes-write.md).
- **A change to the file holds a lock from the read to the write:** a file beside it with `.lock` added to its name, locked by the operating system as the credentials file is. Two sessions of one project that store an approval at the same moment both keep theirs. On a file system that says it cannot lock there is no lock, as for the session store; any other failure to lock is an error, and nothing is written.
- **A stored approval is not pinned to what it runs.** "Always allow `make *`" still allows `make` after the `Makefile` changes, and `npm run *` whatever `package.json` then says. That is by design and a stated limit: the approval is of a command, and what the project's files make it do is the project's. Where that matters, allow once.

**In the RPC protocol** an approval is a request from the server to the client ([11-rpc-protocol.md](11-rpc-protocol.md)). If no client answers within the configured time, the answer is deny.

**As built in step 13a** (`cineris-core`, `policy` and `clients`).

- **Who is asked.** Every client that has the session open, has the `user` role and declared that it answers approvals. The request carries the choices, the reasons the call asks (`reasons`) and the rules that "for this session" or "in this project" would add (`adds`), so a front end can show exactly what an answer allows from then on.
- **A user client that opens the session while a question waits is asked it too,** under the same id, and may answer it. A question whose askers have all gone waits for that, for its time limit, or for the turn to be interrupted; it is not answered for them.
- **Whose answer counts.** A `user` client's that was asked. An answer from any other client is refused with `role_not_allowed` and changes nothing: the question stays open. A `supervisor` is asked nothing and may answer nothing until task leases give it a grant to answer within (step 37). *Since step 27:* a `supervisor` that its server connected with a grant is put every approval of its session with the users, and its answer counts within the grant ("When a session is supervised", below); a user may answer at any moment until the request is settled, and the first answer taken settles it.
- **The first answer wins.** The request is closed with it; a later answer is told the request is not open, and every client of the session gets `request.resolved`.
- **The time limit** is the core's option, and is none by default: a person at a terminal is not hurried. The RPC server sets one (step 27). Past it the call is denied and the clients are told (`timed_out`).
- **A question that is withdrawn** (the turn was interrupted, or the response the call came from was thrown away) is closed, and the clients are told (`cancelled`).
- **Recording** an answer for the project writes a file, so it is done on the blocking pool. If it cannot be written, the call is still allowed, once, and the user is told.
- **With nobody to ask** (no client that may answer: headless mode), the call is refused with the engine's own explanation, and the refusal is kept: it is the headless result's `permission_denials`. With the core's `fail_on_denial` option the same refusal ends the turn, through the loop's `Stop`, with the outcome `denied` ([10-headless.md](10-headless.md)). A rule's own `deny` is neither: it is an ordinary refusal, and is not listed. **A core that keeps sessions** (`serve --socket`, since step 27) asks somebody later instead, where the session has no client at all: the question is put to nobody, and waits for a `user` client to open the session, for its time limit, as one whose askers have gone does. A session whose clients are there and answer no approvals is refused at once, as above.
- **Whether an answer may stop the turn** (`Policy::answers_may_stop`) is the core's `fail_on_denial` option, or commands being confined. A person's choices have no "stop", so with neither a later call that was allowed runs beside an open question. With the option, a question nobody can be asked ends the turn, and whether somebody can be asked is not known until it is put. While commands are confined, a command beside the question may change protected paths and stop the turn, and the answer is then a stop (found by the second review of step 26). So with either, the loop holds every later call with side effects behind an open question, whoever is there to answer.
- **A changed level** (`session/set`) starts a new engine from the same rules and what was approved so far, and records the old standing lease as ended and the new one as granted. What was allowed for the session stays the session's own, under the ids it had (`Engine::keep_session_approvals_of`, added to `cineris-policy` for this).
- **A tool that declares an effect this version does not know is refused.** The engine cannot be shown the whole of what the call does, so it is not asked to allow it. A level that cannot be held is refused and nothing changes. Raising the level needs a `user` client.

**When a session is supervised** by another session, its approval requests go to the supervisor first. The supervisor may answer only what its own lease covers, and passes the rest up to the user. A user attached to the session can always answer directly. See [17-coordinator.md](17-coordinator.md).

**As built in step 27** (`cineris-core`, `clients` and `policy`), inert until a supervisor is given a grant, which nothing on the wire does: a server gives it at connect (`Core::connect_with_grant`), and the steps of task and team leases decide where a real one comes from.

- **A session has at most one supervisor.** A second one's `session/open` is refused `supervisor_present`.
- **Every approval goes to the supervisor and the users at once,** when the supervisor declared `approvals` and has a grant. The orchestration layer decides what it answers and what it escalates (17-coordinator, steps 1 to 3); a user attached to the session sees every request and may answer it.
- **The grant is enforced on the supervisor's answer, whatever the layer did.** A denial is always taken. An allow is taken only when an engine made from the session's own engine template allows the call, with a lease whose scope is the session's standing lease narrowed by the grant (`standing.narrow(grant)`: writable and readable roots, protected paths, network, and every `deny` and `ask` rule of both, so a grant never widens what the session holds) and whose level and allow rules are the grant's (the level at most `autonomous`: `unrestricted` is never handed on; `guided`, not there yet, is read as `supervised`). A narrow that fails, a level the session could not hold, and a grant whose engine cannot be made (a rule this build cannot read) cover nothing, and the supervisor is told which and why. The engine that judges an answer and the notice of what applies are made by one function, so they never differ. The level and allow rules are the grant's because narrowed alone they would be the session's own, under which the call was asked about: no grant could then cover anything. Three limits hold besides (the review of step 27's second pull request):
  - **A grant never lifts a question about a path:** one beyond the writable roots, a file that looks like a secret, a protected path (built in, or `sandbox.protect`). Its `read` and `write` allow rules are not used, and such a call stays with the users. An allow of a command lifts no question about a protected path or a file that looks like a secret that a command line names, which the engine asks about whatever allows the command; a path beyond the writable roots that a command writes is the sandbox's to stop, not the engine's (next).
  - **A command is within a grant only where it runs in the sandbox it was judged in:** the session's own scope, confined. There its level (`autonomous`) or an `exec` allow rule of the grant covers a command, and the sandbox holds what the command writes. Where the narrowed scope (read, write, protected, network) is not the session's own (told by what its path sets hold, each within the other, not by how their roots are spelled or split), or the session's commands are not confined (at `unrestricted`, or where the sandbox does not enforce: Windows until its sandbox), neither does, since the engine leaves a command's writes beyond the roots to a sandbox that is not there (the second review of step 27's fixes found `cp x ~/.bashrc` covered by an allow of `cp *`). Its allow rules of tools and hosts (`ext`, `network`) still apply there: they are not commands. The supervisor is told all of this when it opens the session (11-rpc-protocol, "As built in step 27").
  - **The session is read when the answer comes,** not when the question was put: a change of level while the request is open reaches the judgement. And a grant whose expiry `at`, or whose `time` to stop, has passed by the core's clock covers nothing, and the refusal says the grant has expired; its other conditions to stop are not judged here.
- **An allow within the grant is taken as `allow_once`,** whatever scope it named (one wider is logged): it allows that call, settles the request, and never reaches `Engine::record`. Nothing is stored for the session or the project.
- **An allow beyond the grant is not taken.** It is refused with `role_not_allowed`, which is also logged, since a JSON-RPC response gets no reply, and the request stays open for the users: that is how the supervisor passes it up.
- **"A user's answer always wins"** (17-coordinator, step 4) is read as: a user may answer at any moment until the request is settled, and the first answer taken settles it. A supervisor that answers within its grant before a person has read the request settles it; the requests a person is left to decide are those beyond the grant.

## Project trust

Opening a project for the first time asks one question before anything from the project that can run code or grant permissions is used.

- **Needs trust:** project configuration that sets permission `allow` rules, hooks, extensions, MCP servers (including ones found in another tool's configuration file in the repository), and learned or shared commands used for the safe list.
- **Does not need trust:** project instruction files (`AGENTS.md`, `CLAUDE.md`), custom command files and skills. They are text for the model. They cannot grant anything, and the injection risk they carry is covered in [../security.md](../security.md). The shared project profile needs no trust to be read and listed either, and needs something else: each of its entries is a proposal to each user until that user accepts it, and reaches neither the prompt nor the safe list before (since step 17, [14-adaptation.md](14-adaptation.md), "Where it lives").
- Trust is stored per canonical project path in the user's state directory, with a hash of the trusted definitions. If a hook, extension or MCP server definition changes, it is disabled until the user approves the change. This closes the "approved once, changed later" hole.
- Headless runs cannot ask. They skip untrusted project resources and report that they did, unless a flag grants trust for the run. Over RPC the question is put to a `user` client that declared `trust`, as the interactive application is (below); a client that did not declare it is a headless run here.

**As built in step 23: the question** (`docs/steps/23/answered.md`, "The design, before building", point 3, with the owner's answers of 2026-11-15).

- **When it is asked.** When a session opens (`session/new`, `session/open`, `session/fork`), the client that opens it declared `trust` (the interactive application does; headless does not, and nothing changes for it), `--trust-project` was not given, and the project is not trusted, or is trusted with its rules changed since. A project the user said never to trust, or answered "not now" for in this process, is not asked about. Nor is one with nothing to trust: the question is asked when there is git to trust (the workspace is in a repository: trust lets git read the repository's own configuration), a rule, or a setting held back.
- **What it lists**, each a `TrustResource`: git (`repository`); every permission rule of the project's files, outside any profile and in each profile they define, with the profile (`rules`, `changed` when the project was trusted with other rules); and every setting held back for trust (`settings`), outside any profile by its key and in each profile the project's files define as `profiles.<name>.<key>`, whichever profile is selected (since the review of step 23: a project's profile can be selected, and its settings must have been shown), its value and what it does: runs code, widens access, raises cost, stops commands, or is a first limit ([08-configuration.md](08-configuration.md), "A first limit waits for trust").
- **How it is asked.** The open looks at the project's trust on the blocking pool, then puts `trust/request` to the client that opens it and waits on the core thread, then puts the session together with the answer. The wait ends as no answer when the client goes away (it then gets no session, and the core goes on) or the core is ending: `Core::shutdown` withdraws every open trust and rewind question (since the review of step 23).
- **The three answers.** *Trust it* pins the definition of the rules that were shown, not the files as they are when the answer comes, through the same functions as `cineris permissions trust`; the session is then put together as a start puts it, so a change made while the question was open reads as rules changed. *Not now* (`trusted: false`) is kept in the core only: not asked again in this process, nothing stored. *Never for this project* (`never: true`) is kept in the trust store as `[[refused]]`, beside the trusted projects: later starts do not ask either, and a session there says so at its open, with the command that changes it. Trusting the project lifts it.
- **One pin of every rule** (the owner's answer of 2026-11-15). The rules are pinned outside any profile and in each profile the project's files define, each profile's own with its name (`Definition::of_project_rules`, `assemble::pinned_rules`), so the pin is the same whichever profile is selected, and a change to any of them is a change. A project with no profile pins as it did before. A profile the managed file's lock on `profile` keeps from being selected is neither pinned nor listed: it never applies (since the review of step 23).
- **A profile only the project defines can be selected** (since the review of step 23). `--profile ci`, or the user's `profile = "ci"`, with `[profiles.ci]` in `.cineris/config.toml`: a load with no workspace (the binary's own, for the interface's settings) cannot see the project's profiles and does not refuse the name; each session's load, which reads the project's files, says whether it exists.
- **An answer that cannot be kept** (since the review of step 23). When the trust store cannot be written (the state folder read-only or full, its lock not to be had), the session still opens, with a notice: *trust it* trusts the project for that session alone, as `--trust-project` does for a run, while its rules are still the ones shown; *never* is taken as *not now*. An answer that says both `trusted` and `never` is refused where it is given, and the question stays open. **A yes after the project's trust changed meanwhile is not used** (the second review of step 23's fixes): when the question was about changed rules and the project's trust was taken away while it was open (revoked, forgotten or refused, by another client or another process), nothing is written and nothing is trusted, so neither the git the question left out nor the project past a never is trusted by it. The session opens with the trust as it now stands, with a notice; a session opened again asks afresh, unless the project was marked never.
- **From the command line.** `cineris permissions list` names a fourth state, *never trusted*, with the two commands that change it; `permissions trust` trusts the project and says when it lifts a never; `permissions forget` forgets the project's trust and its never alike, so that the next session there asks again; `permissions untrust` takes trust away and leaves a never as it is.

**As built in the follow-ups of step 23: what is new in a trusted project, and git pinned** (the owner's decisions of 2026-11-16; [steps/23/after-review/followups.md](../steps/23/after-review/followups.md)).

- **A trusted project is asked only about what is new in it:** its rules changed since they were approved, or git, now that it is a repository and git was never shown. `trust/request` says so with `trusted: true`, and the dialog asks "Trust what is new in this project?".
- **Its third answer is "Keep trusting it, without what is new"**, not never, which took away the trust given before, git's included. The label names what is new: "without the new rules", "without git", or "without these" when both are. *Keep* (`keep: true`) keeps the trust as it was, git's and the pin of the rules as they were, and is kept in the store: the hash of the declined rules (`declined_rules`), so that they stay held back and are not asked about while they are those rules, and a further change asks again; and git declined (`git = "declined"`), not asked about again. `cineris permissions trust` approves both. *Never* is offered only on a project's first question. A `never` to a trusted project's question, a `keep` to an untrusted one's, and a `keep` beside another answer are refused (`invalid_answer`), and the question stays open.
- **The dialog says what each answer does,** on a line under the answers (reference screen 2c): trust turns them on and asks again if they change; not now asks again at the next start; never does not ask again, and `cineris permissions trust` lifts it; keep keeps what was trusted and does not ask again until what is new changes.
- **Whether git was shown is pinned,** as the rules are (`git = "trusted"` in the project's entry, version 2 of `trust.toml`). Git is listed only when the folder is a repository at the time of the question, so a folder trusted before `git init` had git trusted unshown. Now `Parts::repository_trusted`, the shell tool's and the base capture's answer is "the project is trusted **and git was shown and trusted**". A trusted folder that has since become a repository is asked about git alone, its rules unchanged; until it is trusted git does not read the repository's configuration, and the session says why. `permissions trust` trusts git with the rest in a repository, and says so; `permissions list` names a fifth state, *trusted, but not its git*.
- **The store's version 2.** This build writes version 2 and reads 1 and 2. A version 1 store is read with git never shown for every project: each trusted repository is asked about git once more, and the session says the store was older. It is written as version 2 at its next change, owner-only and atomically as always. A build from before reads only version 1 and refuses a version 2 file whole, and so trusts nothing until it is updated (decision [0028](../decisions/0028-enforced-values-fail-closed.md)'s rule; `TrustStore`'s documentation). A version 1 file that holds `git` or `declined_rules` is refused, since no build wrote one.

**As built in step 12** (`cineris-policy`, `trust`).

- **A project is known by its key** ([07-sessions-and-context.md](07-sessions-and-context.md), "On-disk layout"), made from its resolved root in the case stored on disk. A folder that differs from a trusted one only in case is another project.
- **A definition is a kind, a name and a hash** of everything that decides what it does: a hook's command and each argument, an extension's manifest and script, an MCP server's command, arguments and tool definitions. The parts are hashed with their lengths, so moving one character from an argument to the next is another definition.
- **A project's rules are a definition too,** all of them, hashed together and without regard to their order: the rules that deny and ask as well as those that allow. The rules that deny and ask take effect without trust; they are pinned because what the user trusted was the allow rules *with* what held them back. A project that drops `deny exec make deploy` beside `allow exec make *` has loosened, and its allow rules wait until the user approves its rules again (found by the review of step 12). One that adds a rule that holds back has changed too: the new rule holds at once, and the allow rules wait. `ProjectTrust::rules_approved` is given every rule of the project as its files hold them now.
- **The engine is told two things about trust, side by side.**

| `Parts` field | Filled from | Decides |
|---|---|---|
| `trusted` | `ProjectTrust::rules_approved` with every rule of the project | Whether the project's allow rules apply. Any change to them sets it to `false` until the user approves again. |
| `repository_trusted` | `ProjectTrust::is_trusted` | Whether git's commands that read the repository's own configuration (`git status`, `git diff`) may be on the table of commands that only read. |

  So a trusted project that only adds a rule that holds back keeps git's forms that only read, while its allow rules wait for the user. The split was made in the fixes to step 12, for step 13a.
- **A definition stands in one of four ways.**

| Standing | Meaning | Used |
|---|---|---|
| Approved | Its hash is the one the user approved. | Yes. |
| Changed | It was approved, and has changed since. | No, until the user approves the change. |
| New | The project is trusted, and this was not among what the user saw. | No, until approved. |
| Untrusted | The project is not trusted. | No. |

- **The file is `trust.toml` in the state directory:** one `[[project]]` table per trusted project with its key, its root as it should be shown, the time, and a `[[project.pinned]]` table per definition. It is written owner-only and atomically, under a lock held from the read to the write (`trust.toml.lock`), so that a `trust` and a `revoke` at the same moment cannot undo each other, and read strictly: a key this build does not know or a newer `version` refuses the whole file, and then **nothing is trusted**.
- **Nothing a tool does can write it.** The state directory is closed to every tool by the built-in protection, at every level.

**As built in step 28: the user's own pins** (`cineris-policy`, `trust::PinStore`). The user's own extensions, in the configuration directory's `extensions/`, belong to no project, and are pinned in a file of their own beside `trust.toml`: `pins.toml` in the state directory, `version = 1` and one `[[pin]]` table for each kind and name, with its `kind` (`extension`; step 30 adds the user's MCP servers' pins of each tool), `name`, `hash` and `at`, when it was pinned. Pinning again replaces the pin of the same kind and name. A definition stands with it as with a trusted project (approved, changed or new; never untrusted). It is written owner-only and atomically under a lock held from the read to the write (`pins.toml.lock`), and read strictly, as `trust.toml` is: an unknown key, a value of the wrong type, another `version`, a hash that is not one, or two pins of one kind and name refuse the whole file, and then nothing is taken as pinned, and the error says why. A link at its name is not followed, neither read through nor written through (since the review of step 28). A kind the build does not know is kept and written back. **It is a file of its own, not a version 3 of `trust.toml`,** because a build that reads `trust.toml` up to version 2 refuses a version 3 whole, and would then trust no project at all for the sake of pins it has no use for. Like `trust.toml`, no tool can read or write it: the state directory is closed (`start::the_state_directory_is_closed_whole_to_every_tool` in `cineris-core`).

**As built in step 28's second pull request: a project's extensions in the trust question** (`assemble::look`, `ext::found::asked`; [steps/28/parts/pr2a1-found.md](../steps/28/parts/pr2a1-found.md)).

- **One reading of the store.**
  The question is shaped by the one reading of the trust store the session's trust was worked out from, never by a second, which a change in between could make another.
- **What it lists.**
  When it is put, the project's extensions are listed, each a `TrustResource` of kind `extension`, its detail in words: its name and folder; "It has changed since you approved it." when it was approved at another hash (`changed`); the programs it may run, said as they would run (below); the workspace files it may read and write, the hosts it may reach (a `*` as "any host on the internet, including addresses on your own network that have public addresses", and `*.example.org` as "any host under `example.org`"), what each event it may hook lets it see, whether it adds to the system prompt and its version; and last, on a line of its own, its author's description, labelled and quoted so that it cannot close its quotes.
  Each path, name and text of the manifest's is shown as code it cannot end (`cineris_base::in_code`, since the full review of step 28): what acts unseen is escaped, and so are a backtick and a `\`, so that a folder named to end its code cannot write sentences that seem the harness's after it.
  **What its programs can do is said as the session would start them.**
  Where nothing confines them on the system (Windows in the first release, no Landlock, the sandbox off, or a write root that holds the harness's own files), the detail says, after the extension's name and folder and before anything it may do: "Programs this extension runs are not confined on this system. They can do anything you can, including changing Cineris's own settings and approvals."; and it claims no limit on the network or on writing that the programs do not have. An extension that runs no program says nothing of confinement.
  Where they are confined, it says whether they may reach the network, that they read what the session's commands may read, and that they write only in the folders of the manifest's `write` globs and the session's own temporary folder: the folders the launcher grants, made by its own code (`cineris_ext::capability::write_roots`).
  Whether they are confined is judged of the load a yes puts in force, the project's own settings in it, where the question's own load may hold a setting back (since the fresh review of step 28); `cineris permissions trust` judges it as the sessions do, with the launcher they are given.
  One approved as it is now is not listed, nor one whose name a folder of the user's own extensions takes.
- **A write root that would turn the sandbox off is said first.**
  Where the sandbox would otherwise enforce, each write root of the project's own `sandbox.write` that a yes would add and that holds the harness's own files, judged as a session's start judges its roots (`assemble::harness_write_warnings`), is listed before everything else, a `TrustResource` of kind `settings` named `sandbox.write`: "This project's configuration asks to write in `~/.cargo`, which holds Cineris's own files. If you trust it, its sessions run without the sandbox.", the folder under `~` and shown as code it cannot end.
  So is such a root in each profile the project defines, since a yes trusts every profile: "This project's configuration asks, in its profile `dev`, to write in `~/.cargo`, which holds Cineris's own files. If you trust it, its sessions with that profile run without the sandbox."
  `cineris permissions trust` prints the same before it trusts, and says so of a project already trusted too: what it holds back is judged against the configuration with no trust, and the sentence is as true there. Leaving such a root out and keeping the sandbox on is step 32's ([the item](../outstanding/a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off.md)).
- **When the project's folder is looked in.**
  Only below the never and the not now: a project marked never, or answered not now in this process, is not looked in; one that is not trusted only by its question, never by its sessions.
  A trusted project with nothing else new is asked when an extension in it is new or changed.
- **A yes pins each extension listed, as it was read for the question,** with the rules when they were listed.
  What changed in a folder while the question was open is not what was pinned, so it is never run: each place the question read was stamped before it was read (since the full review of step 28), the session takes the extensions as the question read them, and its first look, before the first turn, sees the change, withdraws the extension and says it changed; the runtime, which reads the folder again at an extension's first call, refuses another hash besides.
  A yes the store cannot keep approves the extensions listed by the hashes shown, for that session alone, and says that rules and extensions will be asked about again.
- ***Keep trusting it, without what is new*** leaves the extensions listed unapproved, kept by the core for its run alone (no field of `trust.toml` records it): the core takes it as a not now, so no later session of the process asks about the project, and the next start asks again.
  The dialog's third answer then reads "Keep trusting it, without these" (unless git or the rules alone are new), and the line under the answers says that keep leaves the extensions off until the next start.
- **`--trust-project` approves no extension that is not pinned,** and the session names `cineris permissions trust`.
- **`cineris permissions trust`** shows the project's extensions as the question does, and asks whether to approve them too.
  **At a terminal the question decides, and `--with-extensions` is ignored there:** a yes (`y` or `yes`) pins them with the rules, and anything else, an empty line included, leaves them unapproved.
  **Only where there is no terminal to ask at** (standard input or standard output is not one) does `--with-extensions` approve them; without it, the command trusts the rules and says the extensions were left unapproved.
  So a script run at a terminal cannot answer with the flag: it is asked.

**As wired in step 13a** (`cineris-core`, `assemble::trust`). The core works trust out once for a session, and gives it to four places, the engine twice. It is two answers, because trust lets a project do two different kinds of thing.

| The answer | Who is given it | What it decides there |
|---|---|---|
| **The project is trusted, and its rules are the ones the user saw.** | Configuration (`cineris-config`'s `ProjectTrust`) | Whether the project's file may set what is marked as needing trust: a dearer model, a looser setting. |
| | The permission engine (`Parts::trusted`) | Whether the project's `allow` rules count. |
| **The project is trusted.** | The permission engine (`Parts::repository_trusted`) | Whether git's commands that read the repository's own configuration are commands that only read. |
| | The shell tool (`repository_trusted`) | Whether git is asked what a command changed. |
| | The capture of the base state | Whether git is asked for the repository's state. |

- **The first is about what loosens or costs,** so any change to the project's rules sets it aside until the user approves the change: the project's settings that need trust are held back, its `allow` rules are not used, and the user is told in a notice.
- **The second is about letting git read the repository's own configuration,** which can name programs for git to run. A change to the project's permission rules does not change what git would run, so it does not take this away: a project that adds a rule does not lose its base state, the exact record of what its commands changed, or `git status` as a command that needs no question.
- **`--trust-project` gives both, for the run.** A trust store that cannot be read gives neither, and the user is told.

**Trusting a project from the command line (step 13b).** `cineris permissions trust` trusts the project of the working directory, or approves the change of its rules when it is trusted already. It pins the definition of **every** rule the project's files hold now, allow, deny and ask, read as a trusted load reads them: the very set a start compares with the pin (`assemble::pinned_rules` since step 23, one function for both, every profile included), so the next start reads the project as trusted. An approval keeps the other definitions pinned for the project; a first trust pins only the rules (and, since step 28, the project's extensions it shows and is told to approve: "As built in step 28's second pull request", above). It prints the rules it pinned and the settings that trust now turns on. `permissions untrust` takes the trust away, `permissions list` says which of the three states the project is in (not trusted; trusted; trusted with its rules changed since), with its stored approvals, and `permissions revoke` removes those by id. A system test trusts a project this way and sees its model chosen on the next start, then set aside again when a rule is added.

## The sandbox

`cineris-sandbox` defines a policy and applies it when a child process starts.

```rust
pub struct SandboxPolicy {
    pub read: PathSet,        // default: everything readable
    pub write: PathSet,       // default: the workspace and a private temp directory
    pub protect: PathSet,     // inside `write`, but must not be modified
    pub network: Network,     // Off | Full
}
```

**Default policy at the `supervised` level:** read anywhere, write only inside the workspace and a per-session temporary directory, no network.

**Protected paths inside the workspace:** git's hooks and configuration (`.git/hooks`, `.git/config`, `.git/config.worktree`, `.git/info`), the project's `.cineris` directory, shell start-up files, CI configuration, and registry and build settings, as listed in "Protected paths" above, plus `sandbox.protect`. A process that can write these can run code later, outside the sandbox. A nested repository's configuration (`sub/.git/config`, `.git/modules/*/config`) is not a fixed path, so the sandbox step looks for it by name.

### How it is applied

The harness re-executes itself as a small launcher: `cineris` with a hidden subcommand, the policy passed on a file descriptor, then the command. The launcher is single-threaded and does nothing but apply the policy and `exec` the target. Doing this in a fresh process avoids the hazards of running setup code between `fork` and `exec` in a multi-threaded program. The cost is one extra process start of our own binary per command, which the performance budget covers.

The shell tool has the hook for it since step 11 (`cineris_tools::process::Launcher`): the tool hands the shell's command line to its launcher, and starts what the launcher returns, with the bytes the launcher gives as the standard input of what is started. That input is the file descriptor the policy travels on: the launcher reads it to its end and gives the command a closed input, unless the policy says the command's own input follows it (since step 28: an extension's `cineris.exec` with `stdin`, and a hook command's event). The process is started as a tree either way, so stopping it and cleaning up after it do not depend on the sandbox.

### Linux: in the first release

| Mechanism | Used for | Needs |
|---|---|---|
| `no_new_privs` | Required by the two below. | Nothing. |
| Landlock | Filesystem allow-list: read-everything, write only to the listed roots. | Kernel 5.13 or later with Landlock enabled. Rule coverage improves with newer kernels. |
| seccomp filter | Network off (deny creating sockets), plus denying the calls that would let a process undo the sandbox (new user namespaces, `io_uring`, and similar) and keeping every process in the command's group. Tracing other processes is kept inside the sandbox by Landlock. | Nothing. |
| Session, process group, death signal and the guard process | Clean-up. Built in step 11, in `cineris_sandbox::tree` ([decision 0030](../decisions/0030-process-tree-control-in-cineris-sandbox.md)), and used with or without the sandbox. | Nothing. |

- No root, no helper program, no user namespaces, no container runtime. This matters because Ubuntu 24.04 and later restrict unprivileged user namespaces by default, which breaks the namespace-based approach other tools use unless an administrator installs a profile.
- **Detection.** At start-up the harness asks the kernel which Landlock version it has. If Landlock is missing or disabled, the sandbox is reported as "not enforcing" in the hint row, commands that run are not confined, and the `guided` and `autonomous` levels are refused.
- **Known limits, stated plainly.**
  - Landlock rules only add permissions. With the workspace writable, protected paths inside it cannot be blocked by Landlock alone. The harness therefore checks the protected paths after every sandboxed command and, if one changed, stops the turn, shows the change and offers to restore it from a checkpoint taken before the command. This detects; it does not prevent.
  - Reads are not restricted by default, so a sandboxed command can read files such as SSH keys. It cannot send them anywhere while the network is off. A stricter read policy is a configuration option.
  - Network control is all or nothing. There is no per-host allow-list.

### As built in step 26

Step 26 built the Linux sandbox in `cineris-sandbox` and wired it into the core and the binary. What the kernel must offer, and what a command may reach, are [decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md).

**The parts.**

| Part | Where | What it does |
|---|---|---|
| The policy | `cineris_sandbox::policy::Policy` | What the launcher applies: what may be read (everything, or the trees under some roots), the write roots, the network, the session's temporary directory, and the Landlock version the harness counted on. It is made from the lease's grant: the lease's `SandboxPolicy` stays the policy of record, and the protocol type is not changed. Its wire form is a list of NUL-terminated fields read strictly: a field or a value it does not know, a field given twice, a missing one, or anything after its end refuses it whole ([decision 0028](../decisions/0028-enforced-values-fail-closed.md)). Since step 28 an optional `input` field gives the length of the command's input (at most 1 MiB, in decimal digits with no sign or leading zero), and exactly that many bytes follow the policy's end; any other number of bytes after the end, a length not written so, or a second `input` refuses it whole. |
| Detection | `cineris_sandbox::support::detect` | One system call at start-up for the kernel's Landlock version. `Support::assess` says whether the sandbox enforces and names what it leaves open. |
| The launcher's two sides | `cineris_sandbox::launch` | `Launcher` (the harness's side): what to start in place of a command, and the policy's bytes. `run` (the launcher's side): the body of the hidden subcommand `cineris __sandbox -- <command>`. It reads the policy to its end, applies Landlock and then the seccomp filters to itself, marks every descriptor above the standard three to close, gives the command a closed input and `TMPDIR`, and becomes the command. With an `input` field (step 28) it puts the bytes after the policy in an anonymous memory file before it confines itself (close-on-exec, written, sealed against writing, growing, shrinking and further seals, rewound), and gives the command that as its standard input in place of the closed one: the command can read its input and cannot change it. `Launcher::wrap_with_input` is the harness's side of it. |
| The backends | `cineris_sandbox::confine`, behind the feature `backends` | Landlock and seccomp ([decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md)). Off by default, so that `cineris-tools`, which needs only process trees, links neither; `cineris-core`'s `sandbox` feature, on by default, turns it on. Without it the launcher refuses to run anything and detection says the build has no sandbox. |
| What the model is told | `cineris_sandbox::explain` | One line added to the result of a command that failed: the path of a write the sandbox refused (an absolute path outside the write roots, named in a line that reports a refusal), that the network is off (from what programs print when they cannot make a socket or look up a name), or the launcher's own reason when the command did not run. It names only what it can: a refusal of a path inside the workspace is the file system's own, and is not explained as the sandbox's. |
| The session's sandbox | `cineris-core`, `sandbox` | The state a session reports, the launcher it hands the shell tool, and its private temporary directory. |

**The state a session reports.**

| The system | `sandbox.enabled` | The binary gave a launcher | State | `autonomous` |
|---|---|---|---|---|
| Linux, Landlock 6 or newer, `sandbox` feature | true | yes | enforcing | starts |
| Linux, Landlock 3 to 5 | true | yes | enforcing, with one line at the session's start: a command can still signal a process outside the sandbox | starts |
| Linux, Landlock 1 or 2 | any | any | off: `NoSandbox::LandlockTooOld`, which names the version and what it lacks | refused |
| Linux without Landlock, or with it turned off | any | any | off: `NoLandlock` | refused |
| A write root (the workspace, the temporary directory or `sandbox.write`) holds the harness's own configuration, state or data directory, or lies inside one, or holds the user's configuration file (as named, or where a link leads), or the program started as the launcher | true | yes | off: `HarnessFilesWritable`: a confined command could change what it is allowed, or replace the launcher | refused |
| A build without the `sandbox` feature, or a core given no launcher | any | | off: `NotBuilt` | refused |
| Windows | any | any | off: `UnsupportedSystem` | refused |
| any | false | any | off: `TurnedOff` | refused |

- **The state follows the level.** At `unrestricted` nothing is confined, so a session there reports the sandbox off (`SessionInfo::sandbox`, and the session's state after a `session/set`), and an approval's `outside_sandbox` is true. The `initialize` result reports whether a session with the sandbox enabled would be confined on this machine.
- **The binary gives the core its own executable** (`CoreOptions::launcher`) as it gives the guard's: the same path, the subcommand `__sandbox`. `CoreOptions::sandbox` replaces detection for a test, which can then say the kernel has no Landlock, or an old one.

**The network** ([decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md), accepted by the owner on 2026-11-13). With `sandbox.network = "off"`, the default, a confined command makes no socket but netlink's route one and a connected pair: no TCP or UDP, to another machine or to this one's own loopback addresses, no Unix socket by its path or an abstract one. With `"full"` it makes the internet's sockets (TCP and UDP, loopback included) and netlink's route one, and nothing else: still no Unix socket of anyone else's (the user's session bus, an agent or the Docker socket would let it out of the sandbox), no virtual machine's socket to its host, no raw or packet socket. **What it costs:** a test suite that starts a server on loopback or on a Unix socket fails inside the sandbox, and so does a tool that talks to a local daemon (`docker`, a signing agent), and Python's `multiprocessing` with the `forkserver` start method (the default from Python 3.14). The way out is `sandbox.network = "full"` for loopback, or one approved run outside the sandbox (`sandbox: false`).

**Every command below `unrestricted` is started through the launcher** where the sandbox enforces: the shell tool hands the shell's command line to the session's launcher, and starts `cineris __sandbox -- <shell> <arguments>` with the policy on its standard input. The process tree is set up by the runner before the launcher starts, as before.

**It fails closed.** Once the session reports the sandbox enforcing, a failure to apply it fails the call and says why; nothing is run unconfined in its place. Each failure is tested: a policy the launcher does not understand, Landlock refused by the kernel (a parent that refuses its system call), a seccomp filter that does not load (the same), and a launcher that cannot start at all, which the runner reports as a command that was not started. The launcher's own line begins `cineris-sandbox:` and its exit code is 125.

**The temporary directory.** The lease names `cineris-<session>` in the system's temporary directory among its write roots (step 12). Since step 26 the core makes it when the session is assembled, open to the user alone, gives it to every confined command as `TMPDIR`, and removes it, with everything in it, when the session ends. Something already at that path that is not a private directory of the user's own (another user's, or a link) is not used: commands are then given no temporary directory, it is taken out of the launcher's write roots, and the user is told.

**Protected paths after each command.** Landlock cannot keep a path inside the writable workspace from changing, so the core looks at the protected paths before and after every confined command that may write (`cineris_sandbox::protect`; `cineris-core`, `checkpoints`):

- **What is looked at:** the lease's protected paths, each with everything under it, and, found by a walk of the workspace that follows no link, the protected names at any depth: in every repository the walk meets, nested ones and the submodules kept in `.git/modules` included, its `config`, `config.worktree`, `hooks` and `info`, and each worktree's `config.worktree`, `commondir` and `gitdir` (a module or a worktree that is a link is reported as the link, and not looked through); a bare repository made of plain files (a directory with `HEAD`, `objects` and `refs`); a `.git` file; `.cargo/config.toml` and `.cargo/config`; `.npmrc`, `.yarnrc`, `.yarnrc.yml` and `.envrc`.
- **How a change is told:** a path that appears, goes, or changes its kind, mode or content; where the content was not kept, its size, modification time, change time or inode. The change time and the inode are set by the system alone, so a command that puts a file's modification time back is still found. The same bytes written again are no change. A protected path reached through a linked directory in the workspace is not looked through: the link is what is reported, and what was under the directory is reported gone.
- **What happens:** the result names each changed path and how it changed; the turn stops: no later call of it is decided as anything but a stop (since the re-decide step of step 12 every call that has not started is decided again when the command ends, so the turn stops at the first of them, a read included), an answer to a question that was open at the stop is a stop, and no call of it that changes anything runs, even one decided or approved before the stop (`StopsWithTurn`, around every tool of the session); and what each path held before is recorded in the round's checkpoint, so undoing the turn (`session/rewind` with `restore_files`) puts it back. Content is kept for files up to 256 KiB and 4 MiB in all. A file past that, a link, a special file, a change of mode, a directory, a path outside the workspace, a path that a link leads to when the command ends, and everything in a session kept in memory is found and reported, and the result names it as one to put back by hand. A file whose content and mode both changed keeps its content for undo, and its mode is named as one to put back by hand. **Undo never writes through a link:** a file whose directory on the way, or which itself, is a link when it is put back is left as it is and reported, so a command that swaps a directory for a link cannot have undo write where the link leads (found by the review of step 26; `cineris-session`, `snapshot::through_a_link`).
- **Bounded:** the walk lists at most 20,000 directories and stops after a quarter of a second. Past either the look is incomplete, and **it warns and does not stop**: the result says that the look stopped at its bound and a change further down would not have been seen. Stopping every command in a large workspace would make the sandbox unusable there; asking would ask about every command. **Unless the look before the command was complete:** then the command itself made the workspace too large to look at whole, perhaps to hide a change further down, and the turn stops as for a change (found by the second review of step 26). A change to what *was* looked at is still found, and two looks are compared only where both reached, so a look that stopped sooner than the other reports nothing made or gone beyond it.
- **Nothing a command starts outlives it, so far as the harness can make sure.** A confined process cannot leave the command's session (`setsid` is answered as done without being done). It may make a process group of its own, which `timeout` and test runners need; when the command ends the tree sweeps the session, stopping every member it finds and killing the stopped ones, until a long run of empty passes or a bound. That clears an ordinary command's leftovers and, more often than not, a process that forks in a loop; it cannot be sure against a fork storm (freezing the whole tree at once needs a control group, left for step 32). So nothing rests on the kill winning: when the kill reaches its bound with the session alive the turn stops with a warning, and, whichever way the kill went, the look before every confined command is compared with the look after the last one, and at the session's close against the last look — a protected path that changed in between, and that none of the harness's own tools changed, stops the turn and is reported in neutral words (a process a command left running, or a change by hand). What the kill costs programs that make a terminal is in [decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md); the residual is in [outstanding/the-last-writes-of-a-killed-command.md](../outstanding/the-last-writes-of-a-killed-command.md).
- **The harness's own git runs inside the sandbox** while commands are confined (change detection, the base state, a file's earlier content): a repository's configuration can name a program for git to run, and a confined command may rewrite that configuration, so git runs read-only, with no write root and no network, and a program it names runs confined. The look at what an extension's program changed follows the program's confinement instead: inside the sandbox wherever the program's own launcher confined it, `unrestricted` included (since the fixes to step 28's process starts; [12-extensions.md](12-extensions.md)). Confined or not, git and what it starts are given no provider key variable and a `PATH` of absolute directories only, and git is never a program inside the workspace or a write root (since the harness's own processes' fixes; [security.md](../security.md), section 12). A git that must write to answer (a git-lfs filter) fails, and the change list says it is incomplete, with the reason ([decision 0032](../decisions/0032-what-the-linux-sandbox-needs.md), "After the third review"; [outstanding/confined-git-and-lfs-filters.md](../outstanding/confined-git-and-lfs-filters.md)).
- **A write root opened with no link followed:** the launcher opens each write and read root with `openat2(RESOLVE_NO_SYMLINKS)`, so a root a confined command swapped for a symbolic link is refused and the command does not run. A shell or a git whose real path lies inside a write root is passed over, where a confined command could have planted it.
- **The owner and extended attributes are refused** by the filter, inside the workspace too; a change of **mode** or **time** is allowed, because builds set them, and that a confined command can change the mode and times of a file outside the write roots — not its content — is a stated limit ([outstanding/modes-and-times-outside-the-write-roots.md](../outstanding/modes-and-times-outside-the-write-roots.md)).
- **What it costs,** measured on the reference machine in a release build: 23.6 ms a look for this repository with its build directory (3,278 directories), so about 47 ms a command, before and after. A workspace of 20,000 directories or more reaches the bound, a quarter of a second a look ([outstanding/the-cost-of-the-look-at-protected-paths.md](../outstanding/the-cost-of-the-look-at-protected-paths.md)).
- **Not looked at:** a command the user let leave the sandbox. Every confined command is looked at, one the engine marked as only reading included, since the program it runs may do otherwise. Nothing is looked at where the sandbox does not enforce.
- **Watched at any depth:** every repository's `.git/config`, hooks and `info`; a main repository's `.git/commondir`, followed to the common directory's configuration and hooks without following a link; and a `.cargo`, a directory or a link, with its `config.toml` and `config`. A protected file too large to keep whole is compared by a hash, so a same-length change with its times put back is found whatever the file system's change time does; one too large to hash marks the look incomplete.
- **Not on the list,** and so not watched: other files that git's configuration names inside the workspace (`core.hooksPath`, `core.fsmonitor`, an `include.path`), and protected names inside a `sandbox.write` root outside the workspace ([outstanding/files-git-runs-that-the-protected-list-does-not-name.md](../outstanding/files-git-runs-that-the-protected-list-does-not-name.md)).
- **A session kept in memory** has no checkpoints: a change is found and the turn stops, and the result says it cannot be put back from there.

**Measured** on the reference machine, release build, 200 runs, nothing else running: `bash -c 'exit 0'` alone 0.93 ms; through `cineris __sandbox` 1.71 ms. The launcher adds 0.78 ms, to the runner's 0.18 ms and the tool's 0.24 ms of step 11: about 1.2 ms in all against the 5 ms budget ([performance-budgets.md](../performance-budgets.md)).

### Linux: later, opt-in

A namespace-based backend (mount and network namespaces, with protected paths mounted read-only and a proxy for per-host network rules) where the system allows unprivileged user namespaces. It gives prevention where the default gives detection. It is optional because it cannot be relied on to work.

### Windows: in the first release

| Mechanism | Used for |
|---|---|
| Job Object, kill on close | The whole process tree dies with the command or the harness. Built in step 11 (`cineris_sandbox::tree`). |
| Job Object limits | Since step 26 every tree is held to 1,024 processes at once and, where Windows says how much memory the machine has, three quarters of it committed by the whole tree (`cineris_sandbox::tree::Limits`). A command that starts processes without end, or takes the machine's memory, fails there instead of taking the machine with it. |
| Approvals | Every shell command that no rule allows is asked. |

There is no filesystem or network confinement of child processes on Windows in the first release. The hint row says "sandbox: off", and the `guided` and `autonomous` levels are not available. Extension scripts are still confined, because their confinement is the script runtime and not the OS.

### Windows: later, after its own spike

An AppContainer-based backend: a per-session container identity, access-control entries that grant it the workspace, no network capability, inside a Job Object. It needs no administrator rights. It is a separate build step, after a spike, because three things are unproven:

1. Whether a console program behaves correctly under an AppContainer when attached to a pseudo-console.
2. How much of a developer's toolchain needs read grants outside system directories.
3. Reliable clean-up of the access-control entries after a crash.

Per-host network filtering on Windows needs a one-time administrator set-up in every tool that has it. It is out of scope.

### macOS: not a target now

The policy type is OS-neutral, so a Seatbelt backend can be added behind the same interface.

### What runs in the sandbox

| Child | Policy |
|---|---|
| Shell commands | The session policy. |
| Verification commands | The session policy. |
| Hook commands | Their own policy: read the workspace, no writes, no network, unless the hook's entry declares more and the user approved it. |
| Programs started by an extension script through the host (`cineris.exec`) | A launcher of the extension's own, at every level, `unrestricted` included (step 28): the session's reads; as write roots only the directories of the manifest's `write` globs (each glob's directory before its first wildcard: `out/**` gives `out/`, `**/*.rs` the workspace; a glob that ends in a separator its folder whole, `out/` giving `out/` too, since the fresh review of step 28, before which it gave the workspace) and the session's private temporary directory, its `TMPDIR`, and nothing of the session's grant; the network only with `exec_network = true`. The protected paths are looked at before and after a start that has a write root, as around a confined command: a change from outside since the last look refuses the start, and one the program made fails its call; either stops the turn. Where the sandbox does not enforce (Windows, no Landlock, the sandbox off) the program is started as it is, as the shell's commands are, and the manifest's `write` binds the script's `fs.write` and not its programs. Either way it starts only after the permission engine decided an `exec` of its line (and a `network` to any host with `exec_network`). |
| MCP stdio servers | Read the workspace, no writes, no network, unless the server's entry declares more and the user approved it. Many need the network. |

**What runs inside the harness process, and how it is confined.**

| Code | Confinement |
|---|---|
| Built-in file tools | The permission check on canonical paths. |
| Extension scripts | The script runtime ([12-extensions.md](12-extensions.md)). A script has no access to files, processes or the network of its own. Everything it does goes through host functions, and each of those is checked against the extension's manifest and the permission engine. This confinement does not depend on the OS, so it is the same on Windows as on Linux. |

### Leaving the sandbox

When a sandboxed command fails because of the sandbox, the result names the blocked operation. The model can ask to run the command again with more access. That request is an approval like any other, and the hint row shows when a command ran outside the sandbox.

**As built in step 26.**

- **The model asks with the `shell` tool's argument `sandbox: false`.** The argument is optional and additive: a call without it runs in the sandbox as before. The tool's result for a command the sandbox blocked tells the model of the argument ([05-tools.md](05-tools.md), "`shell`").
- **The plan says so.** Such a call declares the effect `unconfined` beside its `exec` (`cineris_tools::Effect::Unconfined`, `cineris_policy::Access::Unconfined`), so the engine is shown it and nothing about it is left to the tool.
- **It is asked about at every level but `unrestricted`,** where nothing is confined to leave: the column "Leave the sandbox" of the level table. A rule that allows the command does not lift the question; a rule that denies it still refuses it. Where the sandbox does not enforce there is nothing to leave, and the argument decides nothing.
- **Only "allow once" and "deny" are offered,** not the choices of an ordinary approval. A rule says what a command is, not where it runs: an "allow for this session" of `cargo build *` kept from this question would let every later `cargo build` run unasked *inside* the sandbox, which the user did not decide, and nothing could let a later one run *outside* it. So no answer is kept, and each command that leaves the sandbox is asked about. (The maintainers, for the owner, agreed this narrowing on 2026-11-13: [steps/26/answered.md](../steps/26/answered.md).)
- **The approval says the command runs outside the sandbox** (`CommandDetail::outside_sandbox`, per call), and its reason says with what: the user's full rights, writes anywhere the user can, and the network.
- **What the user allowed runs as it is,** with no launcher, and its protected paths are not looked at afterwards: the user allowed the user's own rights.

## Summary of what is realistic

| | Linux | Windows |
|---|---|---|
| First release | Enforcing filesystem and network sandbox for shell commands, verification, hook commands and MCP servers, with the stated limits. Extension scripts confined by the script runtime. Approvals to leave the sandbox. | Process-tree control. Approvals for every command not on a safe list. Extension scripts confined by the script runtime. No OS confinement of child processes. |
| Later | Namespace backend for prevention of protected-path writes and per-host network rules. | AppContainer backend after a spike. |
| Not planned | Anything that needs root or a daemon. | Anything that needs administrator rights, including per-host network filtering. |

## Invariants to test

What a review of this page's code should try to break ([testing-and-ci.md](../testing-and-ci.md), "Invariants to test"). The core's adversarial fixture (`crates/cineris-core/tests/it/adversary.rs`) runs a confined command and says what it reached.

| Invariant | How a review breaks it | Held by |
|---|---|---|
| A protected path never changes without the user being asked, or the turn stopping. | A script that writes `.git/hooks/*` or `.git/config` without naming it on its command line; `mv` or `cp -r` of a folder onto `.cargo` (the parent is not protected); a link that leads nowhere yet. | The look before and after a confined command (`protect`), `adversary::sys_an_adversarial_command_reaches_only_what_it_may`, `mutations/step-26*.breaks`. The parent and the dangling link are stated limits. |
| A change made between two commands is found. | A process left running (`setsid`, a double fork) that writes a hook after the look (step 26, finding 2; the third review's finding 2: a fork loop outlives a one-pass kill). | `sandbox::sys_a_change_between_two_commands_is_found_and_stops_the_turn`, the kill's sweep. |
| A stopped turn runs nothing more. | A second call of the same response, decided or approved before the stop; an answer to a question open when it stopped (step 26, finding 3; the second review's finding 7). | `sandbox::sys_a_stopped_turn_runs_no_more_commands_of_the_same_response` (with the fixture), `StopsWithTurn`. |
| Nothing a confined command writes is written outside the write roots. | A write to a folder outside; a write root swapped for a link before the next command (the third review's finding 6); a change of mode, time or extended attribute, which Landlock does not govern (finding 5). | `sys_commands_are_confined_where_the_sandbox_enforces`, the fixture's `outside`, `openat2(RESOLVE_NO_SYMLINKS)` on each root. |
| The harness's own git runs nothing a repository names, unconfined. | A filter, a text converter, `core.fsmonitor` or an `include.path` planted by a confined command, then `git status` by the harness (the third review's finding 1). | `GIT_HARDENING`, the git inside the sandbox; [security.md](../security.md), section 11. Files git's configuration names in the workspace are not watched ([outstanding](../outstanding/files-git-runs-that-the-protected-list-does-not-name.md)). |
| The harness's own files are never in a write root. | A workspace that is the home folder; `~/.cargo` in `sandbox.write` with `cineris` installed there; a link to the user's configuration (step 26, finding 9; the second review's finding 1). | `a_workspace_that_holds_the_harness_files_is_not_confined` and the tests beside it. |
| Undo writes only where it found the file. | A command that swaps `.git` for a link to `~/.ssh` before an undo (step 26, finding 1); a path now reached through a link (the second review's finding 6). | `sys_undo_does_not_write_through_a_link_a_command_planted`. |
| The look that stops at its bound does not hide a change. | 20,000 directories, then a protected file beyond the bound (the second review's finding 5). | `sys_a_command_that_makes_the_look_stop_short_stops_the_turn`. |
| With the network off, no socket reaches anything. | A Unix socket to the session bus or the Docker socket; netlink beyond route; a datagram pair that reaches journald by path (step 26, findings 6 and 10). | The seccomp filter's tests in `cineris-sandbox`; the datagram pair is [outstanding](../outstanding/datagrams-from-a-socket-pair.md). |
| A command left running does not outlive its call. | `setsid sh -c 'sleep 600' &`, a fork loop. | The fixture's `survivors`; `cineris-sandbox`'s tree tests. |
| A call never acts through a path an earlier call of its response changed without the engine judging where it leads; nor runs on a rule taken back while it waited. | `ln -s . d` then `cp evil d/.bashrc`; `ln -s .git d` then `write d/hooks/pre-commit`; `ln -s ~ d` then `read d/…`; a working directory made a link; an approval revoked, or the level lowered, while an earlier call's question is open (the re-decide step of step 12). | The round's second decision (`cineris-agent`, `round`), `Policy::narrowed`; `cineris-core`'s `redecide` tests and `cineris-agent`'s; `mutations/step-12-redecide.breaks`. |
| On Windows, a path is compared by place. | A root from the configuration written plainly against a canonical `\\?\` path, or in another case (step 15's instruction discovery; step 26's fixes, `find_outside`). | `cineris_base::same_location`, `location_within`, `strip_location_prefix` and their tests; the places that compare are listed in [steps/tooling-4/answered.md](../steps/tooling-4/answered.md). |
