//! The parts this test binary plays when it is run again as a child, for
//! the external editor's system tests: a shell, the application that hands
//! its terminal to an editor, and the editor.
//!
//! - **The shell** records its terminal's modes, runs the application, and
//!   prints whether the modes after it are the ones before.
//! - **The application** enters the terminal, draws, edits a prompt with
//!   the external editor, says what came back and whether the terminal is
//!   in its raw modes again, then waits for one key and leaves.
//! - **The editor** says whether the terminal it was given has the shell's
//!   modes and whether its file and folder are private, then does what its
//!   scenario says to the file.
//!
//! The application and the editor write what they found to a report file in
//! the test's scratch directory, one `name: value` line each.

use std::path::{Path, PathBuf};

use cortex_term::{Event, Frame, Inline, Line, Overrides, Terminal};
use cortex_testkit::role::{ModeShell, Report, Role, same_modes_as_the_shell, write_terminal, yes};
pub(crate) use cortex_testkit::role::{NOT_RESTORED, RESTORED, this_binary};
use cortex_tui::editor::{Chip, Fragment};
use cortex_tui::external::{self, Edited, Env, TerminalHandoff};

/// This binary as the shell: `scenario|scratch`.
pub(crate) const SHELL: Role = Role::new("cxtui-shell=");
const APP: Role = Role::new("cxtui-app=");
const EDITOR: Role = Role::new("cxtui-editor=");
/// What the application draws while it waits for a key.
pub(crate) const WAITING: &str = "waiting for a key";
/// What the application prints when it has no terminal after the editor.
pub(crate) const LOST: &str = "the application has no terminal";
/// What the application prints when a resize reached it after the editor.
pub(crate) const RESIZED: &str = "the application was resized";
/// What the editor prints while it waits for a line.
pub(crate) const EDITOR_WAITING: &str = "editor waiting for a line";
/// The variable the shell gives the application, and through it the
/// editor: the test's scratch directory, where the report is.
const SCRATCH: &str = "CXTUI_SCRATCH";

/// Becomes the child the arguments name, if they name one.
pub(crate) fn enter() {
    if let Some(options) = SHELL.options() {
        let (scenario, scratch) = options.split_once('|').expect("scenario|scratch");
        // As a shell does with its job: a Ctrl+C typed for the editor is
        // not the shell's. The application keeps its own (see the guard).
        let mut app = APP.command(scenario);
        app.env(SCRATCH, scratch);
        ModeShell::new().ignoring_interrupts().run(app);
    }
    if let Some(scenario) = APP.options() {
        app(&scenario);
    }
    if let Some(scenario) = EDITOR.options() {
        let arguments: Vec<String> = std::env::args().collect();
        let file = arguments.last().expect("the file's path");
        editor(&scenario, Path::new(file));
    }
}

