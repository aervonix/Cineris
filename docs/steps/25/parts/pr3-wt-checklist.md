# Step 25, its third pull request: the Windows Terminal checklist, with the real interface

The checklist step 20's prototypes went through ([measurements/windows-terminal-checklist.md](../../../measurements/windows-terminal-checklist.md)), run again on the real interface: the `dist` build of `main` once step 25's third pull request has merged, in a fresh Windows Terminal window of 120 by 40 on the desktop's interactive Windows session, against the test kit's scripted provider on the loopback, so that no paid call is made. The design is [answered.md](../answered.md), "The design, before building", point 8; the first run and `auth login` it checks are [pr3-first-run.md](pr3-first-run.md).

The owner chose the scripted route: the coordinator drives every row marked *script* from a one-off scheduled task with `/IT` in that session ([conpty.md](../../../conpty.md)), following the steps in part 5, and a person does only the rows marked *person*. The row K1, the open check `key-latency-over-conpty`, is done in the same sitting. It has not been run yet: [the-windows-terminal-check-of-step-25](../../../outstanding/the-windows-terminal-check-of-step-25.md) stays open until it has.

**Time:** the build, 10 to 20 minutes; the script's rows, about 15; the person's, about 15.

## 1. What needs a person's eyes or hands

Five things no script can judge. The script has taken the screenshots each needs before the person starts.

