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

At the `supervised` level Cineris asks before `git log` runs, unless your rules allow it, in two cases: in a project you have not trusted, since a repository's own configuration can name programs that git starts; and on Windows, or wherever the session's shell is PowerShell or `cmd`, since a line that only reads may there start another program than the one it names. Answer allow once, or allow it with a rule:

```toml
[[permissions.rules]]
effect = "exec"
match = "git log *"
action = "allow"
```

With no revision, `/changelog:draft` drafts from the last 200 commits. A revision that begins with `-` is refused, so that git cannot read it as an option.

This example is run as a test on Linux and Windows: `crates/cineris-cli/tests/system/examples.rs`.
