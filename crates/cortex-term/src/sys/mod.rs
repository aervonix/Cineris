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
/// The alternate screen, while the viewer is open (since step 23).
const ALTERNATE: u8 = 4;

/// What puts the terminal's own state back after a panic, a signal or a
/// console event, when the presenter could not finish: ends a frame that was
/// cut short, resets the style, leaves the alternate screen if it is on (the
/// main screen comes back as it was), erases the live region from its first
/// row down (the cursor is moved up the region's rows above it first, by
/// [`restore`], so that a message printed next is not mixed into it), turns
/// off what was turned on, and turns automatic wrap back on. The raw-mode
/// or console-mode half is the system's. One for each set of keyboard
/// reporting and screen that may be on, indexed by [`PROTOCOLS`].
const RESTORE: [&[u8]; 8] = [
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\r\x1b[J\x1b[<u\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1049l\r\x1b[J\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1049l\r\x1b[J\x1b[<u\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1049l\r\x1b[J\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
    b"\x1b[?2026l\x1b[0m\x1b[?1049l\r\x1b[J\x1b[<u\x1b[?9001l\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h",
];

/// The restore sequence for the terminal's state now, without the move up
/// to the live region's first row ([`restore`] adds it). Async-signal-safe:
/// one atomic load.
pub(crate) fn restore_sequence() -> &'static [u8] {
    RESTORE[usize::from(PROTOCOLS.load(Ordering::SeqCst) & 7)]
}

/// Where in every restore sequence the erase of the live region begins.
const ERASE: &[u8] = b"\r\x1b[J";

/// How many rows of the live region are above the cursor, as last drawn:
/// the emergency restore moves up that many before it erases, so that the
/// region's top rows (the input box's top border, the text that streamed)
/// do not stay on the screen (since the review of step 23).
static ROWS_ABOVE: AtomicU16 = AtomicU16::new(0);

/// Records how many rows of the live region are above the cursor.
pub(crate) fn set_rows_above(rows: usize) {
    ROWS_ABOVE.store(u16::try_from(rows).unwrap_or(u16::MAX), Ordering::SeqCst);
}

/// Room for the longest restore sequence with its move up.
pub(crate) const RESTORE_MOST: usize = 128;

/// The restore sequence for the terminal's state now, written into
/// `buffer`: [`restore_sequence`], with the move up to the live region's
/// first row before its erase. Async-signal-safe: atomic loads, and copies
/// into a buffer on the caller's stack.
pub(crate) fn restore(buffer: &mut [u8; RESTORE_MOST]) -> &[u8] {
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
    let rows = ROWS_ABOVE.load(Ordering::SeqCst);
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

/// Whether a signal of exit code `code` is the one a hang-up has already
/// ended the program for. Async-signal-safe: atomics alone.
pub(crate) fn hung_up_already(code: u8) -> bool {
    code == 129 && HUNG_UP.load(Ordering::SeqCst) && ended_by().is_some()
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

    #[test]
    fn the_restore_moves_up_to_the_live_regions_first_row_before_it_erases() {
        let mut buffer = [0u8; RESTORE_MOST];
        set_rows_above(0);
        assert_eq!(restore(&mut buffer), restore_sequence());
        for (rows, up) in [
            (1, &b"\x1b[1A"[..]),
            (12, b"\x1b[12A"),
            (40_000, b"\x1b[40000A"),
        ] {
            set_rows_above(rows);
            let sequence = restore(&mut buffer).to_vec();
            let at = sequence
                .windows(ERASE.len())
                .position(|window| window == ERASE)
                .unwrap();
            assert!(sequence[..at].ends_with(up), "{rows}: {sequence:?}");
            let mut without = sequence[..at - up.len()].to_vec();
            without.extend_from_slice(&sequence[at..]);
            assert_eq!(without, restore_sequence(), "{rows}");
        }
        set_rows_above(0);
    }
}
