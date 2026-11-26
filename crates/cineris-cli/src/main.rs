//! The `cineris` binary, also installed under the alias `cin`.
//!
//! It parses the command line, chooses a mode, drives the core and exits
//! with the code of the table in `docs/architecture/10-headless.md`.
//!
//! | Module | What it does |
//! |---|---|
//! | [`args`] | The command line, read into what to do. |
//! | [`help`] | The help, in the name the program was started with. |
//! | [`headless`] | A prompt run to completion, with standard output for the result. |
//! | [`interactive`] | The terminal UI on a session: the core's thread, the in-process link, start-up. |
//! | [`report`] | The result record and what each output format writes. |
//! | [`exit`] | The exit codes. |
//! | [`signals`] | Ctrl-C, SIGTERM and the console's events. |
//! | [`commands`] | The subcommands: sessions, configuration, permissions, credentials. |
//! | [`serve`] | The RPC server over standard input and output. |
//!
//! The program behaves identically under both names. It reads the name it
//! was started with only to echo it in usage, help and error text
//! (`docs/decisions/0038-the-project-is-named-cineris.md`).

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the binary is the one place that writes to the user's terminal"
)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the binary's own threads read standard input, the working directory and the \
              environment, outside the core's runtime"
)]

mod args;
mod attach;
mod commands;
mod exit;
mod export;
mod ext;
mod headless;
mod help;
mod history;
mod interactive;
mod json;
mod nested;
mod project_view;
mod report;
mod serve;
mod setup;
mod signals;
mod usage_view;
mod workspace;

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use crate::args::{Command, Run};

/// The command's own name. `--version` prints it under either name.
const COMMAND: &str = "cineris";
/// The short alias: the same executable under a second name.
const ALIAS: &str = "cin";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let started = Instant::now();
    let mut arguments = env::args_os();
    let name = invoked_name(arguments.next().as_deref());
    let rest: Vec<OsString> = arguments.collect();

    // The guard of process trees is started by the harness itself, and is
    // started for every command: nothing else is done before it.
    if rest.first().is_some_and(|first| first == args::GUARD) {
        cineris_sandbox::tree::run_guard();
        return ExitCode::SUCCESS;
    }
    // The sandbox launcher: it confines itself and becomes the command, and
    // returns only when it could not.
    if rest.first().is_some_and(|first| first == args::SANDBOX) {
        let code = cineris_sandbox::launch::run(rest.into_iter().skip(1).collect());
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }

    let command = match args::parse(rest) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{name}: {}", error.0);
            eprintln!("Try '{name} --help'.");
            return ExitCode::from(exit::USAGE);
        }
    };
    ExitCode::from(dispatch(command, name, started))
}

fn dispatch(command: Command, name: &'static str, started: Instant) -> u8 {
    match command {
        Command::Help(topic) => {
            print_out(&help::help(topic, name));
            exit::COMPLETED
        }
        Command::Version => {
            print_out(&format!("{COMMAND} {VERSION}\n"));
            exit::COMPLETED
        }
        Command::Run(run) => match mode(&run, std::io::stdout().is_terminal()) {
            Mode::Headless => headless::run(*run, name, started),
            Mode::Interactive => interactive::run(*run, name, started),
        },
        Command::Sessions(command) => commands::sessions(command, name),
        Command::Config(command) => commands::config(command, name),
        Command::Permissions(command) => commands::permissions(command, name),
        Command::Auth(command) => commands::auth(command, name),
        Command::Models => commands::models(name),
        Command::Usage(command) => commands::usage(command, name),
        Command::Stats(command) => commands::stats(command, name),
        Command::Project(command) => commands::project(command, name),
        Command::Ext(command) => ext::ext(command, name),
        Command::Setup(command) => setup::command(command, name),
        Command::Serve(serve) => serve::run(*serve, name),
        Command::Attach(attach) => attach::run(*attach, name),
    }
}

