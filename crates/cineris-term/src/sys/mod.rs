//! The operating system's side of the terminal: one module per system, the
//! only places in this crate where `unsafe` is allowed, and what both share.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

/// What ended a wait for input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    /// Bytes arrived.
    Input,
    /// The window changed size.
    Resize,
    /// The program was stopped and has been continued: the terminal was
    /// put back while it was stopped, and is in raw mode again.
    Resumed,
    /// A [`Waker`](crate::Waker) woke the wait.
    Woken,
    /// The time ran out.
    Timeout,
}

/// Which keyboard reporting was turned on, which screens hold the keyboard
/// protocol's flags, and whether the alternate screen is on, so that putting
/// the terminal back turns them off: [`KEYBOARD`], [`WIN32`], [`ALTERNATE`]
/// and [`MAIN_KEYBOARD`] as bits.
static PROTOCOLS: AtomicU8 = AtomicU8::new(0);
/// The enhanced keyboard protocol: while the alternate screen is shown, its
/// flags are pushed there.
const KEYBOARD: u8 = 1;
/// Windows Terminal's win32 input mode.
const WIN32: u8 = 2;
/// The alternate screen, while the viewer or the full-screen presenter is
/// shown (since step 23). Mouse reporting is on only with it (since step
/// 25), so its restore turns that off too.
const ALTERNATE: u8 = 4;
/// The main screen's stack of keyboard flags holds the program's push
/// (since the fixes of step 25's review). Apart from [`KEYBOARD`]: the full
/// screen pushes on the alternate screen before the main screen has had the
/// flags, and a pop there would take the entry of the shell or of the
/// program the terminal runs under.
const MAIN_KEYBOARD: u8 = 8;

/// What puts the terminal's own state back after a panic, a signal or a
/// console event, when the presenter could not finish, put together by
/// [`restore`] from these parts, in this order: it ends a frame that was cut
/// short and resets the style ([`RESTORE_BEGIN`]); with the alternate
/// screen on, it turns mouse reporting off (modes 1006 and 1000, on only
/// with it since step 25), pops the keyboard protocol's flags if they are
/// on (the alternate screen's stack is its own) and leaves the alternate
/// screen, so that the main screen comes back as it was; it erases the live
/// region from its first row down ([`ERASE`]; the cursor is moved up the
/// region's rows above it first, so that a message printed next is not
/// mixed into it); it pops the main screen's flags only if the program
/// pushed there ([`MAIN_KEYBOARD`]); it turns the win32 input mode off if
/// it was on, and the rest off, with automatic wrap back on
/// ([`RESTORE_END`]). The raw-mode or console-mode half is the system's.
const RESTORE_BEGIN: &[u8] = b"\x1b[?2026l\x1b[0m";
/// Mouse reporting off, the SGR form first.
const RESTORE_MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1000l";
/// One level of the keyboard protocol's flags popped, on the screen shown.
const RESTORE_KEYBOARD_POP: &[u8] = b"\x1b[<u";
/// The alternate screen left.
const RESTORE_ALTERNATE_OFF: &[u8] = b"\x1b[?1049l";
/// The win32 input mode off.
const RESTORE_WIN32_OFF: &[u8] = b"\x1b[?9001l";
/// Focus reports and bracketed paste off, automatic wrap on, the cursor
/// shown.
const RESTORE_END: &[u8] = b"\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h";

/// The restore sequence for the terminal's state now, without the move up
/// to the live region's first row ([`restore`] adds it): for tests.
#[cfg(test)]
pub(crate) fn restore_sequence() -> Vec<u8> {
    let mut buffer = [0u8; RESTORE_MOST];
    restore(&mut buffer, 0).to_vec()
}

/// The most one wait for input reads, in bytes: one `read` of the terminal
/// on Unix, and on Windows the console's records gathered until this many
/// bytes or none waits (the review of step 25's second pull request). A
/// paste comes in one read where it can, so that its line breaks are seen
/// inside it and not at a read's end.
pub(crate) const READ_BYTES: usize = 4096;

