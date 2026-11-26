# shell-gate: a gate on the shell tool's commands

An extension whose one hook stops two shell commands that cannot be taken back before they run: `rm` with `-r` and `-f`, and a forced `git push`. Every other command goes on as it would without the extension.

## What it shows

- **A `tool.before` hook,** registered with `cineris.on("tool.before", { tools: ["shell"] }, …)`. It is asked about each call of the shell tool after your permission level and rules have decided it and before it runs or you are asked about it, and is given the call's arguments as the model wrote them.
- **A gate that only narrows.** It answers `deny` with a reason, which the model is shown in place of the command's output, or nothing at all. A hook can refuse a call, or have you asked about one your level would allow, but nothing it answers lets through a call that your level or your rules would ask about or refuse.
- **What it refuses:**
  - `rm` given both `-r` (or `-R`) and `-f`, in any form: `rm -rf`, `rm -fr`, `rm -r -f`, `rm --recursive --force`, and after `sudo`, `xargs`, `&&` or `;`.
  - `git push --force`, `git push -f` (also among other short options, as in `-uf`), and a push of a refspec that begins with `+` (`git push origin +main`), which forces that branch.
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

Cineris reads the folder, shows what the extension may do and the hook it registers, and asks you to approve it. A yes copies the folder into your configuration directory's `extensions/shell-gate/` (`~/.config/cineris/` on Linux, `%APPDATA%\cineris\` on Windows) and pins it by its hash. Where there is no terminal to ask at, run it once to see the hash, then again with `--hash <the hash shown>`.

Once approved, its gate holds: if the folder changes or cannot be read, the shell tool's calls are refused, saying why, until you approve it again (`cineris ext add`) or turn it off (`cineris ext disable shell-gate`).

## What it does not do

It is a guard against a slip, not a sandbox. It reads the words of a command line as a POSIX shell splits them, so a command that only looks like one of these is refused too (`echo rm -rf`, `git rm -rf`): reword it, or run it yourself. And a command that does the same thing in other words is not refused: a script that runs `rm -rf`, a git alias, a variable that holds `--force`, or PowerShell's `Remove-Item -Recurse -Force`. What a command may do at all is set by your permission level, your rules and the sandbox.

This example is run as a test on Linux and Windows: `crates/cineris-cli/tests/system/examples.rs`.