/// Marks this process as one that runs sessions, before it starts any
/// process (`cineris_sandbox::session`): on Windows it joins a job of its
/// own for its sessions' commands, which the approving subcommands look
/// for. Where it cannot, it does not run unmarked at full width: its
/// sessions are held at [`UNMARKED_LEVEL`] at most, and what it says of that
/// is returned, for the run to show (the full review of the third part).
pub(crate) fn mark_sessions() -> Option<String> {
    let error = cineris_sandbox::session::mark().err()?;
    tracing::warn!(
        %error,
        "this process could not make a job of its own for its sessions' commands"
    );
    Some(format!("{UNMARKED} ({error})"))
}

/// The widest level of a harness that could not mark its commands.
pub(crate) const UNMARKED_LEVEL: cineris_protocol::lease::Level =
    cineris_protocol::lease::Level::Supervised;

/// What a harness that could not mark its commands says.
pub(crate) const UNMARKED: &str = "this harness holds its sessions at `supervised` at most, \
                                   whatever its configuration says: it could not make a job of \
                                   its own for its commands, so the subcommands that approve \
                                   extensions and trust projects could not see them as a \
                                   session's";

/// `Err` with the exit code when `what`, a subcommand that approves,
/// trusts or answers for the person, runs under a session's command, or
/// where that cannot be told: it is refused before anything is read
/// (since step 28's third pull request, the owner's answer).
pub(crate) fn not_under_a_session(name: &str, what: &str) -> Result<(), u8> {
    match cineris_sandbox::session::under_a_session().refusal() {
        None => Ok(()),
        Some(why) => {
            let named = if what.is_empty() {
                format!("`{name}`")
            } else {
                format!("`{name} {what}`")
            };
            eprintln!(
                "{name}: {named} was refused, and nothing was read or written: {why}. It \
                 approves extensions, trusts projects or answers for you, so a session's command \
                 may not run it: run it in your own terminal."
            );
            Err(exit::USAGE)
        }
    }
}

/// What a headless run held to `read-only` under a session's command is
/// said to be, on standard error and in its result.
pub(crate) const CAPPED: &str = "this run is held to `read-only`, whatever its configuration \
                                 says, because it runs under a session's command, whose own \
                                 level is not known to it";

/// The open item that names the precise cap, by its file's name.
pub(crate) const CAPPED_ITEM: &str = "a-nested-run-under-a-session-is-capped-at-read-only";

/// Under a session's command, a headless run carries no flag the CLI's
/// grammar refuses there, and its level is at most `read-only`: the
/// session that started it may be at no more, and is not known (the
/// owner's answer to the early review, B). `Err` with the exit code when it
/// carries one; `Ok(true)` when it runs capped.
pub(crate) fn nested_run(name: &str, run: &crate::args::Run) -> Result<bool, u8> {
    if cineris_sandbox::session::under_a_session()
        .refusal()
        .is_none()
    {
        return Ok(false);
    }
    let command = crate::args::Command::Run(Box::new(run.clone()));
    if let cineris_core::approving::Standing::Refused(refusal) = crate::nested::standing(&command) {
        eprintln!(
            "{name}: `{}` was refused under a session's command: {}",
            refusal.what,
            cineris_core::approving::reason(&refusal)
        );
        return Err(exit::USAGE);
    }
    // Its configuration and its approvals are read where the session's
    // are, or it does not run (the re-look at the early review, B).
    if let Some(moved) = cineris_sandbox::session::moved_stores() {
        eprintln!(
            "{name}: `{name} -p` was refused under a session's command: {moved} is set otherwise \
             than for the session that started it, and moves where its configuration and its \
             approvals are read from. If you mean it, run it in your own terminal."
        );
        return Err(exit::USAGE);
    }
    Ok(true)
}

/// Writes `text` to standard output. A reader that has gone (`| head`)
/// ends the output quietly, where `print!` would panic.
pub(crate) fn print_out(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Text as it is safe to show at a terminal: every character that would act
/// unseen but a line break and a tab (a control character, a bidirectional
/// control, a tag character) is written as an escape (`\u{1b}`), so that
/// text a repository or a model wrote cannot move the cursor, clear a line,
/// reorder what is next to it or hide text: `cineris_base::visible`'s set,
/// since the fixes of the independent review of step 18 found this one
/// passing the format characters.
pub(crate) fn visible(text: &str) -> std::borrow::Cow<'_, str> {
    cineris_base::visible_keeping(text, &['\n', '\t'])
}

