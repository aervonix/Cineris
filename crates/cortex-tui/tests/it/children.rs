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

use std::io::Write as _;
use std::path::{Path, PathBuf};

use cortex_term::{Event, Frame, Inline, Line, Overrides, Terminal};
use cortex_tui::editor::{Chip, Fragment};
use cortex_tui::external::{self, Edited, Env, TerminalHandoff};

/// The name of the entry test.
pub(crate) const ENTRY: &str = "child_entry";
/// The argument that makes this binary the shell: `scenario|scratch`.
pub(crate) const SHELL: &str = "cxtui-shell=";
const APP: &str = "cxtui-app=";
const EDITOR: &str = "cxtui-editor=";
/// The shell's verdicts.
pub(crate) const RESTORED: &str = "restored: yes";
pub(crate) const NOT_RESTORED: &str = "restored: NO";
/// What the application draws while it waits for a key.
pub(crate) const WAITING: &str = "waiting for a key";
/// What the application prints when it has no terminal after the editor.
pub(crate) const LOST: &str = "the application has no terminal";
/// What the editor prints while it waits for a line.
pub(crate) const EDITOR_WAITING: &str = "editor waiting for a line";
/// The variables the shell gives the application, and through it the
/// editor.
const MODES: &str = "CXTUI_MODES";
const SCRATCH: &str = "CXTUI_SCRATCH";

/// The arguments that start this binary as a child.
pub(crate) fn child_arguments(mark: &str, options: &str) -> Vec<String> {
    vec![
        "--exact".to_owned(),
        ENTRY.to_owned(),
        "--nocapture".to_owned(),
        "--test-threads=1".to_owned(),
        format!("{mark}{options}"),
    ]
}

/// This test binary's path.
pub(crate) fn this_binary() -> String {
    std::env::current_exe()
        .expect("the test binary has a path")
        .to_str()
        .expect("the test binary's path is Unicode")
        .to_owned()
}

/// Becomes the child the arguments name, if they name one.
pub(crate) fn enter() {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(options) = arguments.iter().find_map(|a| a.strip_prefix(SHELL)) {
        shell(options);
    }
    if let Some(scenario) = arguments.iter().find_map(|a| a.strip_prefix(APP)) {
        app(scenario);
    }
    if let Some(scenario) = arguments.iter().find_map(|a| a.strip_prefix(EDITOR)) {
        let file = arguments.last().expect("the file's path");
        editor(scenario, Path::new(file));
    }
}

/// Appends `name: value` to the report.
fn report(name: &str, value: &str) {
    let scratch = std::env::var_os(SCRATCH).expect("the scratch directory");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(PathBuf::from(scratch).join("report.txt"))
        .expect("open the report");
    writeln!(file, "{name}: {value}").expect("write the report");
}

fn yes(ok: bool) -> &'static str {
    if ok { "yes" } else { "NO" }
}

fn shell(options: &str) -> ! {
    cortex_term::pty::ignore_control_events();
    let (scenario, scratch) = options.split_once('|').expect("scenario|scratch");
    let before = cortex_term::pty::modes().expect("the terminal's modes");
    // As a shell does with its job: a Ctrl+C typed for the editor is not
    // the shell's. The application keeps its own (see the guard).
    let ignored = cortex_term::ignore_interrupts().expect("ignore interrupts");
    let status = std::process::Command::new(this_binary())
        .args(child_arguments(APP, scenario))
        .env(MODES, &before)
        .env(SCRATCH, scratch)
        .status()
        .expect("start the application");
    drop(ignored);
    let after = cortex_term::pty::modes().expect("the terminal's modes");
    let verdict = if before == after {
        RESTORED
    } else {
        NOT_RESTORED
    };
    print!("\r\n{verdict} ({status})\r\n");
    std::io::stdout().flush().expect("write to the terminal");
    #[expect(clippy::exit, reason = "a child ends when it has reported")]
    std::process::exit(0);
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
        _ => {
            let arguments = child_arguments(EDITOR, scenario).join(" ");
            format!("\"{}\" {arguments}", this_binary())
        }
    }
}

fn app(scenario: &str) -> ! {
    let scratch = PathBuf::from(std::env::var_os(SCRATCH).expect("the scratch directory"));
    let temp = scratch.join("temp");
    std::fs::create_dir_all(&temp).expect("the temporary folder");
    let mut terminal = Some(Terminal::enter(Overrides::default()).expect("enter the terminal"));
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

    if terminal.is_none() {
        report("raw again", "NO: the terminal was not taken back");
        print!("\r\n{LOST}\r\n");
        std::io::stdout().flush().expect("write to the terminal");
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
    let modes = cortex_term::pty::modes().expect("the editor's terminal");
    let shell = std::env::var(MODES).expect("the shell's modes");
    report("editor modes", yes(modes == shell));
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
    print!("{EDITOR_WAITING}\r\n");
    std::io::stdout().flush().expect("write to the terminal");
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
            print!("\x1b[?1049h\x1b[?1000h\x1b[>1u\x1b[>1u");
            std::io::stdout().flush().expect("write to the terminal");
            std::process::abort()
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
