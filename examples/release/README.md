# release: a file command beside a script command

Two commands for a release, one of each kind:

- `/changelog:draft [since]`, an extension's command: it reads the commits since a tag (or any revision) with `git log`, and sends a prompt asking for a changelog entry made from them.
- `/release-notes <version> [audience]`, a command file: a prompt with blanks, which asks for the release notes of a version from its changelog entry.

Draft the changelog with the first, put the entry in your changelog, then turn it into release notes with the second.

This folder is laid out as your configuration directory is: `commands/` and `extensions/`.

## What it shows

- **A command file** (`commands/release-notes.md`): Markdown with front matter. The front matter gives the menu's description and the arguments, `version` and `audience`, the second with a default. What you type after the name fills them, in order or by name (`/release-notes 2.1.0 audience=operators`), and the body, with `{{version}}` and `{{audience}}` put in, is sent as your prompt. A command file runs nothing: it is text, so it needs no approval.
- **A command with code** (`extensions/changelog/main.js`), registered with `cineris.command`. Its function is given what you typed, runs `git log` through `cineris.exec`, and returns the text to send. The model is told that the text is the extension's.
- **A program run through its manifest:** `cineris.exec("git", …)` is checked against the manifest's `exec` list, then decided by your permission level and rules as the same command typed in the shell would be. Where the sandbox confines programs (Linux), a command's program may reach no network unless the manifest says `exec_network = true`, and may write no folder at all; where nothing confines them (Windows), the approval says so.

## What its manifest asks for, and why

| Capability | Value | Why |
|---|---|---|
| `exec` | `["git"]` | It reads the commits with `git log`. No other program may be run. |
| `exec_network` | `false` (left out) | Reading the log needs no network. |

Nothing else: it reads or writes no file through Cineris and hooks no event.

## Install it

The extension:

```
cineris ext add examples/release/extensions/changelog
```

Cineris reads the folder, shows what the extension may do and the command it registers, and asks you to approve it. A yes copies the folder into your configuration directory's `extensions/changelog/` (`~/.config/cineris/` on Linux, `%APPDATA%\cineris\` on Windows) and pins it by its hash. Where there is no terminal to ask at, run it once to see the hash, then again with `--hash <the hash shown>`.

The command file is copied, not approved: put `commands/release-notes.md` in your configuration directory's `commands/` folder, for every project, or in a project's `.cineris/commands/`, for that project alone. If both have a `release-notes.md`, yours is used.

## Try it

```
/changelog:draft v2.0.0
/release-notes 2.1.0
```

With no revision, `/changelog:draft` drafts from the last 200 commits. A revision that begins with `-` is refused before git is started, since git would read it as an option.

### When Cineris asks before `git log` runs

At the `supervised` level Cineris asks before the extension's `git log` runs, in two cases: in a project you have not trusted, since a repository's own configuration can name programs that git starts; and on Windows, or wherever the session's shell is PowerShell or `cmd`, since a line that only reads may there start another program than the one it names.

**Answer allow once.** It lets that one run go and nothing else, and it is the narrower choice.

A rule takes the question away, and it reaches further than the extension. **An `exec` rule is not the extension's own: the model's shell tool may then run the same line without asking too, in every project, those you have not trusted included.** So if you add one, have it name the extension's line word for word, and no more:

```toml
[[permissions.rules]]
effect = "exec"
match = "git log --no-merges --max-count=200 --oneline --no-decorate --no-show-signature --end-of-options *"
action = "allow"
```

The `*` stands for the words after the options: the revision. It comes after `--end-of-options`, after which git reads no word as an option and refuses one that looks like one. So this rule allows no other option, such as `--output`, which writes the log to a file, or `-p`, which runs the diff programs a repository's configuration can name. A rule of `git log *` would allow both, to the model as well as to the extension.

What a repository's own configuration can make git start, this line closes where an option does: `--no-show-signature` keeps git from checking each signed commit's signature, which the configuration can turn on (`log.showSignature`) and run through a program it names (`gpg.program`). What no option closes on every git:

- **A partial clone's fetch.** A repository set up as a partial clone has git fetch an object it lacks from its promisor remote, and the configuration names the program that reaches that remote (the remote's URL, `core.sshCommand`). Any object the walk through the history needs and the repository lacks starts it: in a run with no revision as well as in one whose revision names a commit the repository lacks. `git --no-lazy-fetch` stops that from git 2.45 on, and an older git refuses the option, so the extension does not pass it.
- **A pager.** git starts the pager the configuration names (`core.pager`, `pager.log`) only when its output is a terminal. Cineris runs git with none, for the extension and for the model's shell.

So in a project you have not trusted, answer the question each time, and allow the line only for a repository you know.

This example is run as a test on Linux and Windows, with the rule above in the user's configuration: `crates/cineris-cli/tests/system/examples.rs`. The command's script is run on its own in `crates/cineris-ext/tests/it/examples.rs`, and the rule is asked about lines it must not allow in `crates/cineris-policy/tests/it/decide.rs`.