/// How a run is carried out.
#[derive(Debug, PartialEq, Eq)]
enum Mode {
    Headless,
    Interactive,
}

/// Headless with `-p`, or when a prompt is given and standard output is
/// not a terminal (`docs/architecture/10-headless.md`, "Invocation").
fn mode(run: &Run, stdout_is_terminal: bool) -> Mode {
    if run.print || (run.prompt.is_some() && !stdout_is_terminal) {
        Mode::Headless
    } else {
        Mode::Interactive
    }
}

/// The name to show in usage and error text: `cin` when the program was
/// started under its alias, `cineris` otherwise.
///
/// Only the file name counts, without its directory or extension, and without
/// regard to case, so `C:\tools\CIN.EXE` is `cin`. An unexpected or missing
/// name is treated as `cineris`.
fn invoked_name(arg0: Option<&OsStr>) -> &'static str {
    let stem = arg0
        .map(Path::new)
        .and_then(Path::file_stem)
        .and_then(OsStr::to_str);
    match stem {
        Some(stem) if stem.eq_ignore_ascii_case(ALIAS) => ALIAS,
        _ => COMMAND,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::{Mode, Run, invoked_name, mode, visible};

    #[test]
    fn control_characters_are_shown_as_escapes() {
        assert_eq!(visible("plain\ttext\nnext"), "plain\ttext\nnext");
        assert_eq!(visible("a\u{1b}[2Kb"), "a\\u{1b}[2Kb");
        assert_eq!(visible("\r\u{7}\u{9b}"), "\\r\\u{7}\\u{9b}");
        assert_eq!(visible("a\u{202E}b\u{E0041}c"), "a\\u{202e}b\\u{e0041}c");
    }

    fn name_for(arg0: &str) -> &'static str {
        invoked_name(Some(OsStr::new(arg0)))
    }

    #[test]
    fn the_alias_is_recognised_by_file_name() {
        assert_eq!(name_for("cin"), "cin");
        assert_eq!(name_for("./cin"), "cin");
        assert_eq!(name_for("bin/cin"), "cin");
        assert_eq!(name_for("cin.exe"), "cin");
        assert_eq!(name_for("CIN.EXE"), "cin");
    }

    #[cfg(windows)]
    #[test]
    fn the_alias_is_recognised_in_a_windows_path() {
        assert_eq!(name_for(r"C:\tools\CIN.EXE"), "cin");
        assert_eq!(name_for(r"C:\tools\cineris.exe"), "cineris");
    }

    #[test]
    fn every_other_name_is_cineris() {
        assert_eq!(name_for("cineris"), "cineris");
        assert_eq!(name_for("target/debug/cineris"), "cineris");
        assert_eq!(name_for("cineris.exe"), "cineris");
        assert_eq!(name_for("renamed-by-the-user"), "cineris");
        assert_eq!(name_for("cinx"), "cineris");
        assert_eq!(name_for(""), "cineris");
        assert_eq!(invoked_name(None), "cineris");
    }

    #[test]
    fn a_run_is_headless_with_print_or_a_prompt_and_no_terminal() {
        let with = |print: bool, prompt: Option<&str>| Run {
            print,
            prompt: prompt.map(str::to_owned),
            ..Run::default()
        };
        assert_eq!(mode(&with(true, None), true), Mode::Headless);
        assert_eq!(mode(&with(true, Some("x")), true), Mode::Headless);
        assert_eq!(mode(&with(false, Some("x")), false), Mode::Headless);
        assert_eq!(mode(&with(false, Some("x")), true), Mode::Interactive);
        assert_eq!(mode(&with(false, None), false), Mode::Interactive);
        assert_eq!(mode(&with(false, None), true), Mode::Interactive);
    }
}
