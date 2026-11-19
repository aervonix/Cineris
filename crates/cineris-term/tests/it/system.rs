//! The demo in a pseudo-terminal (ConPTY on Windows): it draws, keys reach
//! it, and the terminal is put back after a normal exit, a panic, and a
//! signal or console event. Each test runs the shell child, which runs the
//! demo and then compares the terminal's modes with those it found.
//!
//! On Windows the shell turns virtual-terminal processing off first, as the
//! old console host has it, so these tests also show that the demo turns it
//! on: otherwise its escape sequences would be on the screen as text.

use std::time::{Duration, Instant};

use cineris_testkit::VTerm;
use cineris_testkit::pty::Session;

use super::demo::STATUS_MARK;
use super::{DEMO, NOT_RESTORED, RESTORED, SHELL, STARTING, this_binary};

/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(20);

/// The coarse check on the first frame: five times the runner's budget for
/// the interface's first frame (`docs/performance-budgets.md`), for a demo
/// built without optimisation.
const FIRST_FRAME: Duration = if cfg!(windows) {
    Duration::from_millis(600)
} else {
    Duration::from_millis(250)
};

/// The coarse check on a key's echo: generous, as the budget's check in the
/// system tier is (`docs/performance-budgets.md`).
const KEY_ECHO: Duration = Duration::from_millis(250);

fn start(options: &str) -> Session {
    Session::start(&this_binary(), &SHELL.arguments(options), 100, 30).answering()
}

/// Waits for the demo's first frame and checks it came in time, with no
/// escape sequence shown as text.
fn first_frame(session: &mut Session) {
    let starting = session.wait_for(LIMIT, "the shell to start the demo", |t| t.shows(STARTING));
    let drawn = session.wait_for(LIMIT, "the first frame", |t| t.shows(STATUS_MARK));
    let took = drawn - starting;
    assert!(
        took < FIRST_FRAME,
        "the first frame took {took:?}, more than {FIRST_FRAME:?}"
    );
    let screen = session.screen().screen().join("\n");
    for literal in ["[?2026", "[?25", "[K", "[?2004", "\u{2190}["] {
        assert!(
            !screen.contains(literal),
            "an escape sequence is shown as text ({literal}):\n{screen}"
        );
    }
}

/// Waits for the shell's verdict and checks the terminal is as it was:
/// modes put back, nothing left on.
fn restored(session: &mut Session) {
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        t.shows(RESTORED) || t.shows(NOT_RESTORED)
    });
    // The verdict may arrive in pieces: read up to its end.
    session.wait_for(LIMIT, "the whole verdict", |t| {
        t.shows(RESTORED) && t.shows(")") || t.shows("after:")
    });
    let screen = session.screen();
    assert!(
        screen.shows(RESTORED),
        "the modes were not put back:\n{}",
        screen.dump()
    );
    let modes = screen.modes();
    assert!(modes.cursor_visible, "the cursor is hidden");
    assert!(!modes.sync, "a synchronized update was left open");
    assert!(!modes.bracketed_paste, "bracketed paste was left on");
    // ConPTY asks the terminal for focus events itself, for the console it
    // runs, so on Windows the mode says nothing about the demo.
    assert!(cfg!(windows) || !modes.focus, "focus events were left on");
    assert!(
        screen.keyboard().is_empty(),
        "the keyboard protocol was left on"
    );
    // ConPTY asks the terminal for the win32 input mode itself, and draws
    // with automatic wrap as it likes, so on Windows these say nothing about
    // the demo.
    if cfg!(unix) {
        assert!(!modes.win32_input, "the win32 input mode was left on");
        assert!(modes.autowrap, "automatic wrap was left off");
    }
    session.wait_exit(LIMIT);
}

/// The shell's own check: a program that ends with the terminal raw is
/// reported as not having put it back, with the modes before and after.
#[cfg(unix)]
#[test]
fn sys_the_shell_says_when_the_terminal_was_not_put_back() {
    use super::SHELL_OVER_RAW;

    let mut session =
        Session::start(&this_binary(), &SHELL_OVER_RAW.arguments(""), 100, 30).answering();
    session.wait_for(LIMIT, "the shell's verdict", |t| t.shows("after:"));
    let screen = session.screen().dump();
    assert!(session.screen().shows(NOT_RESTORED), "{screen}");
    assert!(!session.screen().shows(RESTORED), "{screen}");
    assert!(session.screen().shows("before:"), "{screen}");
    session.wait_exit(LIMIT);
}

