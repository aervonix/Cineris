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

/// Which keyboard reporting was turned on, and whether the alternate screen
/// is, so that putting the terminal back turns them off: [`KEYBOARD`],
/// [`WIN32`] and [`ALTERNATE`] as bits.
static PROTOCOLS: AtomicU8 = AtomicU8::new(0);
/// The enhanced keyboard protocol.
const KEYBOARD: u8 = 1;
/// Windows Terminal's win32 input mode.
const WIN32: u8 = 2;
/// The alternate screen, while the viewer or the full-screen presenter is
/// shown (since step 23). Mouse reporting is on only with it (since step
/// 25), so its restore turns that off too.
const ALTERNATE: u8 = 4;

/// What puts the terminal's own state back after a panic, a signal or a
/// console event, when the presenter could not finish: ends a frame that was
/// cut short, resets the style, leaves the alternate screen if it is on (the
/// main screen comes back as it was), erases the live region from its first
/// row down (the cursor is moved up the region's rows above it first, by
/// [`restore`], so that a message printed next is not mixed into it), turns
/// off what was turned on, and turns automatic wrap back on. With the
/// alternate screen on it turns mouse reporting off first (modes 1006 and
/// 1000), which is on only with it (since step 25), and pops the keyboard
/// protocol's flags before it leaves the alternate screen as well as after:
/// each screen has a stack of its own (since the fixes of step 25's
/// review). A pop of a stack that holds nothing does nothing. The raw-mode
/// or console-mode half is the system's. One for each set of keyboard
/// reporting and screen that may be on, indexed by [`PROTOCOLS`].
const RESTORE: [&[u8]; 8] = [
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[<u\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[?1049l\r\x1b[J\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[<u\x1b[?1049l\r\x1b[J\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[?1049l\r\x1b[J\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1006l\x1b[?1000l\x1b[<u\x1b[?1049l\r\x1b[J\x1b[<u\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
];

/// The restore sequence for the terminal's state now, without the move up
/// to the live region's first row ([`restore`] adds it). Async-signal-safe:
/// one atomic load.
pub(crate) fn restore_sequence() -> &'static [u8] {
    RESTORE[usize::from(PROTOCOLS.load(Ordering::SeqCst) & 7)]
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

/// The restore sequence for the terminal's state now, written into
/// `buffer`: [`restore_sequence`], with a move up of `rows` before its
/// erase, to the live region's first row. Async-signal-safe: atomic loads,
/// and copies into a buffer on the caller's stack.
pub(crate) fn restore(buffer: &mut [u8; RESTORE_MOST], rows: u16) -> &[u8] {
    fn push(buffer: &mut [u8; RESTORE_MOST], length: &mut usize, bytes: &[u8]) {
        for &byte in bytes {
            if *length < RESTORE_MOST {
                buffer[*length] = byte;
                *length += 1;
            }
        }
    }
    let sequence = restore_sequence();
    let at = sequence
        .windows(ERASE.len())
        .position(|window| window == ERASE)
        .unwrap_or(sequence.len());
    let mut length = 0;
    push(buffer, &mut length, &sequence[..at]);
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
    push(buffer, &mut length, &sequence[at..]);
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
            set_win32(win32);
            let sequence = restore_sequence();
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
        set_win32(false);
    }

    /// Every restore leaves the terminal, as one with a stack of keyboard
    /// flags per screen follows it (kitty, ghostty, the test kit's
    /// `VTerm`), with no flags pushed on either screen, the mouse off and
    /// the alternate screen left, whatever was on: the flags pushed on the
    /// main screen and on the alternate one, or only on the alternate one
    /// (a reply that came while it was shown), and with the win32 input
    /// mode on too (the fixes of step 25's review; `RESTORE[5..=7]`'s mouse
    /// off had no test).
    #[test]
    fn the_restore_leaves_no_keyboard_flags_and_no_mouse_on_either_screen() {
        use cineris_testkit::vterm::VTerm;

        for bits in 0..8 {
            let (keyboard, win32, alternate) = (
                bits & KEYBOARD != 0,
                bits & WIN32 != 0,
                bits & ALTERNATE != 0,
            );
            // Where the flags were pushed: on the main screen and on the
            // alternate one shown after it, or only on the alternate one.
            for on_main in [true, false] {
                if !on_main && !(keyboard && alternate) {
                    continue;
                }
                let mut term = VTerm::new(40, 10);
                let mut state = b"\x1b[?2004h\x1b[?1004h\x1b[?7l".to_vec();
                if keyboard && on_main {
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
                term.feed(&state);
                PROTOCOLS.store(bits, Ordering::SeqCst);
                let mut buffer = [0u8; RESTORE_MOST];
                term.feed(restore(&mut buffer, 2));
                let case = format!(
                    "keyboard {keyboard}, win32 {win32}, alternate {alternate}, pushed on the \
                     main screen {on_main}: {:?}",
                    String::from_utf8_lossy(restore_sequence())
                );
                let modes = term.modes();
                assert!(!modes.alternate, "the alternate screen is shown; {case}");
                assert!(
                    !modes.mouse && !modes.mouse_sgr,
                    "the mouse is reported; {case}"
                );
                assert!(!modes.win32_input, "the win32 input mode is on; {case}");
                assert!(
                    term.main_keyboard().is_empty(),
                    "the main screen's flags {:?}; {case}",
                    term.main_keyboard()
                );
                assert!(
                    term.alternate_keyboard().is_empty(),
                    "the alternate screen's flags {:?}; {case}",
                    term.alternate_keyboard()
                );
            }
        }
        PROTOCOLS.store(0, Ordering::SeqCst);
    }

    #[test]
    fn the_restore_moves_up_to_the_live_regions_first_row_before_it_erases() {
        let mut buffer = [0u8; RESTORE_MOST];
        assert_eq!(restore(&mut buffer, 0), restore_sequence());
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