/// Whether a wait that has read `read` bytes of input reads the console's
/// records again: while `waiting` more are there, up to [`READ_BYTES`]
/// (Windows; since the review of step 25's second pull request, when one
/// read of at most 64 records, about 32 pasted characters with their key
/// releases, cut a paste into many).
#[cfg(any(windows, test))]
pub(crate) fn reads_more(read: usize, waiting: u32) -> bool {
    waiting > 0 && read < READ_BYTES
}

/// The whole milliseconds left until `deadline` at `now`, rounded up, for a
/// wait that takes milliseconds: `u32::MAX`, which is no end, when there is
/// no deadline, and just under it for a deadline too far away to count
/// (since the review of step 25: each system's wait waits to a deadline).
pub(crate) fn millis_left(deadline: Option<std::time::Instant>, now: std::time::Instant) -> u32 {
    deadline.map_or(u32::MAX, |deadline| {
        let left = deadline.saturating_duration_since(now);
        u32::try_from(left.as_nanos().div_ceil(1_000_000))
            .unwrap_or(u32::MAX)
            .min(u32::MAX - 1)
    })
}

/// Where in every restore sequence the erase of the live region begins.
const ERASE: &[u8] = b"\r\x1b[J";

/// How many rows of the live region are above the cursor once the frame
/// last handed to the terminal has landed (the presenter says so before it
/// writes the frame): the emergency restore moves up that many before it
/// erases, so that the region's top rows (the input box's top border, the
/// text that streamed) do not stay on the screen (since the review of step
/// 23).
static ROWS_ABOVE: AtomicU16 = AtomicU16::new(0);

/// Set while a write to the terminal is under way: the cursor is then
/// anywhere in the frame, and a restore that interrupts it moves up
/// nothing; the write sends the restore again once its frame has landed,
/// when [`ROWS_ABOVE`] is exact.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Records how many rows of the live region will be above the cursor once
/// the frame about to be written has landed.
pub(crate) fn set_rows_above(rows: usize) {
    ROWS_ABOVE.store(u16::try_from(rows).unwrap_or(u16::MAX), Ordering::SeqCst);
}

/// A write to the terminal begins (`true`) or has ended.
#[cfg_attr(
    windows,
    allow(dead_code, reason = "Windows lets a frame finish before its restore")
)]
pub(crate) fn set_in_flight(on: bool) {
    IN_FLIGHT.store(on, Ordering::SeqCst);
}

/// The rows a restore moves up now: none while a write is under way.
/// Async-signal-safe: atomic loads.
pub(crate) fn rows_to_move_up() -> u16 {
    if IN_FLIGHT.load(Ordering::SeqCst) {
        0
    } else {
        ROWS_ABOVE.load(Ordering::SeqCst)
    }
}

/// Room for the longest restore sequence with its move up.
pub(crate) const RESTORE_MOST: usize = 128;

/// The restore sequence for the terminal's state now ([`RESTORE_BEGIN`]
/// says what it is), written into `buffer`, with a move up of `rows` before
/// its erase, to the live region's first row. Async-signal-safe: one atomic
/// load, and copies into a buffer on the caller's stack.
pub(crate) fn restore(buffer: &mut [u8; RESTORE_MOST], rows: u16) -> &[u8] {
    fn push(buffer: &mut [u8; RESTORE_MOST], length: &mut usize, bytes: &[u8]) {
        for &byte in bytes {
            if *length < RESTORE_MOST {
                buffer[*length] = byte;
                *length += 1;
            }
        }
    }
    let bits = PROTOCOLS.load(Ordering::SeqCst);
    let mut length = 0;
    push(buffer, &mut length, RESTORE_BEGIN);
    if bits & ALTERNATE != 0 {
        push(buffer, &mut length, RESTORE_MOUSE_OFF);
        // The alternate screen's flags, before it is left.
        if bits & KEYBOARD != 0 {
            push(buffer, &mut length, RESTORE_KEYBOARD_POP);
        }
        push(buffer, &mut length, RESTORE_ALTERNATE_OFF);
    }
    if rows > 0 {
        // `ESC [ n A`, its number written out by hand: nothing here may
        // allocate.
        let mut digits = [0u8; 5];
        let mut count = 0;
        let mut left = rows;
        while left > 0 && count < digits.len() {
            digits[count] = b'0' + u8::try_from(left % 10).unwrap_or(0);
            left /= 10;
            count += 1;
        }
        push(buffer, &mut length, b"\x1b[");
        while count > 0 {
            count -= 1;
            push(buffer, &mut length, &[digits[count]]);
        }
        push(buffer, &mut length, b"A");
    }
    push(buffer, &mut length, ERASE);
    // The main screen's flags, only where the program pushed them.
    if bits & MAIN_KEYBOARD != 0 {
        push(buffer, &mut length, RESTORE_KEYBOARD_POP);
    }
    if bits & WIN32 != 0 {
        push(buffer, &mut length, RESTORE_WIN32_OFF);
    }
    push(buffer, &mut length, RESTORE_END);
    &buffer[..length]
}