fn editor_shows(term: &VTerm, text: &str) -> bool {
    term.shows(&format!("> {text}"))
}

#[test]
fn sys_keys_are_drawn_and_the_terminal_is_put_back_after_a_normal_exit() {
    let mut session = start("rate=400");
    first_frame(&mut session);

    let mut worst = Duration::ZERO;
    let mut typed = String::new();
    for c in ['h', 'i', '!'] {
        typed.push(c);
        let sent = session.send(c.to_string().as_bytes());
        let shown = session.wait_for(LIMIT, "the key's echo", |t| editor_shows(t, &typed));
        worst = worst.max(shown.saturating_duration_since(sent));
    }
    assert!(
        worst < KEY_ECHO,
        "a key took {worst:?} to show, more than {KEY_ECHO:?}"
    );

    // A narrower window: the live region is drawn again at the new width.
    session.resize(60, 30);
    session.send(b"?");
    session.wait_for(LIMIT, "the echo after a resize", |t| {
        editor_shows(t, "hi!?")
    });
    // The rule under the text is as wide as a line may be at the new width
    // (59 columns), which only a frame drawn after the resize was seen has.
    session.wait_for(LIMIT, "the live region drawn at the new width", |t| {
        t.screen().iter().any(|row| {
            row.chars().count() == 59 && row.chars().all(|c| c == '-' || c == '\u{2500}')
        })
    });
    let status_rows = session
        .screen()
        .screen()
        .iter()
        .filter(|row| row.contains(STATUS_MARK))
        .count();
    assert_eq!(status_rows, 1, "\n{}", session.screen().dump());

    session.send(b"\x03");
    restored(&mut session);
    // The live region is gone: the shell's prompt comes back where it was.
    assert!(
        !session.screen().shows(STATUS_MARK),
        "\n{}",
        session.screen().dump()
    );
}

#[test]
fn sys_the_terminal_is_put_back_after_a_panic() {
    let mut session = start("panic-on=P");
    first_frame(&mut session);
    let started = Instant::now();
    session.send(b"P");
    restored(&mut session);
    // The terminal was put back before the panic's message was printed:
    // each of its lines starts at the left edge, where in raw mode a line
    // break would not have returned the cursor there.
    let screen = session.screen().screen();
    assert!(
        screen
            .iter()
            .any(|row| row.starts_with("demo panic on request")),
        "the panic's message was printed in raw mode:\n{}",
        session.screen().dump()
    );
    assert!(started.elapsed() < LIMIT);
}

/// A panic another thread catches leaves the terminal the program's: the
/// demo goes on drawing in raw mode, keys reach it, and the terminal is put
/// back when it ends (the open item on a caught panic, closed in step 23:
/// `docs/steps/23/answered.md`).
#[test]
fn sys_a_panic_caught_on_another_thread_leaves_the_terminal_to_the_program() {
    let mut session = start("caught-panic-on=K");
    first_frame(&mut session);
    session.send(b"K");
    // The demo types `!` once the other thread's panic was caught.
    session.wait_for(LIMIT, "the caught panic's mark", |t| {
        t.screen().iter().any(|row| row.contains("> !"))
    });
    session.send(b"x");
    last_key(&mut session, "x");
    assert!(
        session.screen().shows("> !x"),
        "a key after the caught panic did not reach the demo:\n{}",
        session.screen().dump()
    );
    // Still in raw mode: the panic's message, printed by the default hook,
    // did not put the terminal back, so the demo's own Ctrl+C ends it.
    session.send(b"\x03");
    restored(&mut session);
}

/// An abort, which no panic hook sees: the terminal is put back by the
/// handler of SIGABRT (Unix; on Windows an abort ends the process at once,
/// `docs/outstanding/terminal-test-gaps.md`).
#[cfg(unix)]
#[test]
fn sys_the_terminal_is_put_back_after_an_abort() {
    let mut session = start("abort-on=A");
    first_frame(&mut session);
    session.send(b"A");
    restored(&mut session);
}

#[test]
fn sys_the_terminal_is_put_back_after_a_signal_or_console_event() {
    let mut session = start("terminate-on=T");
    first_frame(&mut session);
    session.send(b"T");
    restored(&mut session);
}

/// Waits until the status line names `key` as the last key read.
fn last_key(session: &mut Session, key: &str) {
    let want = format!("last key {key}");
    session.wait_for(LIMIT, &want.clone(), |t| {
        t.screen().iter().any(|row| row.trim_end().ends_with(&want))
    });
}