1. **The feel of typing,** in both presenters, idle and while an answer streams: each key shows at once, none lost or doubled. Rows I5 and F5 (the script checks what arrived; the feel is the person's).
2. **The wheel and `Shift`+drag selection,** in the full screen and the viewer. Under Windows Terminal the program asks for no mouse ([the-mouse-under-windows-terminal](../../../outstanding/the-mouse-under-windows-terminal.md)), so the terminal keeps its own selection, and its wheel on the alternate screen is likely to be alternate scroll's `Up` and `Down`: in the full screen they would reach the input's history, in the viewer scroll it. Whether that feels right. Rows A1, A2 and A3.
3. **The glyphs in your own font:** `◆ ◇ ╭ ╮ ╰ ╯ ─ │ · …` drawn as themselves, not as boxes; the line of wide text (Greek, Cyrillic, Japanese, Korean, an emoji) in place, and the right edges of the boxes in line. Rows I8 and F8; C6 for the old console host's ASCII set.
4. **The colours on your own scheme:** the answer at full brightness, the dim parts readable, the accent and the warnings told apart. Rows I9 and F9; C6.
5. **A look through the flicker screenshots:** 30 frames 60 ms apart for each presenter, none with the live area blank or half drawn. Rows I2, F2 and C2. The script checks that each frame has its bottom rows drawn; it cannot tell a frame that is full but wrong.

Optional, with your own files, never done by the script: the first run's imports step with your own `~/.claude.json` (`Space` on its row): each server's command shown whole; a key of a known format in an argument shown `[redacted]`, and its server not offered; a value under a credential's name (`--api-key …`, `GITHUB_TOKEN=…`) shown `[redacted]`, its row saying it is imported as it is, unseen, when the value is one token, and its server not offered, the row saying to move the value into a variable, when it is not; a value under a name ending in `command`, `cmd`, `hook`, `script`, `file`, `path`, `url`, `uri`, `endpoint`, `dir` or `directory` shown, an address there with its host and path visible and its user part and any `?api_key=…` masked; a path under a key's name shown when it is relative (`--key ./client.key`) or names a file that exists, and masked when it names nothing. Row R5.

Write down for all of them: the Windows Terminal version (Settings, About), the Windows build (`winver`), the font and its size, and the colour scheme.

## 2. Building and copying

**What it needs:** the desktop, logged on and unlocked; Windows Terminal; PowerShell 7 (`pwsh`); Rust for Windows (`rustup` with the MSVC toolchain: the repository's `rust-toolchain.toml` picks the version) and Git; a clone of the repository on Windows' own disk, not under `\\wsl$`. Below it is `$HOME\src\cineris`. No key of a real provider is needed or read.

In Windows Terminal, a tab of PowerShell 7 (tab A), in the repository's folder:

```powershell
cd $HOME\src\cineris
git switch main
git pull --ff-only
cargo build --locked --profile dist -p cineris-cli --bin cineris
```

`dist` is the shipped binary's profile (`[profile.dist]` in `Cargo.toml`: fat LTO, one codegen unit, stripped), so the build takes several minutes. The binary lands at `target\dist\cineris.exe`. `cargo xtask startup --profile dist` builds the same binary, with the build machine's folders taken out of its paths, and then measures its start-up; either will do here.

Copy it, under both its names, into a folder of the check's own, which the check window puts first on `PATH`, so that a later build leaves the copy alone:

```powershell
$check = "$HOME\cineris-check"
New-Item -ItemType Directory -Force "$check\bin", "$check\work", "$check\first-run", "$check\shots", "$check\appdata\Roaming", "$check\appdata\Local" | Out-Null
Copy-Item target\dist\cineris.exe "$check\bin\cineris.exe"
Copy-Item target\dist\cineris.exe "$check\bin\cin.exe"
& "$check\bin\cineris.exe" --version
& "$check\bin\cin.exe" --version
```

`cin` is the alias, the same program under a second name (`cargo xtask alias` makes it a hard link beside a debug or release build; a copy is the same). Both print the same line, `cineris` and the version. Outside the check window, run either by its path: `& "$check\bin\cineris.exe"`.

Everything Cineris writes in the check goes under `$HOME\cineris-check`: its settings, sessions, log and history (the check window points `APPDATA` and `LOCALAPPDATA` there). The one exception is the key the first run and `auth login` store, which goes to Credential Manager as `cineris:scripted` and is removed by the rows that stored it.

## 3. The scripted provider, the files and the window

### The scripted provider

In a second tab of the same window (tab B: `Ctrl+Shift+T`, PowerShell 7), in the repository's folder:

```powershell
cd $HOME\src\cineris
cargo run --locked -p cineris-testkit --example scripted-provider --features scripted-provider -- --log "$HOME\cineris-check\provider.log"
```

The first build takes a minute or two. Then it prints, and keeps the same lines in `provider.log`:

```text
09:41:58.204Z  base_url = "http://127.0.0.1:41917"
09:41:58.204Z  ready: answering on http://127.0.0.1:41917, 50 words a second, 18 paragraphs a long answer; Ctrl+C ends it
```

Leave it running for the whole check; `Ctrl+C` in tab B ends it at the end. It is the test kit's scripted SSE server, as the system tests use it, answering in Anthropic's Messages protocol on the loopback only: it checks and prints no key, and sends nothing anywhere. Its source is `crates/cineris-testkit/examples/scripted-provider.rs`.

- **What it answers.** A prompt gets the long answer: a first line, `This is the answer to request N: …`; `Paragraph 1 of 18.` to `Paragraph 18 of 18.`; after the ninth a list and a block of Rust with one line wider than any window; a line of wide text, `café, naïve, Ελληνικά, Русский, 日本語の文, 한국어, and a seedling 🌱`; the last line, `The answer to request N ends here, and the listing follows.`; then a call to `list` of the workspace, which asks nothing. The call's result gets three short paragraphs, `This is the answer to request N+1, after the call.`, `Closing 1 of 3.` to `Closing 3 of 3. The turn of request N+1 is complete.`, which end the turn. A prompt with the word `dialog` in it gets, after the listing, a `write` to `.cineris/checklist-note.md`, which asks first, and the closing paragraphs after that call's result. The long answer streams for about 29 seconds, two words at a time.
- **Its log** has a line for each request and one when each answer ends:

  ```text
  09:42:10.031Z  request 1: a prompt in 2 text blocks (1 line, 33 characters; 1 line, 24 characters): "tell me about the garden" -> the long answer (1438 words, about 29 s), then a call to `list`
  09:42:39.140Z  request 1: answered whole in 28.8 s
  09:42:39.160Z  request 2: the result of the call toolu_scripted_1 -> the closing paragraphs (60 words, about 2 s), which end the turn
  ```

  The prompt's own text is the last block, and its first 40 characters are quoted; the block before it is what Cineris adds. An answer the client stopped reading ends `cut off by the client`. Request numbers count from the provider's start, across every run of Cineris.
- **One request early.** The server takes its reply from a queue as a request arrives, so the reply to the next request is chosen when this one arrives. A turn stopped part-way (`Esc`, `Ctrl+C`, quitting) leaves the next prompt the closing paragraphs once, and the log says `[out of step: …]`: send the prompt again.
- **Another port.** `--port N` if 41917 is taken (Windows keeps ranges for Hyper-V and WSL: `netsh interface ipv4 show excludedportrange protocol=tcp`), and the same port in `config.toml`, `fkeys.toml` and `$firstRun`. `--rate WORDS` changes the speed.

### The check's files

In tab A, paste this block once. It writes the configuration, the configuration for the function keys, the helper that reads the console's modes, and the set-ups of the check window (PowerShell 7) and of the old console host (`cmd`):

```powershell
$check = "$HOME\cineris-check"
Set-Content -Encoding utf8NoBOM "$check\config.toml" @'
# Cineris pointed at the scripted provider of step 25's Windows Terminal check
# (docs/steps/25/parts/pr3-wt-checklist.md). Nothing here reaches a real provider.

# A model of the built-in catalog. Its provider, the built-in `anthropic`, is sent
# to the loopback, with its key from SCRIPTED_MODEL_KEY, which any text satisfies.
model.main = "claude-haiku-5-5"

[providers.anthropic]
base_url = "http://127.0.0.1:41917"
key_env = "SCRIPTED_MODEL_KEY"

# A provider of the check's own, for the rows that store a key: what they store
# goes to Credential Manager as cineris:scripted, never over a real provider's.
[providers.scripted]
protocol = "anthropic-messages"
base_url = "http://127.0.0.1:41917"
'@
Set-Content -Encoding utf8NoBOM "$check\fkeys.toml" ((Get-Content -Raw "$check\config.toml") + @'

# Row A5: every function key ends a line in the prompt, as the system test
# sys_function_keys_and_the_arrows_reach_the_application binds them.
[keys.editor]
new_line = ["f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12"]
'@)
Set-Content -Encoding utf8NoBOM "$check\modes.ps1" @'
# Prints the console's input and output modes, read from the console itself
# (CONIN$ and CONOUT$), whatever standard input and output are.
if (-not ('CinerisCheck.Console' -as [type])) {
    Add-Type -Namespace CinerisCheck -Name Console -MemberDefinition '
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        public static extern IntPtr CreateFile(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll")]
        public static extern bool GetConsoleMode(IntPtr handle, out uint mode);
        [DllImport("kernel32.dll")]
        public static extern bool CloseHandle(IntPtr handle);
    '
}
function Get-Mode([string] $name) {
    # Read and write, shared for both, an existing file.
    $handle = [CinerisCheck.Console]::CreateFile($name, 3221225472, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)
    $mode = [uint32]0
    [void][CinerisCheck.Console]::GetConsoleMode($handle, [ref]$mode)
    [void][CinerisCheck.Console]::CloseHandle($handle)
    $mode
}
'input 0x{0:X4} output 0x{1:X4}' -f (Get-Mode 'CONIN$'), (Get-Mode 'CONOUT$')
'@
Set-Content -Encoding utf8NoBOM "$check\tab.ps1" @'
# The check window's set-up, dot-sourced as it starts: Cineris reads and writes
# only the check's own folders, and reads no real provider's key.
$check = "$HOME\cineris-check"
$env:Path = "$check\bin;$env:Path"
$env:CINERIS_CONFIG = "$check\config.toml"
$env:SCRIPTED_MODEL_KEY = 'scripted-any-key'
$env:APPDATA = "$check\appdata\Roaming"
$env:LOCALAPPDATA = "$check\appdata\Local"
Set-Location "$check\work"
# The first run's settings, on the command line: its own file does not exist yet.
# The first run writes no model for the provider `scripted`, whose models the
# catalog does not know, so the session runs model.main from here: a model of the
# built-in `anthropic`, sent to the loopback.
$firstRun = @(
    '-c', 'providers.anthropic.base_url=http://127.0.0.1:41917',
    '-c', 'providers.anthropic.key_env=SCRIPTED_MODEL_KEY',
    '-c', 'providers.scripted.protocol=anthropic-messages',
    '-c', 'providers.scripted.base_url=http://127.0.0.1:41917',
    '-c', 'model.main=claude-haiku-5-5'
)
# Runs cineris with the arguments given, then says whether the console's modes
# are the ones it found, as the system tests' shell does.
function Invoke-Check {
    $before = & "$check\modes.ps1"
    & cineris @args
    $code = $LASTEXITCODE
    $after = & "$check\modes.ps1"
    if ($before -eq $after) { "check: exit $code, modes RESTORED ($after)" }
    else { "check: exit $code, modes NOT RESTORED (before $before, after $after)" }
}
'@
Set-Content -Encoding utf8NoBOM "$check\tab.cmd" @'
@echo off
rem The old console host's set-up for step 25's Windows Terminal check: the same
rem folders and settings as tab.ps1. `check ARGS` runs cineris as Invoke-Check does.
set "CHECK=%USERPROFILE%\cineris-check"
set "PATH=%CHECK%\bin;%PATH%"
set "CINERIS_CONFIG=%CHECK%\config.toml"
set "SCRIPTED_MODEL_KEY=scripted-any-key"
set "APPDATA=%CHECK%\appdata\Roaming"
set "LOCALAPPDATA=%CHECK%\appdata\Local"
cd /d "%CHECK%\work"
'@
Set-Content -Encoding utf8NoBOM "$check\bin\check.cmd" @'
@echo off
rem Runs cineris with the arguments given, then says whether the console's modes
rem are the ones it found. Quit cineris with Ctrl+D, not Ctrl+C, or cmd asks
rem whether to end this batch file.
for /f "delims=" %%m in ('pwsh -NoLogo -NoProfile -File "%CHECK%\modes.ps1"') do set "BEFORE=%%m"
cineris %*
set "CODE=%ERRORLEVEL%"
for /f "delims=" %%m in ('pwsh -NoLogo -NoProfile -File "%CHECK%\modes.ps1"') do set "AFTER=%%m"
if "%BEFORE%"=="%AFTER%" (echo check: exit %CODE%, modes RESTORED ^(%AFTER%^)) else (echo check: exit %CODE%, modes NOT RESTORED ^(before %BEFORE%, after %AFTER%^))
'@
```

- **`config.toml`** is the configuration for every row but A5 and the first run. `key_env` replaces the built-in provider's own variable, `ANTHROPIC_API_KEY`: the variable named is set, and it is looked at before the credential store, so neither a real key in the environment nor one stored as `cineris:anthropic` is read. `CINERIS_CONFIG` points the binary at this file, so your own configuration is untouched.
- **`fkeys.toml`** binds F1 to F12 to the prompt's new line for row A5. The list replaces the action's defaults, so under this file `Shift+Enter` makes no new line.
- **The first run** (rows R1 to R4) has no file: `CINERIS_CONFIG` names one that does not exist, and the settings come on the command line (`$firstRun`), as the first run's system test gives them. It chooses the provider `scripted` (R2), whose models the catalog does not know, so the file it writes names no model, nor a provider for the main role, and the session runs the command line's `model.main`, `claude-haiku-5-5`, as the built-in `anthropic`'s, at the loopback. Until the review of the first run's fixes it wrote `model.roles.main.provider = "scripted"`, and that model would not have been found.

### The check window

From tab A, a fresh window of 120 by 40, with the check's set-up:

```powershell
wt.exe -w new --size 120,40 --title cineris-check --suppressApplicationTitle pwsh -NoLogo -NoExit -Command '. (Join-Path $env:USERPROFILE cineris-check\tab.ps1)'
```

The rows run in that window, in its folder `$HOME\cineris-check\work`, with `Invoke-Check` in place of `cineris`. The title stays `cineris-check`, which is how the script finds the window. The old console host's window is started in row C1.

## 4. The rows

*Script*: the coordinator's script runs the row by the steps in part 5 and records the result; *person*: the person does it; *script, person*: the script runs it and the person judges what the shots show. Every row of a presenter runs in the check window at 120 by 40 with `config.toml` unless it says otherwise.

### The inline presenter (run 1)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| I1 | **It starts and draws.** `Invoke-Check` | The header card (`Cineris` and the version), the input box (`Describe a task`) and the hint row, within 2 s of `Enter` | script | |
| I2 | **No flicker while streaming.** A prompt; 30 screenshots 60 ms apart | Every frame has the input box and the hint row drawn; none blank or half drawn | script, person | |
| I3 | **Text lands in the scrollback.** After the turn, the whole buffer | `Paragraph 1 of 18.` to `Paragraph 18 of 18.` once each, in order; the code block; the call's box; `Closing 1 of 3.` to `Closing 3 of 3.` once each; no input box or hint row left in the scrollback | script | |
| I4 | **The window does not jump.** While it streams | The view stays at the bottom: the running turn's hint row (`esc stops`) is on the screen at every look, twice a second | script | |
| I5 | **Typing while streaming.** 30 characters at a person's pace | The input box shows `the quick brown fox jumps over`, nothing lost or doubled; the feel is the person's | script, person | |
| I6 | **Resize while streaming.** 60 columns, then 120 again | One input box and one hint row at the bottom; this turn's paragraphs once each; no row of an old frame left behind | script | |
| I7 | **Quitting restores the terminal.** `Ctrl+D` | `check: exit 0, modes RESTORED`; the prompt back, the cursor shown, typing echoed (`echo after-inline`) | script | |
| I8 | **Glyphs** in your font, from shots I1, I5 and I6 | As in part 1, item 3 | person | |
| I9 | **Colours** on your scheme, from the same shots | As in part 1, item 4 | person | |

### The full screen (run 2)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| F1 | **It starts and draws.** `Invoke-Check --fullscreen` | The alternate screen: the header at the top, the input box at the bottom, none of the shell's lines | script | |
| F2 | **No flicker while streaming.** As I2 | As I2 | script, person | |
| F3 | **Text scrolls.** `PageUp` after the turn, then `PageDown` | `more below` shown and earlier paragraphs on the screen; back at the end, `more below` gone | script | |
| F4 | **The window does not jump.** `PageUp` while it streams | The view stays where it was put while text arrives; after `PageDown` to the end it follows the stream again | script | |
| F5 | **Typing while streaming.** As I5 | As I5 | script, person | |
| F6 | **Resize while streaming, and re-wrapping.** 60 columns, then 120 | At 60 no row wider than 59 and the paragraphs wrapped to it; at 120 wrapped wider again; the input box spans the window each time | script | |
| F7 | **Quitting restores the terminal.** `Ctrl+D` | `check: exit 0, modes RESTORED`; the main screen as the shell left it, with nothing of the full screen's turns; `echo after-full` echoed; a plain drag selects (no mouse mode left on) | script | |
| F8 | **Glyphs,** from shots F1, F5 and A10 | As in part 1, item 3 | person | |
| F9 | **Colours,** from the same shots | As in part 1, item 4 | person | |

### The added rows

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| A1 | **The wheel in the full screen** (run 2). Three notches up, three down | Recorded: what moved. Likely alternate scroll's `Up` and `Down`: the input shows an earlier prompt and the transcript stays. A fail is a mouse report typed into the input (`[<64;…M`) | script, person | |
| A2 | **The wheel in the viewer** (run 2). `Ctrl+O`, three notches down, three up | Recorded: whether the viewer scrolled (its `up/down scrolls`); nothing typed | script, person | |
| A3 | **`Shift`+drag selects** (run 2), across a row of the transcript, then `Ctrl+Shift+C` | The row highlighted, and its text on the clipboard | script, person | |
| A4 | **After quitting, a plain drag selects in the shell,** with QuickEdit off in the console's defaults (run 5) | `after-run` dragged over is on the clipboard, as `before-run` was before Cineris ran | script | |
| A5 | **F1 to F12 and the arrows** (run 4, `fkeys.toml`). Each function key between two words, then the arrows | Each of F1 to F10 and F12 ends its line; the arrows give `b12U` and `pMqND`. F11 is Windows Terminal's own full-screen key unless that binding is removed: recorded either way. **Once this row passes, the file `docs/outstanding/function-keys-through-conpty.md` is deleted** (with the system test of the second pull request, the design's close) | script | |
| A6 | **A 200-line paste with `Ctrl+V`** (run 3) | `[pasted 200 lines]` in the input; nothing sent until `Enter`; then one request, its prompt 200 lines | script | |
| A7 | **The same paste with a right-click** (run 3) | As A6 | script | |
| A8 | **`/copy`** (run 2), after a turn | The clipboard holds the last answer exactly as the model wrote it: the closing paragraphs, unindented, unwrapped | script | |
| A9 | **Search** (run 2). `Ctrl+F`, `seedling`, `Up`, `Esc` | The search row with `of 2` (one match in each long answer), the match reversed; `Up` goes to the older; `Esc` gives the hint row back | script | |
| A10 | **Reading the transcript under a dialog** (run 2). A prompt with `dialog`; at the dialog `Tab`, `PageUp`, `y`, `Tab`, `n` | `tab reads above` offered; reading, `tab goes back` and the transcript scrolls; `y` does nothing while reading; back at the dialog its answers return; `n` denies, nothing is written | script | |

### The first run in an empty profile (run 6)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| R1 | **The first step.** `Invoke-Check @firstRun`, `CINERIS_CONFIG` naming a file that does not exist | The set-up card and `Provider  1 of 4`; its keys (`enter chooses · s skips this step · x skips the set-up`) a moment after the step | script | |
| R2 | **A key typed and a key pasted,** for the provider `scripted` | `36 characters, hidden` typed; `0 characters` after `Ctrl+U`; `35 characters, hidden` pasted with `Ctrl+V` and with a right-click; nothing of either key on the screen | script | |
| R3 | **Each step after it.** `Enter` at the imports and at the mode | Each step's keys a moment after it; the mode offers `read-only` and `supervised`, never `autonomous` (no sandbox enforces on Windows yet); `Set-up done.`; a prompt then answered | script | |
| R4 | **What is left.** Credential Manager, the files, the screen | `cmdkey` lists `cineris:scripted`; the first run's file holds the choices and no `[model]` table (no model of `scripted` is known, which the last line says); no file under the check's folder, and nothing in the buffer, holds eight characters of either key; the modes restored | script | |
| R5 | **Optional, your own files:** the imports step with your `~/.claude.json` | Each command whole; a key of a known format `[redacted]`, its server not offered; a token under a credential's name `[redacted]`, the row saying it is imported unseen; any other value under such a name `[redacted]`, its server not offered, the row saying to move it into a variable; a value under a name ending in `command`, `file`, `path`, `url` or `dir` shown, an address's user part and query key masked; a path under a key's name shown when relative or when it exists | person | |

### `cineris auth login` (runs 7 and 8)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| L1 | **In Windows Terminal.** `auth login scripted`: type, `Ctrl+U`, type, `Esc`; then again, `Enter` | The row counts characters and shows none; `Ctrl+U` empties it; `Esc` says `no key was stored`, exit 2, nothing stored; `Enter` says `Stored the key of scripted.`, exit 0, `cmdkey` lists it; the modes restored each time | script | |
| L2 | **In the old console host,** the same with `check` | As L1 | script | |

### The old console host, inline only (run 8)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| C1 | **It starts and draws.** `check` in `cmd` under `conhost.exe` | The header card and the input box, in the ASCII set (`+`, `-`, `\|`, `>`), no escape sequence printed as text | script | |
| C2 | **No flicker while streaming.** As I2 | As I2 | script, person | |
| C3 | **Typing while streaming.** As I5 | As I5 | script | |
| C4 | **Resize while streaming.** As I6 | As I6 | script | |
| C5 | **Quitting restores the terminal.** `Ctrl+D` | `check: exit 0, modes RESTORED`; `echo after-conhost` echoed | script | |
| C6 | **Glyphs and colours** of the ASCII set and 16 colours, from shots C1 and C3 | Readable, boxes in line | person | |

### Key latency (run 9)

| # | Action | Expected | Who | pass / fail / note |
|---|---|---|---|---|
| K1 | **`key-latency-over-conpty`.** `cargo xtask perf` in tab A | The two tables, recorded with the Windows Terminal version and the Windows build | script | |

## 5. The script's steps

### How a script does each kind of step

| Step | How |
|---|---|
| The session | A one-off scheduled task in the logged-on user's interactive session: `schtasks /Create /TN cineris-check /SC ONCE /ST 23:59 /IT /TR "pwsh -NoLogo -File <the script>"`, then `schtasks /Run /TN cineris-check`. The desktop stays unlocked, and nothing else takes the focus while it runs: keys go to the window in front. The script itself, which holds the keys of R2 and L1, is kept outside `$HOME\cineris-check`. |
| The window | Started as part 3 says; found by its title (`EnumWindows`, `GetWindowText`) and brought to the front (`SetForegroundWindow`) before each step. The old console host: `Start-Process conhost.exe 'cmd.exe /k "%USERPROFILE%\cineris-check\tab.cmd"'`, found by its process. |
| `type`, `key` | `SendInput` from PowerShell (`Add-Type` over `user32.dll`): `KEYEVENTF_UNICODE` with each character for text; virtual keys for named keys, with the modifier held down around the key. Windows Terminal turns them into win32 input records, as it does a keyboard's. |
| `wheel`, `drag`, `right-click` | `SendInput` mouse input at a cell's pixel (the client rectangle divided into its columns and rows): `MOUSEEVENTF_WHEEL` (120 a notch); `LEFTDOWN`, `MOVE`, `LEFTUP`, with `VK_SHIFT` down around them where the step says `Shift`; `RIGHTDOWN`, `RIGHTUP`. |
| `screen has`, `buffer` | UI Automation (`System.Windows.Automation`): the terminal control's `TextPattern`, which a screen reader reads; `GetVisibleRanges()` for the screen, `DocumentRange` for the whole buffer with the scrollback. Polled every 100 ms. The old console host has the same pattern. |
| `shot`, `burst` | `System.Drawing.Graphics.CopyFromScreen` over `GetWindowRect`, a PNG in `$HOME\cineris-check\shots`; a burst times its 30 frames with a `Stopwatch`, 60 ms apart. The pixel check of I2, F2 and C2: in each frame, the band from one to four rows above the bottom holds pixels of more than the background's colour. |
| `resize` | `SetWindowPos`, the pixel size for the cells asked: the cell's size measured at 120 by 40, plus the window's frame. |
| `clip` | `Set-Clipboard`, `Get-Clipboard`. |
| `log has` | The lines of `$HOME\cineris-check\provider.log` written since the step began. |
| The console's modes | `Invoke-Check` (`check` in `cmd`) prints them before and after: `check: exit 0, modes RESTORED (…)`, read with `screen has`. |
| Timing | A `Stopwatch` from the `SendInput` to the end of the wait. |

### The notation

- `run TEXT`: types `TEXT` at the shell's prompt, then `Enter`.
- `type TEXT`: types `TEXT` as keys, one every 120 ms, a person's pace; `type fast TEXT`, one every 20 ms.
- `key K`: one key: `Enter`, `Esc`, `Tab`, `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `F1` to `F12`, `Ctrl+` a letter, `Ctrl+Shift+C`.
- `screen has T (≤ S s)`: waits until the visible text holds `T`, and fails the row after `S` seconds, 10 when not given. `screen lacks T` checks at once that it does not.
- `buffer`: the window's whole text, the scrollback included.
- `log has T (≤ S s)`: waits until a line of the provider's log written since the step began holds `T`. `N` is the request number of the row's prompt, read from that line.
- `turn ends`: `screen has The turn of request N+1 is complete. (≤ 60 s)`, or `N+2` after the dialog of A10, then waits until `screen lacks esc stops` (≤ 5 s): the running turn's hint row is gone.
- `shot NAME`: a screenshot, `shots\NAME.png`; `burst NAME`: 30 of them, 60 ms apart.
- `resize C×R`, `wheel up K`, `wheel down K` (over the middle of the window), `drag R1:C1 to R2:C2` (cells counted from 1), `right-click` (over the middle), `wait S s`.
- `clip set T`, `clip`: the clipboard.
- `check`: what the row needs to pass. Any wait that runs out fails the row; the shots taken by then are kept, and the run goes on with the next.

### Run 1: the inline presenter (I1 to I7)

1. **I1.** `run Invoke-Check` · `screen has Describe a task (≤ 5 s)` · `shot I1`. Check: the screen holds `Cineris ` and the version `cineris --version` prints, the input box and the hint row, all within 2 s of the `Enter`.
2. **Turn A (I2, I4, I5).** `type tell me about the garden` · `key Enter` · `log has -> the long answer (≤ 5 s)` · `screen has Paragraph 2 of 18 (≤ 10 s)` · `burst I2` · from here until the closing paragraphs begin, every 500 ms: the screen holds `esc stops`, the running turn's hint row under the input box (I4) · `type the quick brown fox jumps over` · `shot I5` · `turn ends`. Check: every frame of the burst passes the pixel check (I2); the input box's text, inside its border and after its `◆`, is `the quick brown fox jumps over` exactly (I5); every look found the hint row (I4).
3. **I3.** `buffer`. Check: `Paragraph 1 of 18.` to `Paragraph 18 of 18.` once each, in order; `fn days_to_harvest`; `The answer to request N ends here`; `Closing 1 of 3.` to `Closing 3 of 3.` once each; `Type to steer` and `esc stops` nowhere (no input box or hint row of the turn left in the scrollback). Then `key Ctrl+C`, which empties the input · `screen has Describe a task`.
4. **Turn B (I6).** `type tell me about the garden` · `key Enter` · `log has -> the long answer` · `screen has Paragraph 4 of 18` · `resize 60×40` · `wait 2 s` · `shot I6-narrow` · `screen has Paragraph 9 of 18 (≤ 20 s)` · `resize 120×40` · `wait 2 s` · `shot I6-wide` · `turn ends`. Check: in the `buffer` after `This is the answer to request N:`, each of `Paragraph 1 of 18.` to `Paragraph 18 of 18.` once; `Type to steer` and `esc stops` nowhere; `Describe a task` once, in the input box on the screen; no row of the screen wider than the window.
5. **I7.** `key Ctrl+D` · `screen has check: exit 0, modes RESTORED (≤ 5 s)` · `run echo after-inline` · `screen has after-inline` twice (the line typed and its output) · `shot I7`. Check: the RESTORED line; the shot shows the cursor at the prompt.

### Run 2: the full screen (F1 to F7, A1 to A3, A8 to A10)

1. **F1.** `run Invoke-Check --fullscreen` · `screen has Describe a task (≤ 5 s)` · `shot F1`. Check: `screen lacks Invoke-Check` (the shell's lines are gone: the alternate screen); the header's `Cineris ` on the screen's first rows; the input box on its last.
2. **Turn A (F2, F4, F5).** `type tell me about the garden` · `key Enter` · `log has -> the long answer` (its `N` is A's) · `screen has Paragraph 2 of 18` · `burst F2` · `type the quick brown fox jumps over` · `shot F5` · `screen has Paragraph 10 of 18 (≤ 20 s)` · `key PageUp` · `screen has more below` · note the screen's first row · `wait 3 s` · `shot F4` · check: the first row is the same, and `more below` still shown · `key PageDown` until `screen lacks more below` (at most 10) · `wait 3 s` · check: the highest `Paragraph K of 18` on the screen grew · `turn ends`. Check: the burst (F2) and the input box's row (F5) as in run 1.
3. **F3.** `key Ctrl+C` · `key PageUp` three times · `screen has more below` · `shot F3`. Check: `screen lacks Closing 3 of 3.` and the screen holds a `Paragraph` line. `key PageDown` until `screen lacks more below` · `screen has The turn of request N+1 is complete.`
4. **Turn B (F6).** `type tell me about the garden` · `key Enter` · `log has -> the long answer` · `screen has Paragraph 4 of 18` · `resize 60×40` · `wait 2 s` · `shot F6-narrow` · check: no row of the screen longer than 59 characters, and the input box's border 59 or 60 wide · `screen has Paragraph 9 of 18 (≤ 20 s)` · `resize 120×40` · `wait 2 s` · `shot F6-wide` · check: rows of the transcript longer than 60 characters, and the input box's border as wide as the window · `turn ends`.
5. **A9.** `key Ctrl+F` · `type fast seedling` · `screen has ctrl+f seedling` · `screen has of 2` · `shot A9` (the match reversed) · `key Up` · check: the number before `of 2` changed · `key Esc` · `screen lacks ctrl+f`.
6. **A8.** `type fast /copy` · `key Enter`; if the input box still holds `/copy`, `key Enter` again · `wait 1 s`. Check: `clip`, its line ends made `\n`, is exactly

   ```text
   This is the answer to request M, after the call. Three short paragraphs end the turn.

   Closing 1 of 3. The listing came back, and nothing in it needs to change.

   Closing 2 of 3. The next prompt starts a new turn, with a long answer of its own.

   Closing 3 of 3. The turn of request M is complete.
   ```

   with `M` turn B's `N+1`.
7. **A1.** `wheel up 3` · `wait 1 s` · `shot A1-up` · record: does the screen hold `more below` (the transcript moved), does the input box hold an earlier prompt, `/copy` or `tell me about the garden` (alternate scroll's `Up` reached the history), or neither · `wheel down 3` · `wait 1 s` · `shot A1-down` · record the same. Check: the input box never holds `[<` or a run of digits and `;` (a mouse report typed). If the input box is not empty, `key Ctrl+C`.
8. **A2.** `key Ctrl+O` · `screen has esc returns` · note the screen's first row · `wheel down 3` · `wait 1 s` · `shot A2-down` · `wheel up 3` · `wait 1 s` · `shot A2-up` · record: did the first row change each time (the viewer scrolled) · `key Esc` · `screen has Describe a task`. Check: nothing typed into the input box.
9. **A3.** Find the row of the screen that holds `Closing 2 of 3.` · `drag` from its first column to its last, with `Shift` held · `shot A3` (the highlight) · `key Ctrl+Shift+C`. Check: `clip` holds `Closing 2 of 3.`
10. **A10.** `type show me the dialog` · `key Enter` · `log has -> the long answer` (its `N`) · `log has a short answer (≤ 60 s)` (request `N+1`, the `write` that asks) · `screen has Allow once (≤ 10 s)` · `screen has tab reads above` · `shot A10-dialog` · `key Tab` · `screen has tab goes back` · `key PageUp` · `screen has more below` · `shot A10-reading` · `key y` · `wait 1 s` · check: `screen has tab goes back` still, and no new line in the log · `key Tab` · `screen has Allow once (≤ 5 s)` (the answers come back after a second of quiet) · `key n` · `log has the result of the call toolu_scripted_N+1` · `turn ends` (request `N+2`). Check: `$HOME\cineris-check\work\.cineris\checklist-note.md` does not exist; if it does, the row fails and the file is removed.
11. **F7.** `key Ctrl+D` · `screen has check: exit 0, modes RESTORED (≤ 5 s)`. Check: the `buffer` holds `Invoke-Check --fullscreen` and none of `This is the answer to request R` for the full screen's requests (the main screen as the shell left it) · `run echo after-full` · `screen has after-full` twice · `drag` across the `after-full` output row, no key held · `key Ctrl+Shift+C` · check: `clip` holds `after-full` (no mouse mode left on) · `shot F7`.

### Run 3: pastes (A6, A7)

1. `run Invoke-Check` · `screen has Describe a task`.
2. **A6.** `clip set` 200 lines, `paste line 001 of 200` to `paste line 200 of 200` · `key Ctrl+V` · `screen has [pasted 200 lines] (≤ 3 s)` · `wait 2 s` · check: no new line in the log (nothing sent) · `shot A6` · `key Enter` · `log has a prompt in`. Check: that line's last block is `200 lines` and starts `"paste line 001 of 200` · `turn ends`. If Windows Terminal asks before pasting several lines, answer it to paste, and note it: that is its own setting.
3. **A7.** As A6, with `right-click` in place of `key Ctrl+V` (Windows Terminal pastes on a right-click when nothing is selected) · `shot A7`.
4. `key Ctrl+D` · `screen has check: exit 0, modes RESTORED`.

### Run 4: the function keys and the arrows (A5)

1. `run $env:CINERIS_CONFIG = "$check\fkeys.toml"` · `run Invoke-Check` · `screen has Describe a task`.
2. For each `n` of 1 to 12: `type fast a<n>` · `key F<n>` · `type fast b<n>` · `screen has` a row of the input box whose text, inside its border, ends `a<n>`, and the next row's starts `b<n>` (≤ 2 s). For F11 the wait may run out: Windows Terminal takes F11 for its own full screen by default. Record what happened, `key F11` again if the window went full screen, and go on: F11 passes only where Windows Terminal's binding was removed for the run (Settings, Actions).
3. `key F1` · `type fast pq` · `key Left` · `type fast M` · `key Right` · `type fast N` · `key Up` · `type fast U` · `key Down` · `type fast D` · `shot A5`. Check: inside the input box's border, a row's text ends `b12U`, and one row's text is `pMqND`.
4. `key Ctrl+C` · `screen has Describe a task` · `key Ctrl+D` · `screen has check: exit 0, modes RESTORED` · `run $env:CINERIS_CONFIG = "$check\config.toml"`.

When the row passes (F11 aside if Windows Terminal kept it, said in the note), `docs/outstanding/function-keys-through-conpty.md` is deleted. When a key fails, what arrived goes into that item instead.

### Run 5: a plain drag after quitting, QuickEdit off (A4)

1. In tab A: `run (Get-ItemProperty HKCU:\Console -Name QuickEdit -ErrorAction SilentlyContinue).QuickEdit` and keep the value (it may be absent) · `run Set-ItemProperty HKCU:\Console -Name QuickEdit -Value 0 -Type DWord`.
2. A new check window, as part 3 says: the console's defaults are read when a console starts.
3. `run echo before-run` · `drag` across its output row · `key Ctrl+Shift+C` · record whether `clip` holds `before-run`. If it does not, the shell itself has lost the selection with QuickEdit off: the row cannot judge Cineris; note it and go to step 6.
4. `run Invoke-Check` · `screen has Describe a task` · `key Ctrl+D` · `screen has check: exit 0, modes RESTORED`.
5. `run echo after-run` · `drag` across its output row · `key Ctrl+Shift+C`. Check: `clip` holds `after-run`.
6. Close the window. In tab A, put QuickEdit back: `Set-ItemProperty HKCU:\Console -Name QuickEdit -Value <the value kept> -Type DWord`, or `Remove-ItemProperty HKCU:\Console -Name QuickEdit` if it was absent.

### Run 6: the first run (R1 to R4)

The keys: typed, `sk-check-Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf` (36 characters); pasted, `sk-check-paste-Lm3Np7Qr1St5Uv9Wx2Yz` (35). They are the check's own and reach no provider.

1. **Before.** `run cmdkey /list:cineris:scripted` · check: no entry listed; one left by an earlier check is removed with `cmdkey /delete:cineris:scripted` · `run Remove-Item "$check\first-run\config.toml" -ErrorAction SilentlyContinue` · `run $env:CINERIS_CONFIG = "$check\first-run\config.toml"`.
2. **R1.** `run Invoke-Check @firstRun` · `screen has Provider  1 of 4 (≤ 5 s)` (two spaces), time T0 · `shot R1-step` · `screen has enter chooses`, time T1 · `shot R1-keys`. Record T1 − T0: a step takes its keys once it has been up 300 ms with a second of no input, so about a second after the `Enter` that started it.
3. **R2.** `key Down` until the row holding `scripted` and `store a key` begins, after its spaces, with `◆` · `key Enter` · `screen has Paste or type the key for scripted`, T0 · `screen has enter stores it`, T1 · record T1 − T0 · `type sk-check-Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf` · `screen has 36 characters, hidden` · `shot R2-typed` · `key Ctrl+U` · `screen has 0 characters, hidden` · `clip set sk-check-paste-Lm3Np7Qr1St5Uv9Wx2Yz` · `key Ctrl+V` · `screen has 35 characters, hidden` · `key Ctrl+U` · `screen has 0 characters, hidden` · `right-click` · `screen has 35 characters, hidden` · `shot R2-pasted`. Check, at each shot: the `buffer` holds no eight characters in a row of either key.
4. **R3.** `key Enter` (stores the pasted key) · `screen has Imports  2 of 4`, T0 · `screen has enter keeps what is picked`, T1 · `shot R3-imports` · `key Enter` · `screen has Mode  3 of 4`, T0 · `screen has enter chooses`, T1 · `shot R3-mode` · check: the screen holds `read-only` and `supervised`, and `screen lacks autonomous` · `key Enter` · `buffer` holds `Set-up done. (≤ 10 s)` · `screen has Describe a task` · `type is the set-up done?` · `key Enter` · `log has -> the long answer` · `turn ends`. Record each T1 − T0. No fourth step comes: the work folder has no project files.
5. **R4.** `key Ctrl+D` · `screen has check: exit 0, modes RESTORED` · `run cmdkey /list:cineris:scripted` · check: one entry, its target ending `cineris:scripted` · `run Get-Content "$check\first-run\config.toml"` · check: it holds `mode = "supervised"` and `instructions = false`, and no `[model` (the `buffer` holds `No model of scripted is known`) · check: `Get-ChildItem $check -Recurse -File | Select-String -SimpleMatch -Pattern <every piece of eight characters of each key>` finds nothing, and the `buffer` holds none.
6. **After.** `run $env:CINERIS_CONFIG = "$check\config.toml"` · `run cineris auth logout scripted` · `screen has Removed the stored key of scripted.`

### Run 7: `auth login` in Windows Terminal (L1)

1. `run Invoke-Check auth login scripted` · `screen has Paste or type the key for scripted` · `screen has enter stores it` · `type sk-check-Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf` · `screen has 36 characters, hidden` · `shot L1-row` · `key Ctrl+U` · `screen has 0 characters, hidden` · `type sk-check-Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf` · `key Esc` · `screen has no key was stored` · `screen has check: exit 2, modes RESTORED` · `run cmdkey /list:cineris:scripted` · check: no entry.
2. `run Invoke-Check auth login scripted` · `screen has enter stores it` · `type sk-check-Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf` · `key Enter` · `screen has Stored the key of scripted.` · `screen has check: exit 0, modes RESTORED` · `run cmdkey /list:cineris:scripted` · check: one entry · `run echo after-login` · `screen has after-login` twice · `shot L1-after` · check: the `buffer` holds none of the key.
3. `run cineris auth logout scripted` · `screen has Removed the stored key of scripted.`

### Run 8: the old console host (C1 to C5, L2)

1. `Start-Process conhost.exe 'cmd.exe /k "%USERPROFILE%\cineris-check\tab.cmd"'`, or by hand `conhost.exe cmd.exe /k "%USERPROFILE%\cineris-check\tab.cmd"` from Run (`Win+R`). Record its size; it keeps its scrollback.
2. **C1.** `run check` · `screen has Describe a task (≤ 5 s)` · `shot C1`. Check: `screen lacks ◆` and `screen lacks ╭`; no `[?` or `←[` on the screen (a sequence printed as text).
3. **C2, C3, C4.** `type tell me about the garden` · `key Enter` · `log has -> the long answer` · `screen has Paragraph 2 of 18` · `burst C2` · `type the quick brown fox jumps over` · `shot C3` · `resize 60×(its rows)` · `wait 2 s` · `shot C4-narrow` · `resize` back · `wait 2 s` · `shot C4-wide` · `turn ends`. Check: as I2, I5 and I6.
4. **C5.** `key Ctrl+C` · `key Ctrl+D` · `screen has check: exit 0, modes RESTORED` · `run echo after-conhost` · `screen has after-conhost` twice.
5. **L2.** As run 7, with `check auth login scripted` in place of `Invoke-Check auth login scripted`, then `cineris auth logout scripted`. Close the window with `exit`.

### Run 9: key latency (K1)

In tab A, in the repository's folder, with nothing else running but the provider, idle:

```powershell
cargo xtask perf 2>&1 | Tee-Object "$HOME\cineris-check\perf.txt"
```

It builds the terminal layer's demo and the editor demo in a release build, runs each in a pseudo-console, and prints three tables. Record from the first: `Key to frame, idle` and `Key to frame, streaming 2,000 tok/s`, each with the row under it, `of which the demo's (read to written)`, at the median, 95th and 99th; from the second: `idle`, `idle, the harness's share`, `streaming 2,000 tokens/s` and `streaming, the harness's share`, at the median and 99th. The pseudo-console is the one Windows itself has, which the test kit's driver opens, not the one Windows Terminal ships: the comparison the open item asks for, the runner's older ConPTY against a current Windows. Started from a Windows Terminal tab, as the item says.

## 6. Where the results go

- **Decision 0001,** [0001-tui-language.md](../../../decisions/0001-tui-language.md), "Checked on Windows": a paragraph for the real interface, step 25, with the date, the Windows Terminal version, the Windows build, the font and the scheme, and each row's outcome (the rows that passed in one line; each that failed, or was noted, in a sentence). The screenshots stay outside the repository, in `$HOME\cineris-check\shots`, which the owner keeps.
- **[the-mouse-under-windows-terminal](../../../outstanding/the-mouse-under-windows-terminal.md):** rows A1 to A4 update it. What the wheel did in the full screen and the viewer replaces "not measured"; the selection and the drag after quitting with QuickEdit off are added. It stays open for step 33.
- **[function-keys-through-conpty](../../../outstanding/function-keys-through-conpty.md):** row A5. Once it passes, the file is deleted, and the index written again (`cargo xtask outstanding`): the design closes the item with the system test of the second pull request (`sys_function_keys_and_the_arrows_reach_the_application`) and this row. A key that fails goes into the item with what arrived.
- **[key-latency-over-conpty](../../../outstanding/key-latency-over-conpty.md):** row K1's numbers are added to it. It stays open until the owner decides the budgets.
- **The first run and `auth login`** (rows R1 to R5, L1 and L2) answer the list "For a person at a real Windows Terminal" in [pr3-first-run.md](pr3-first-run.md). A failure is an open item of its own.
- **[the-windows-terminal-check-of-step-25](../../../outstanding/the-windows-terminal-check-of-step-25.md)** is deleted once the results are recorded.