fn set(bit: u8, on: bool) {
    if on {
        PROTOCOLS.fetch_or(bit, Ordering::SeqCst);
    } else {
        PROTOCOLS.fetch_and(!bit, Ordering::SeqCst);
    }
}

/// Records whether the enhanced keyboard protocol is on.
pub(crate) fn set_keyboard(on: bool) {
    set(KEYBOARD, on);
}

/// Records whether the main screen's stack holds the program's keyboard
/// flags.
pub(crate) fn set_main_keyboard(on: bool) {
    set(MAIN_KEYBOARD, on);
}

/// Records whether the win32 input mode is on.
pub(crate) fn set_win32(on: bool) {
    set(WIN32, on);
}

/// Records whether the alternate screen is on.
pub(crate) fn set_alternate(on: bool) {
    set(ALTERNATE, on);
}

/// Whether a signal that ends the program is handed to it (since step 23):
/// the terminal is put back at once, as always, and then the program is
/// woken to end itself, with a watchdog, instead of being ended there.
static HAND_OFF: AtomicBool = AtomicBool::new(false);
/// The exit code of the signal handed over (`128 +` its number), 0 while
/// none has come.
static ENDED_BY: AtomicU8 = AtomicU8::new(0);
/// Set once the program has done what the signal asked, for Windows's
/// handler, which waits for it.
static ENDED: AtomicBool = AtomicBool::new(false);

/// Hands the signals that end the program to it from now on.
pub(crate) fn set_hand_off(on: bool) {
    HAND_OFF.store(on, Ordering::SeqCst);
}

/// Whether the signals that end the program are handed to it.
#[cfg_attr(
    windows,
    allow(dead_code, reason = "Unix keeps its handlers past leaving")
)]
pub(crate) fn hand_off() -> bool {
    HAND_OFF.load(Ordering::SeqCst)
}