/// `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`: one key as Windows Terminal's win32
/// input mode reports it, pressed and released.
fn record(vk: u32, unit: u32, state: u32) -> Vec<u8> {
    let mut bytes = format!("\x1b[{vk};0;{unit};1;{state};1_").into_bytes();
    bytes.extend(format!("\x1b[{vk};0;{unit};0;{state};1_").into_bytes());
    bytes
}

#[test]
fn sys_keys_arrive_with_their_modifiers() {
    let mut session =
        Session::start(&this_binary(), &DEMO.arguments("rate=400"), 100, 30).answering();
    session.wait_for(LIMIT, "the first frame", |t| t.shows(STATUS_MARK));

    // A lone Escape is the Escape key once nothing follows it.
    // raw key: the decoder's reading of a lone Escape byte is what is tested
    session.send(b"\x1b");
    last_key(&mut session, "Esc");
    // With a key right after it, Alt with that key; and a sequence with
    // modifiers. Windows Terminal never sends these to a ConPTY, which
    // asks it for win32 input records (below) and misreads plain
    // sequences that follow an Escape: there they are checked by
    // sequence only, in the decoder's unit tests.
    if cfg!(unix) {
        // raw key: the decoder's reading of plain xterm sequences is what is tested
        session.send(b"\x1bx");
        last_key(&mut session, "Alt+x");
        // raw key: the same, with a modifier parameter
        session.send(b"\x1b[1;5A");
        last_key(&mut session, "Ctrl+Up");
        // The demo turns the win32 input mode on when the terminal says it
        // has it, as this driver does, playing Windows Terminal.
        session.wait_for(LIMIT, "the win32 input mode", |t| t.modes().win32_input);
    }

    // Keys as Windows Terminal sends them: win32 input records. Each is told
    // apart from the key plain bytes would merge it with. On Windows they
    // go through ConPTY to the demo, which asked for the mode itself.
    const SHIFT: u32 = 0x10;
    const LEFT_CTRL: u32 = 0x08;
    const LEFT_ALT: u32 = 0x02;
    const ENHANCED: u32 = 0x100;
    for (bytes, name) in [
        (record(0x0d, 13, SHIFT), "Shift+Enter"),
        (record(0x0d, 13, LEFT_CTRL), "Ctrl+Enter"),
        (record(0x09, 9, LEFT_CTRL), "Ctrl+Tab"),
        (record(0x41, 1, LEFT_CTRL | SHIFT), "Ctrl+Shift+a"),
        (record(0x58, u32::from('x'), LEFT_ALT), "Alt+x"),
        (record(0x20, 0, LEFT_CTRL), "Ctrl+Space"),
        (record(0x26, 0, LEFT_CTRL | ENHANCED), "Ctrl+Up"),
        (record(0x1b, 27, 0), "Esc"),
    ] {
        session.send(&bytes);
        last_key(&mut session, name);
    }
    session.send(b"\x03");
    session.wait_exit(LIMIT);
}

/// The mouse, on with the alternate screen. On Unix the terminal is asked
/// for its reports, which reach the program as themselves: the wheel and a
/// click. On Windows, under a pseudo-console (Windows Terminal), the
/// terminal is asked for nothing: a pseudo-console passes the modes on and
/// then gives the terminal's reports to the program as typed keys, which
/// would type them into the prompt (measured in step 25), so the terminal
/// keeps its own wheel and selection there.
#[test]
fn sys_the_mouse_reaches_the_program_on_the_alternate_screen() {
    use cineris_testkit::vterm::{MOUSE_MODES, modes_set};
    let mut session = Session::start(&this_binary(), &DEMO.arguments("mouse"), 100, 30).answering();
    // On Unix, a terminal that does not know the win32 input mode, which is
    // asked for the mouse; one that knows it is not (the unit tests).
    if cfg!(unix) {
        session = session.without_win32_input();
    }
    session.wait_for(LIMIT, "the first frame", |t| t.shows(STATUS_MARK));
    if cfg!(windows) {
        let out = session_bytes(&session);
        // Parameter by parameter: a pseudo-console asks for several modes
        // in one sequence (`ESC [ ? 1003 ; 1006 h`).
        let set = modes_set(&out);
        assert!(
            !set.iter().any(|mode| MOUSE_MODES.contains(mode)),
            "the terminal was asked for the mouse: every mode set was {set:?}"
        );
        assert!(!session.screen().modes().mouse);
    } else {
        // Asked for, and still on (the terminal's state, not only what was
        // ever written).
        let set = modes_set(&session_bytes(&session));
        assert!(
            set.contains(&1000) && set.contains(&1006),
            "the mouse was not asked for: every mode set was {set:?}"
        );
        let modes = session.screen().modes();
        assert!(modes.mouse && modes.mouse_sgr && !modes.win32_input);
        session.send(b"\x1b[<64;10;5M");
        last_key(&mut session, "Wheel up");
        session.send(b"\x1b[<0;4;3M");
        last_key(&mut session, "Press Left");
        session.send(b"\x1b[<0;4;3m");
        last_key(&mut session, "Release Left");
    }
    session.send(b"\x03");
    session.wait_exit(LIMIT);
}

