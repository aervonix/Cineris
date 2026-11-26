# Step 25 fixes: `auth login`, the commands step and the first run's mode

The review of step 25's findings 1, 17, 5 and 6. Each has a test that fails on `main` and a break the test catches ([`mutations/step-25-fixes-keys.breaks`](../../../../mutations/step-25-fixes-keys.breaks), put together into `step-25-fixes.breaks`).

## 1. `cineris auth login` echoed a key typed before its row was drawn

- **What was wrong:** at a terminal, `auth login` started a whole core, asked it whether the provider is known and asked the credential store where a key goes, and only then took the terminal. Until then the terminal was in its ordinary mode, echo on: a key pasted straight after `Enter` was echoed by the terminal itself, onto the screen and into the scrollback, and the key's row then took the same bytes, so nothing said it had been shown. Unix only: Windows' console echoes nothing outside a read in line mode.
- **The change:** `login_at_terminal` reads the configuration, takes the terminal (`cineris_tui::secret::KeyTerminal::enter`, raw mode, echo off, signals handed over) and only then starts the core, which is given the configuration read rather than reading it again (`CoreOptions::loaded`). What was typed meanwhile waits for the key's row, which takes it as typing. Anything said before the key is read (a provider the core does not know, a store that cannot keep a key, a core that does not start) is said after the terminal is put back, so no line is written in raw mode. What is left of the window is the program's own start and the read of the configuration files, a few milliseconds, against the core's start and the store's question.
- **Structure:** `Context::core` is now `Context::start_core`, which returns the error rather than saying it, under a `core` that says it as before; `known_provider` keeps its message in `unknown_provider`, which says nothing. `cineris_tui::secret::read_key` is `KeyTerminal::enter(…)?.read(…)`, so the first run and `auth login` read a key the same way.
- **The test:** `cineris-cli::system first_run::sys_auth_login_never_echoes_a_key_typed_before_its_row` (Unix). The binary is held just before its core starts by a new test hook, `cineris_base::hook::HOLD_CORE` (`CINERIS_TEST_HOLD_CORE`: the binary makes `held` in the folder it names and waits until the test makes `go`), read only with the `test-hooks` feature; `cargo xtask size` refuses a shipped binary that holds its name, as it refuses the other two hooks'. The test types the key while the core is held, reads everything the terminal sent back, and checks that no eight characters of the key are in it, then lets the core go and checks the key is stored and the terminal put back. On `main`'s code (the hook in place) it failed with the key at the shell's line: `test child_entry ... starting cineris` / `Qx7vR2Zt9kLw4Mp8Hd3Jc6Ny5Bf`.
- **Docs:** [08-configuration.md](../../../architecture/08-configuration.md), "Credentials".

## 17. `cineris auth login` at a terminal ignored `[terminal]`

- **What was wrong:** the key's input was given `Overrides::default()`, so `backspace = "ctrl+h"`, `keyboard = false` and the paste's settings did not apply while the key was typed, where a wrong Backspace matters most.
- **The change:** the configuration read before the terminal is taken gives the overrides, through the interface's own `interactive::overrides`.
- **The test:** `cineris-cli::system first_run::sys_auth_login_at_a_terminal_honours_the_terminal_settings`: with `terminal.keyboard = false`, against a terminal that answers that it has the enhanced keyboard protocol, nothing the binary writes pushes its flags (`CSI >`). On `main` it failed: the binary pushed `CSI > 3 u` after the reply.

## 5. The commands step hid npm's `pre<name>` and `post<name>`

- **What was wrong:** `npm run test` was offered beside `runs jest` though the file's `pretest` (`curl … | sh`, say) and `posttest` run with it; the person kept the row, and in a trusted project verification runs it after every turn that changed files.
- **The change, of the two the review offered:** show them together. `npm run <name>` is offered, when the file has `pre<name>` or `post<name>`, with all of them in the order npm runs them, each named and in double quotes, its `"` and `\` escaped as the import's arguments are (`pretest "eslint .", then "jest", then posttest "report"`). Each is held to the script's own rules (1,000 characters at most, no character that acts unseen), and one that cannot be read whole, or is not text, keeps the script from being offered. A script with neither is shown as before. Showing them, rather than leaving such scripts out, keeps the common `pretest: lint` projects' commands on offer.
- **The other detectors:** only `package.json`'s scripts carry text that is shown. `make`'s targets run their prerequisites, `cargo` its build scripts, `pytest` a `conftest.py`, and none is shown, as for a command typed by hand: those are the tools' own, well-known behaviour, not a second entry in the same file the row hides. No other package manager's file is read.
- **The test:** `cineris-knowledge detect::tests::a_script_is_offered_with_what_npm_runs_before_and_after_it`: a `pretest` and `posttest` around `test`, a `prebuild` alone, a `postlint` that is not text and a `precheck` with a tab. On `main` it failed: `test`'s row said `jest`, and `lint` and `check` were offered.
- **Docs:** [08-configuration.md](../../../architecture/08-configuration.md), "First run", the project's step; the module doc of `cineris-knowledge`'s `detect`.

## 6. The first run offered `autonomous` where the configuration turns the sandbox off

- **What was wrong:** the first run asked `Core::sandbox_status`, which said what the machine can do (`state(true)`), not what a session would get. With a managed `[sandbox] enabled = false` or `-c sandbox.enabled=false` on a Landlock machine, F3 offered `autonomous`, and picking it wrote a mode every session then refused with `sandbox_required`, until the file was edited by hand.
- **The change:** `Core::sandbox_status` now answers for the user's configuration in force (`state(user().settings.sandbox.enabled)`): its meaning changes, its signature does not. Its only caller is the first run. `initialize`'s `sandbox`, which the protocol defines as what the machine can do, is unchanged.
- **Public API:** `cineris_core::Core::sandbox_status`'s meaning, as above. `cineris_tui::app::setup::Providers::sandbox`'s doc says the same.
- **The test:** `cineris-core::it sandbox::the_first_runs_sandbox_status_is_the_configured_one`: a managed file turning the sandbox off on a Landlock 8 machine gives `Off`, and a session at `autonomous` is refused; the same machine with it on gives `Enforcing`. On `main` the first assertion failed (`left: Enforcing, right: Off`), as the review's test showed.
- **Docs:** [08-configuration.md](../../../architecture/08-configuration.md), "First run", the mode.

## Breaks moved

The rewrite moved code two older breaks name: `step-25/auth-login-refuses-a-terminal` now breaks the `auth` command's routing to the terminal (`&& io::stdout().is_terminal()`), and `step-28/pinned-trust-command-without-a-launcher` names `Context::start_core`, where the launcher is now set.

## Verdicts

`cargo xtask mutate step-25-fixes-keys`, on the reference machine beside other work (one job, the disk being busy): 5 breaks, each caught by the test its list names, none flagged.

| Break | Caught by |
|---|---|
| `auth-login-takes-the-terminal-after-something-slow` | `sys_auth_login_never_echoes_a_key_typed_before_its_row` |
| `auth-login-ignores-the-terminal-settings` | `sys_auth_login_at_a_terminal_honours_the_terminal_settings` |
| `npm-runs-a-script-without-its-pre-and-post` | `a_script_is_offered_with_what_npm_runs_before_and_after_it` |
| `an-unreadable-pre-script-is-left-out` | `a_script_is_offered_with_what_npm_runs_before_and_after_it` |
| `autonomous-is-offered-from-the-machine-alone` | `the_first_runs_sandbox_status_is_the_configured_one` |