/// Whether process `pid` has taken signal `signal` (no longer pending), by
/// its `/proc` status, waiting for it at most 30 s. Linux only.
#[cfg(target_os = "linux")]
fn signal_taken(pid: &str, signal: u32) -> bool {
    let bit = 1u64 << (signal - 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        let pending = status
            .lines()
            .filter_map(|line| {
                line.strip_prefix("SigPnd:")
                    .or_else(|| line.strip_prefix("ShdPnd:"))
            })
            .filter_map(|mask| u64::from_str_radix(mask.trim(), 16).ok())
            .any(|mask| mask & bit != 0);
        if !pending {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

/// Appends `name: value` to the report in the test's scratch directory.
fn report(name: &str, value: &str) {
    let scratch = std::env::var_os(SCRATCH).expect("the scratch directory");
    Report::in_dir(Path::new(&scratch)).say(name, value);
}

/// The editor's value for a scenario, as `$EDITOR` would hold it.
fn editor_value(scenario: &str, scratch: &Path) -> String {
    match scenario {
        "missing" => scratch.join("no-such-editor").display().to_string(),
        "broken" => {
            let name = if cfg!(windows) {
                "broken.exe"
            } else {
                "broken"
            };
            let path = scratch.join(name);
            std::fs::write(&path, b"not a program").expect("write the broken editor");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("make it look runnable");
            }
            path.display().to_string()
        }
        _ => EDITOR.command_line(scenario),
    }
}

fn app(scenario: &str) -> ! {
    let scratch = PathBuf::from(std::env::var_os(SCRATCH).expect("the scratch directory"));
    let temp = scratch.join("temp");
    std::fs::create_dir_all(&temp).expect("the temporary folder");
    let mut terminal = Some(Terminal::enter(Overrides::default()).expect("enter the terminal"));
    // As the application does: a signal is its own to end, and its
    // handlers stay while the editor has the terminal.
    terminal.as_mut().unwrap().hand_signals_over();
    let (cols, rows) = terminal.as_ref().unwrap().size().expect("the size");
    let mut inline = Inline::new(cols, rows);
    draw(&mut terminal, &mut inline, &format!("app: {scenario}"));
    let raw = cortex_term::pty::modes().expect("the raw modes");

    let prompt = if scenario == "chip" {
        let mut prompt = Fragment::text("see ");
        prompt.push(Fragment::chip(Chip::new(
            "l\n".repeat(13),
            "[pasted 13 lines]",
        )));
        prompt
    } else {
        Fragment::text("hello")
    };
    let env = Env {
        // Never the developer's own `$VISUAL`, which is looked at first.
        visual: None,
        editor: Some(editor_value(scenario, &scratch).into()),
        ..Env::current()
    };
    let result = match external::find(&env) {
        Err(error) => format!("error: {error:?}"),
        Ok(program) => {
            let mut handoff = TerminalHandoff {
                terminal: &mut terminal,
                inline: &mut inline,
                overrides: Overrides::default(),
            };
            match external::edit(&prompt, &program, &mut handoff, &temp) {
                Ok(Edited::Changed(fragment)) => format!(
                    "changed {:?} with {} chips",
                    fragment.expanded(),
                    fragment.chips().len()
                ),
                Ok(other) => format!("{other:?}"),
                Err(error) => format!("error: {error:?}"),
            }
        }
    };
    report("result", &result);
    let again = cortex_term::pty::modes().expect("the modes after");
    report("raw again", yes(again == raw));
    report(
        "temporary folder empty",
        yes(std::fs::read_dir(&temp).expect("list it").next().is_none()),
    );

    if scenario == "signal" {
        // A signal came while the editor had the terminal: it is not taken
        // back, and the program ends as the signal asks.
        report("taken back", yes(terminal.is_some()));
        report("ending", &format!("{:?}", cortex_term::signalled()));
        cortex_term::signal_ended();
        #[expect(clippy::exit, reason = "a child ends when it has reported")]
        std::process::exit(0);
    }
    if terminal.is_none() {
        report("raw again", "NO: the terminal was not taken back");
        write_terminal(&format!("\r\n{LOST}\r\n"));
        #[expect(clippy::exit, reason = "a child ends when it has reported")]
        std::process::exit(1);
    }
    draw(&mut terminal, &mut inline, WAITING);
    let terminal_now = terminal.as_mut().expect("the terminal is back");
    loop {
        match terminal_now.next(None).expect("read a key") {
            Some(Event::Key(key)) => {
                report("key", &format!("{:?}", key.code));
                break;
            }
            // The resize handler is in place again after the editor.
            Some(Event::Resize { .. }) => {
                report("resized", "yes");
                write_terminal(&format!("\r\n{RESIZED}\r\n"));
            }
            _ => continue,
        }
    }
    let mut terminal = terminal.take().expect("the terminal");
    let caps = *terminal.caps();
    inline.finish(&caps, &mut terminal).expect("finish");
    terminal.leave().expect("leave");
    #[expect(clippy::exit, reason = "a child ends when it has reported")]
    std::process::exit(0);
}

fn draw(terminal: &mut Option<Terminal>, inline: &mut Inline, text: &str) {
    let terminal = terminal.as_mut().expect("the terminal");
    let caps = *terminal.caps();
    let live = [Line::plain(text)];
    inline
        .draw(
            &Frame {
                commit: &[],
                live: &live,
                cursor: None,
            },
            &caps,
            terminal,
        )
        .expect("draw");
}

fn editor(scenario: &str, file: &Path) -> ! {
    report(
        "editor modes",
        yes(same_modes_as_the_shell().expect("the shell's modes")),
    );
    report(
        "private file",
        yes(cortex_base::private::is_private(file).expect("look at the file")),
    );
    let folder = file.parent().expect("the file's folder");
    report(
        "private folder",
        yes(cortex_base::private::is_private(folder).expect("look at the folder")),
    );
    let had = std::fs::read_to_string(file).expect("read the prompt");
    report("file had", &format!("{had:?}"));
    report(
        "editor folder",
        yes(
            std::fs::canonicalize(std::env::current_dir().expect("its folder")).ok()
                == std::fs::canonicalize(folder).ok(),
        ),
    );
    // The test looks at the terminal now, then sends a line: read only if
    // the terminal is in its ordinary, line-by-line mode. A Ctrl+C instead
    // ends this editor.
    write_terminal(&format!("{EDITOR_WAITING}\r\n"));
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).expect("read a line");
    report("editor read", &format!("{line:?}"));
    let code = match scenario {
        "change" => {
            std::fs::write(file, had.replace("hello", "hello edited")).expect("write");
            0
        }
        "chip" => {
            std::fs::write(file, had.replace("see", "look at")).expect("write");
            0
        }
        "newline" => {
            std::fs::write(file, format!("{had}\n")).expect("write");
            0
        }
        "empty" => {
            std::fs::write(file, "").expect("write");
            0
        }
        "fail" => {
            std::fs::write(file, "changed, then failed").expect("write");
            3
        }
        "abort" => {
            // Killed in the middle of a full-screen session: the alternate
            // screen, the mouse and keyboard flags left on.
            write_terminal("\x1b[?1049h\x1b[?1000h\x1b[>1u\x1b[>1u");
            std::process::abort()
        }
        // The application is sent SIGTERM while this editor has the
        // terminal; the editor then ends, its change saved.
        #[cfg(target_os = "linux")]
        "signal" => {
            let parent = std::os::unix::process::parent_id().to_string();
            let sent = std::process::Command::new("kill")
                .args(["-TERM", parent.as_str()])
                .status()
                .expect("kill");
            report("signal sent", yes(sent.success()));
            // The application takes the signal on whichever of its threads
            // the kernel gives it to: the editor ends only once it has, as
            // it would for a person, who does not quit an editor within
            // microseconds of the signal.
            report("signal taken", yes(signal_taken(&parent, 15)));
            std::fs::write(file, had.replace("hello", "hello edited")).expect("write");
            0
        }
        "nottext" => {
            std::fs::write(file, b"caf\xe9 au lait").expect("write");
            0
        }
        "bom" => {
            std::fs::write(
                file,
                format!("\u{FEFF}{}\r\n", had.replace("hello", "bonjour")),
            )
            .expect("write");
            0
        }
        _ => 0,
    };
    #[expect(clippy::exit, reason = "the editor's exit code is the scenario's")]
    std::process::exit(code);
}