/// Everything the program (and the pseudo-console between) has written.
fn session_bytes(session: &Session) -> Vec<u8> {
    session
        .recording()
        .iter()
        .flat_map(|chunk| chunk.bytes.iter().copied())
        .collect()
}

/// A sequence that sets several modes counts each, and a mode reset is not
/// a mode set.
#[test]
fn every_parameter_of_a_mode_set_is_read() {
    use cineris_testkit::vterm::modes_set;
    let bytes = b"\x1b[?1049h\x1b[?1003;1006h\x1b[?1000l\x1b[?25h\x1b[?9001;h";
    assert_eq!(modes_set(bytes), [1049, 1003, 1006, 25, 9001]);
}

/// In a terminal that knows the win32 input mode, as Windows Terminal (and
/// a Linux build under WSL behind it) has it, the demo turns the mode on
/// and the mouse ends off: through a pseudo-console its reports would be
/// typed (step 25's follow-up).
#[cfg(unix)]
#[test]
fn sys_a_terminal_in_the_win32_input_mode_is_not_left_asked_for_the_mouse() {
    let mut session = Session::start(&this_binary(), &DEMO.arguments("mouse"), 100, 30).answering();
    session.wait_for(LIMIT, "the mode on and the mouse off", |t| {
        t.shows(STATUS_MARK) && t.modes().win32_input && !t.modes().mouse
    });
    session.send(b"\x03");
    session.wait_exit(LIMIT);
}

/// Whether `haystack` holds `needle`.
#[cfg(unix)]
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Ctrl+Z: the demo puts the terminal back and stops itself; continued, it
/// turns everything on again and draws.
///
/// The demo here leads its own session in the pseudo-terminal, so its
/// process group has no parent in that session: Linux does not stop such a
/// group, and the demo goes straight on as if continued at once. What is
/// checked is what the demo writes on the way in and out of the stop, in
/// order. The stop itself is not reached by any test
/// (outstanding/terminal-test-gaps.md).
#[cfg(unix)]
#[test]
fn sys_after_ctrl_z_and_fg_everything_is_on_again() {
    let mut session =
        Session::start(&this_binary(), &DEMO.arguments("rate=400"), 100, 30).answering();
    session.wait_for(LIMIT, "the first frame", |t| {
        t.shows(STATUS_MARK) && t.modes().bracketed_paste && t.modes().win32_input
    });
    let mark: usize = session.recording().iter().map(|c| c.bytes.len()).sum();
    session.send(b"\x1a");
    // As a shell's `fg` would, in case the system did stop it.
    cineris_term::pty::continue_process(session.pid()).unwrap();
    let since = |session: &Session| -> Vec<u8> {
        let all: Vec<u8> = session
            .recording()
            .iter()
            .flat_map(|chunk| chunk.bytes.iter().copied())
            .collect();
        all[mark..].to_vec()
    };
    let deadline = Instant::now() + LIMIT;
    loop {
        let bytes = since(&session);
        // Put back (paste, the win32 input mode off, wrap on), then all of
        // it on again and a frame.
        if let Some(back) = bytes
            .windows(8)
            .position(|w| w == b"\x1b[?2004l")
            .filter(|&at| holds(&bytes[..at], b"\x1b[?9001l") && holds(&bytes, b"\x1b[?7h"))
            && holds(&bytes[back..], b"\x1b[?2004h\x1b[?1004h\x1b[?7l")
            && holds(&bytes[back..], b"\x1b[?9001h")
            && session.screen().shows(STATUS_MARK)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the demo did not put the terminal back and turn it on again: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        session.step(Duration::from_millis(50));
    }
    let modes = session.screen().modes();
    assert!(modes.bracketed_paste && modes.focus && modes.win32_input && !modes.autowrap);
    session.send(b"k");
    session.wait_for(LIMIT, "a key after resuming", |t| editor_shows(t, "k"));
    session.send(b"\x03");
    session.wait_exit(LIMIT);
}