/// Takes a signal for the program, `code` its exit code: `true` for the
/// first, which the program is woken for; `false` when the hand-off is off
/// or a signal came before, and this one ends the process at once.
/// Async-signal-safe: atomics alone.
pub(crate) fn hand_over(code: u8) -> bool {
    HAND_OFF.load(Ordering::SeqCst)
        && ENDED_BY
            .compare_exchange(0, code, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
}

/// Set when the terminal went away under a read (it was closed, or the
/// connection dropped) and that was taken as SIGHUP's end: the SIGHUP that
/// follows a hang-up is the same end, not a second signal.
static HUNG_UP: AtomicBool = AtomicBool::new(false);

/// The terminal went away under a read while the program was already
/// ending for a signal handed over: nothing changes in that end, but the
/// SIGHUP that follows the hang-up is now that same end, not a second
/// signal that kills the program part-way through it (since the third pull
/// request of step 23's review fixes: a SIGTERM's end, then the window
/// closed). A second signal a person sends still ends it at once.
pub(crate) fn hung_up_while_ending() {
    if HAND_OFF.load(Ordering::SeqCst) {
        HUNG_UP.store(true, Ordering::SeqCst);
    }
}

/// The terminal went away under a read. While signals are handed to the
/// program that is SIGHUP's end, exit code 129 (since the review of step
/// 23). `true` when the program is to end now (for this, or for a signal
/// that came before it); `false` when nothing is handed over, and the
/// read's error stands.
pub(crate) fn hang_up() -> bool {
    if !HAND_OFF.load(Ordering::SeqCst) {
        return false;
    }
    // Said before the hand-over: a SIGHUP in between finds it.
    HUNG_UP.store(true, Ordering::SeqCst);
    hand_over(129) || ended_by().is_some()
}

/// Whether a signal of exit code `code` is a hang-up the program is already
/// ending for: a SIGHUP (or a console's close) after one was handed over,
/// or after the terminal went away under a read. The kernel and shells send
/// it more than once (to the session's leader, then to the jobs), and in
/// either order with the read's end of input, so it is never a second
/// signal that ends the process at once. Async-signal-safe: atomics alone.
pub(crate) fn hung_up_already(code: u8) -> bool {
    code == 129
        && (ended_by() == Some(129) || (HUNG_UP.load(Ordering::SeqCst) && ended_by().is_some()))
}

/// The exit code of the signal handed over, once one has come.
pub(crate) fn ended_by() -> Option<u8> {
    match ENDED_BY.load(Ordering::SeqCst) {
        0 => None,
        code => Some(code),
    }
}

/// The program has done what the signal asked.
pub(crate) fn mark_ended() {
    ENDED.store(true, Ordering::SeqCst);
}

/// Whether the program has.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only Windows's handler waits")
)]
pub(crate) fn has_ended() -> bool {
    ENDED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wait to a deadline takes what is left of it, rounded up to the
    /// millisecond; nothing left is no wait, and no deadline no end (the
    /// review of step 25).
    #[test]
    fn a_wait_to_a_deadline_takes_what_is_left_of_it() {
        let now = std::time::Instant::now();
        let ms = std::time::Duration::from_micros;
        assert_eq!(millis_left(None, now), u32::MAX);
        assert_eq!(millis_left(Some(now + ms(10_400)), now), 11);
        assert_eq!(millis_left(Some(now + ms(10_400)), now + ms(4_000)), 7);
        assert_eq!(millis_left(Some(now), now + ms(1_000)), 0);
        assert_eq!(
            millis_left(Some(now + std::time::Duration::from_secs(5_000_000)), now),
            u32::MAX - 1
        );
        // Exactly the most a wait counts is still an end.
        let most = std::time::Duration::from_millis(u64::from(u32::MAX));
        assert_eq!(millis_left(Some(now + most), now), u32::MAX - 1);
    }

    /// The console's records are read again while more wait, up to the
    /// bound a read on Unix has: a paste comes in one read, not 64 records
    /// at a time (the review of step 25's second pull request).
    #[test]
    fn the_consoles_records_are_read_while_more_wait_up_to_the_bound() {
        assert!(reads_more(0, 1));
        assert!(reads_more(READ_BYTES - 1, 200));
        assert!(!reads_more(READ_BYTES, 200), "the bound");
        assert!(!reads_more(100, 0), "nothing more waits");
    }

    #[test]
    fn the_restore_sequence_turns_off_what_was_turned_on() {
        let has = |sequence: &[u8], part: &[u8]| sequence.windows(part.len()).any(|w| w == part);
        for (keyboard, win32) in [(false, false), (true, false), (false, true), (true, true)] {
            set_keyboard(keyboard);
            set_main_keyboard(keyboard);
            set_win32(win32);
            let sequence = restore_sequence();
            let sequence = sequence.as_slice();
            assert_eq!(has(sequence, b"\x1b[<u"), keyboard);
            assert_eq!(has(sequence, b"\x1b[?9001l"), win32);
            for needed in [
                &b"\x1b[?2026l"[..],
                b"\x1b[0m",
                b"\x1b[?1004l",
                b"\x1b[?2004l",
                b"\x1b[?7h",
                b"\x1b[?25h",
            ] {
                assert!(has(sequence, needed), "{needed:?}");
            }
        }
        set_keyboard(false);
        set_main_keyboard(false);
        set_win32(false);
    }

    /// Every restore leaves the terminal, as one with a stack of keyboard
    /// flags per screen follows it (kitty, ghostty, the test kit's
    /// `VTerm`), with none of the program's flags on either screen and the
    /// entry of the shell it runs under still there, the mouse off and the
    /// alternate screen left, whatever was on: the flags pushed on the main
    /// screen and on the alternate one, only on the alternate one (a reply
    /// that came while it was shown), or nowhere (a push the terminal
    /// refused), and the win32 input mode on or off (the fixes of step 25's
    /// review; the mouse off under the keyboard protocol and the win32 input
    /// mode had no test, and the second round found the main screen popped
    /// where the program never pushed).
    #[test]
    fn the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen() {
        use cineris_testkit::vterm::VTerm;

        for keyboard in [false, true] {
            for win32 in [false, true] {
                for alternate in [false, true] {
                    for on_main in [false, true] {
                        if on_main && !keyboard {
                            continue;
                        }
                        // The shell's own entry, pushed before the program.
                        let mut state = b"\x1b[>1u\x1b[?2004h\x1b[?1004h\x1b[?7l".to_vec();
                        if on_main {
                            state.extend_from_slice(b"\x1b[>3u");
                        }
                        if win32 {
                            state.extend_from_slice(b"\x1b[?9001h");
                        }
                        if alternate {
                            state.extend_from_slice(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h");
                            if keyboard {
                                state.extend_from_slice(b"\x1b[>3u");
                            }
                        }
                        let mut term = VTerm::new(40, 10);
                        term.feed(&state);
                        let bits = [
                            (keyboard, KEYBOARD),
                            (win32, WIN32),
                            (alternate, ALTERNATE),
                            (on_main, MAIN_KEYBOARD),
                        ]
                        .iter()
                        .filter(|(on, _)| *on)
                        .fold(0, |bits, (_, bit)| bits | bit);
                        PROTOCOLS.store(bits, Ordering::SeqCst);
                        let mut buffer = [0u8; RESTORE_MOST];
                        term.feed(restore(&mut buffer, 2));
                        let case = format!(
                            "keyboard {keyboard}, win32 {win32}, alternate {alternate}, pushed \
                             on the main screen {on_main}: {:?}",
                            String::from_utf8_lossy(&restore_sequence())
                        );
                        let modes = term.modes();
                        assert!(!modes.alternate, "the alternate screen is shown; {case}");
                        assert!(
                            !modes.mouse && !modes.mouse_sgr,
                            "the mouse is reported; {case}"
                        );
                        assert!(!modes.win32_input, "the win32 input mode is on; {case}");
                        assert_eq!(
                            term.main_keyboard(),
                            [1],
                            "the main screen's flags, the shell's first; {case}"
                        );
                        assert!(
                            term.alternate_keyboard().is_empty(),
                            "the alternate screen's flags {:?}; {case}",
                            term.alternate_keyboard()
                        );
                    }
                }
            }
        }
        PROTOCOLS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn the_restore_moves_up_to_the_live_regions_first_row_before_it_erases() {
        let mut buffer = [0u8; RESTORE_MOST];
        assert_eq!(restore(&mut buffer, 0), restore_sequence().as_slice());
        for (rows, up) in [
            (1, &b"\x1b[1A"[..]),
            (12, b"\x1b[12A"),
            (40_000, b"\x1b[40000A"),
        ] {
            let sequence = restore(&mut buffer, rows).to_vec();
            let at = sequence
                .windows(ERASE.len())
                .position(|window| window == ERASE)
                .unwrap();
            assert!(sequence[..at].ends_with(up), "{rows}: {sequence:?}");
            let mut without = sequence[..at - up.len()].to_vec();
            without.extend_from_slice(&sequence[at..]);
            assert_eq!(without, restore_sequence(), "{rows}");
        }
    }

    /// While a frame is being written the cursor is anywhere in it: a
    /// restore then moves up nothing, and the write sends it again with the
    /// frame's own count once it has landed.
    #[test]
    fn a_restore_during_a_write_moves_up_nothing() {
        set_rows_above(3);
        assert_eq!(rows_to_move_up(), 3);
        set_in_flight(true);
        assert_eq!(rows_to_move_up(), 0);
        set_in_flight(false);
        assert_eq!(rows_to_move_up(), 3);
        set_rows_above(0);
    }
}
