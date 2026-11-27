# shell-gate: a gate on the shell tool's commands

An extension whose one hook stops two shell commands that cannot be taken back before they run: `rm` with `-r` and `-f`, and a forced `git push`, in the forms listed below. Every other command goes on as it would without the extension.

**It is an illustration of a hook, not a safety boundary.** It reads the text of a command, and a command can always be written in words it does not read. What protects you is the permission engine, your permission level and your rules, which decide what a command may do whatever its words, and the sandbox, which confines what a command can reach. The gate adds a refusal to theirs, and takes none away.

## What it shows

- **A `tool.before` hook,** registered with `cineris.on("tool.before", { tools: ["shell"] }, …)`. It is asked about each call of the shell tool after your permission level and rules have decided it and before it runs or you are asked about it, and is given the call's arguments as the model wrote them.
- **A gate that only narrows.** It answers `deny` with a reason, which the model is shown in place of the command's output, or nothing at all. A hook can refuse a call, or have you asked about one your level would allow, but nothing it answers lets through a call that your level or your rules would ask about or refuse.
- **What it refuses, in these forms:**
  - `rm` given a recursive option and a forced one. Each is a short option, alone or among others (`rm -rf`, `rm -fr`, `rm -Rf`, `rm -r -f`, `rm -rvf`), or a long one, whole or shortened as GNU `rm` accepts it (`--recursive` or `--rec`, `--force` or `--f`). Case is not looked at, so PowerShell's `rm -Recurse -Force` and `rm -r -Force` are refused too: there `rm` is `Remove-Item`.
  - `git push` given `--force`, `--mirror` (which overwrites every branch of the remote), `-f` (alone or among other short options, as in `-uf`), or a refspec that begins with `+` (`git push origin +main`), which forces that branch; and a push whose `-c` sets a `push` refspec that begins with `+` (`git -c remote.origin.push=+main push origin`). git's own options before `push` are passed over, and the value after each of those that take one (`-C`, `-c`, `--git-dir`, `--work-tree`, `--namespace`, `--config-env`, `--attr-source`).
  - Each of these wherever it is in a command (after `sudo`, `xargs` or `env`), in each command of a line (after `;`, `&&`, `||`, `|`, `&` or a line break, and inside `$( )`, backticks or parentheses), and in the line given to `sh -c` or `bash -c` (or `-lc`, and other short options with `c` among them) and the words given to `eval`, each read again as a line.
- **What it lets through:** everything else, `git push --force-with-lease` included. A push with a lease refuses to overwrite the remote branch when it holds commits you have not fetched, which is the harm a forced push does. (A fetch made in the background can move what the lease compares with; add `--force-if-includes` to close that.)

## What its manifest asks for, and why

| Capability | Value | Why |
|---|---|---|
| `hooks` | `["tool.before"]` | The one event it hooks. The approval says what the event allows: seeing every tool call's arguments before it runs, and refusing or changing the call. Its script puts its hook on the shell tool's calls alone, and only refuses. |

Nothing else: it runs no program, reads or writes no file and reaches no network. It decides from the command's text alone.

## Install it

```
cineris ext add examples/shell-gate
```

Cineris reads the folder, shows what the extension may do and the hook it registers, and asks you to approve it. A yes copies the folder into your configuration directory's `extensions/shell-gate/` (`~/.config/cineris/` on Linux, `%APPDATA%\cineris\` on Windows) and pins it by its hash. The yes is your approvals password, typed at the terminal; with no terminal nothing is approved.

Once approved, its gate holds: if the folder changes or cannot be read, the shell tool's calls are refused, saying why, until you approve it again (`cineris ext add`) or turn it off (`cineris ext disable shell-gate`).

## What it does not do

It refuses the forms above and nothing more. It reads the words of a command line as a POSIX shell splits them, and stops there: it does not see what a shell does to the words before it runs them, nor what a program does once it runs. So it lets through, among others:

- a brace expansion (`rm -{r,f} build`), and a line continued with a backslash in the middle of a word;
- a variable, or a command's output, that holds an option or a whole command (`rm $flags build`, `git push $(echo --force)`);
- a script, a function, an alias or a git alias that runs one of these commands;
- another shell's `-c` (`zsh -c`, `pwsh -Command`), and a shell's option that comes between `-c` and its line;
- PowerShell's `Remove-Item` by its own name or another of its aliases (`del`, `erase`, `ri`, `rd`, `rmdir`);
- a forced push or a mirror set in the repository's own configuration, or by an environment variable, and a `git push` option shortened (`--mirr`, which git takes as `--mirror`);
- anything else beyond the reading above.

And a command that only looks like one of these is refused too (`echo rm -rf`, `git rm -rf`): reword it, or run it yourself.

What a command may do at all is set by your permission level, your rules and the sandbox, and they are what to rely on.

This example is run as a test on Linux and Windows: `crates/cineris-cli/tests/system/examples.rs`. Its hook is given each form above, and those it lets through, in `crates/cineris-ext/tests/it/examples.rs`.
