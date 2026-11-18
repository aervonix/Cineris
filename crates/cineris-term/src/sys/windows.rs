//! The Windows half of the terminal layer: the console's modes and code
//! pages, the window's size, waiting for input, and putting the console back
//! on every way out.
//!
//! The standard library can tell whether a handle is a console, and no more.
//! The console's modes, its code pages, its input records, its control
//! events and the window's size go through the Windows API here, and every
//! `unsafe` block says why it is sound.
//!
//! # Virtual-terminal processing
//!
//! The old console host shows every escape sequence as text unless a
//! program turns on `ENABLE_VIRTUAL_TERMINAL_PROCESSING`; Windows Terminal
//! turns it on by itself. [`enter`] turns it on for output, and turns on
//! virtual-terminal input, so that keys arrive as the same sequences a Linux
//! terminal sends and one decoder serves both systems. Every way out puts
//! back the modes and code pages it found: a normal exit ([`leave`]), a panic
//! and a console control event ([`emergency`]).

#![allow(
    unsafe_code,
    reason = "the console's modes, code pages, input records and control events are only \
              reachable through the Windows API; cineris-term is where such calls live"
)]

use std::io;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{FALSE, HANDLE, TRUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT, CTRL_C_EVENT, DISABLE_NEWLINE_AUTO_RETURN,
    ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    ENABLE_WINDOW_INPUT, GetConsoleCP, GetConsoleMode, GetConsoleOutputCP,
    GetConsoleScreenBufferInfo, GetNumberOfConsoleInputEvents, GetStdHandle, INPUT_RECORD,
    KEY_EVENT, MOUSE_EVENT, MOUSE_EVENT_RECORD, ReadConsoleInputW, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleCtrlHandler, SetConsoleMode, SetConsoleOutputCP,
    WINDOW_BUFFER_SIZE_EVENT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};

use super::{RESTORE_MOST, Ready, restore};

/// Held while a frame is written, and by [`emergency`] while it puts the
/// console back: the console's control handler runs on a thread of its
/// own, so a frame being written as it comes could otherwise land after the
/// restore, in a console that is no longer the program's (the review of
/// step 23). Unix needs none: its handler interrupts the writing thread, or
/// sends the restore again after a frame that raced it.
static WRITING: Mutex<()> = Mutex::new(());

/// The longest [`emergency`] waits for a frame being written to finish.
const WRITE_WAIT: Duration = Duration::from_millis(250);

/// UTF-8, as a console code page.
const UTF8: u32 = 65001;

/// The modes and code pages found at the first [`enter`], which every way
/// out puts back.
static INPUT_MODE: AtomicU32 = AtomicU32::new(0);
static OUTPUT_MODE: AtomicU32 = AtomicU32::new(0);
static INPUT_CP: AtomicU32 = AtomicU32::new(0);
static OUTPUT_CP: AtomicU32 = AtomicU32::new(0);
static SAVED: AtomicBool = AtomicBool::new(false);
/// Whether the console is in our modes now.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The event that wakes a wait. 0 until it exists.
static WAKE: AtomicIsize = AtomicIsize::new(0);
static WOKEN: AtomicBool = AtomicBool::new(false);
/// A resize seen in the console's input, not reported yet.
static RESIZED: AtomicBool = AtomicBool::new(false);
static HANDLER: AtomicBool = AtomicBool::new(false);
/// The first half of a character outside the Basic Multilingual Plane,
/// waiting for its second half in the next record.
static HIGH_SURROGATE: AtomicU32 = AtomicU32::new(0);

fn handle(which: u32) -> HANDLE {
    // SAFETY: `GetStdHandle` reads nothing but its argument.
    unsafe { GetStdHandle(which) }
}

fn console_mode(console: HANDLE) -> Option<u32> {
    let mut mode = 0;
    // SAFETY: `mode` is a local the call fills.
    (unsafe { GetConsoleMode(console, &raw mut mode) } != 0).then_some(mode)
}

fn set_mode(console: HANDLE, mode: u32) -> bool {
    // SAFETY: setting a mode reads nothing but the arguments.
    unsafe { SetConsoleMode(console, mode) != 0 }
}

/// Whether standard input and standard output are both a console.
pub(crate) fn is_terminal() -> bool {
    console_mode(handle(STD_INPUT_HANDLE)).is_some()
        && console_mode(handle(STD_OUTPUT_HANDLE)).is_some()
}

/// The output mode the terminal layer draws in.
fn our_output_mode(found: u32) -> u32 {
    found
        | ENABLE_PROCESSED_OUTPUT
        | ENABLE_VIRTUAL_TERMINAL_PROCESSING
        | DISABLE_NEWLINE_AUTO_RETURN
}

/// The input mode the terminal layer reads in: keys one at a time, not
/// echoed, Ctrl+C as a key, and as virtual-terminal sequences, with resizes
/// reported.
fn our_input_mode(found: u32) -> u32 {
    (found | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT)
        & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT | ENABLE_MOUSE_INPUT)
}

/// The buttons the last mouse record said were down, to tell a press from
/// a release.
static BUTTONS: AtomicU32 = AtomicU32::new(0);

/// A mouse event that came as a console record, written into `into` as the
/// SGR report (mode 1006) a terminal would have sent, so that one decoder
/// reads both. Moves with no button down are not reported.
fn push_mouse(event: &MOUSE_EVENT_RECORD, into: &mut Vec<u8>) {
    const MOVED: u32 = 0x1;
    const WHEELED: u32 = 0x4;
    const HWHEELED: u32 = 0x8;
    const SHIFT: u32 = 0x10;
    const ALT: u32 = 0x1 | 0x2;
    const CTRL: u32 = 0x4 | 0x8;
    let state = event.dwControlKeyState;
    let mut mods = 0;
    if state & SHIFT != 0 {
        mods |= 4;
    }
    if state & ALT != 0 {
        mods |= 8;
    }
    if state & CTRL != 0 {
        mods |= 16;
    }
    let col = i32::from(event.dwMousePosition.X).max(0) + 1;
    let row = i32::from(event.dwMousePosition.Y).max(0) + 1;
    let mut report = |code: u32, release: bool| {
        into.extend_from_slice(
            format!(
                "\x1b[<{};{col};{row}{}",
                code | mods,
                if release { 'm' } else { 'M' }
            )
            .as_bytes(),
        );
    };
    let buttons = event.dwButtonState;
    // The wheel's turn is the high word of the button state, signed.
    let turn = (buttons >> 16) as u16 as i16;
    if event.dwEventFlags & WHEELED != 0 {
        report(if turn > 0 { 64 } else { 65 }, false);
        return;
    }
    if event.dwEventFlags & HWHEELED != 0 {
        report(if turn > 0 { 67 } else { 66 }, false);
        return;
    }
    // Left, right and middle, as the console numbers them, and their codes.
    let held = buttons & 0x7;
    let before = BUTTONS.swap(held, Ordering::SeqCst);
    let codes = [(0x1, 0), (0x4, 1), (0x2, 2)];
    if event.dwEventFlags & MOVED != 0 {
        if let Some((_, code)) = codes.iter().find(|(bit, _)| held & bit != 0) {
            report(code + 32, false);
        }
        return;
    }
    for (bit, code) in codes {
        match (before & bit != 0, held & bit != 0) {
            (false, true) => report(code, false),
            (true, false) => report(code, true),
            _ => {}
        }
    }
}

/// Turns on virtual-terminal processing and input, UTF-8, and raw input.
pub(crate) fn enter() -> io::Result<()> {
    let (input, output) = (handle(STD_INPUT_HANDLE), handle(STD_OUTPUT_HANDLE));
    let (Some(input_mode), Some(output_mode)) = (console_mode(input), console_mode(output)) else {
        return Err(io::Error::other(
            "standard input and standard output must both be a console",
        ));
    };
    if !SAVED.swap(true, Ordering::SeqCst) {
        INPUT_MODE.store(input_mode, Ordering::SeqCst);
        OUTPUT_MODE.store(output_mode, Ordering::SeqCst);
        // SAFETY: both read nothing.
        INPUT_CP.store(unsafe { GetConsoleCP() }, Ordering::SeqCst);
        // SAFETY: as above.
        OUTPUT_CP.store(unsafe { GetConsoleOutputCP() }, Ordering::SeqCst);
    }
    wake_event()?;
    if !HANDLER.swap(true, Ordering::SeqCst) {
        // SAFETY: the handler is a function with the signature Windows
        // asks for, and stays for the life of the process.
        unsafe { SetConsoleCtrlHandler(Some(on_control), TRUE) };
    }
    ACTIVE.store(true, Ordering::SeqCst);
    // SAFETY: setting a code page reads nothing but its argument.
    unsafe {
        SetConsoleOutputCP(UTF8);
        SetConsoleCP(UTF8);
    }
    let saved_output = OUTPUT_MODE.load(Ordering::SeqCst);
    if !set_mode(output, our_output_mode(saved_output))
        // A console that refuses the line-feed flag may still take the rest.
        && !set_mode(
            output,
            our_output_mode(saved_output) & !DISABLE_NEWLINE_AUTO_RETURN,
        )
    {
        let error = io::Error::other(
            "this console cannot process escape sequences: virtual-terminal processing could \
             not be turned on",
        );
        leave();
        return Err(error);
    }
    if !set_mode(input, our_input_mode(INPUT_MODE.load(Ordering::SeqCst))) {
        let error = io::Error::last_os_error();
        leave();
        return Err(error);
    }
    Ok(())
}

/// Puts the modes and code pages back without writing anything. Safe to
/// call more than once.
pub(crate) fn leave() {
    // Marked off only once the modes are back: a control event in between
    // finds the console still the program's, and puts it back itself.
    if ACTIVE.load(Ordering::SeqCst) {
        restore_modes();
    }
    ACTIVE.store(false, Ordering::SeqCst);
}

/// Whether the console is in the program's modes.
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

fn restore_modes() {
    set_mode(handle(STD_INPUT_HANDLE), INPUT_MODE.load(Ordering::SeqCst));
    set_mode(
        handle(STD_OUTPUT_HANDLE),
        OUTPUT_MODE.load(Ordering::SeqCst),
    );
    // SAFETY: setting a code page reads nothing but its argument.
    unsafe {
        SetConsoleCP(INPUT_CP.load(Ordering::SeqCst));
        SetConsoleOutputCP(OUTPUT_CP.load(Ordering::SeqCst));
    }
}

/// Writes the restore sequence, while virtual-terminal processing is still
/// on, then puts the modes and code pages back.
///
/// A frame being written is let finish first, for at most [`WRITE_WAIT`]:
/// the restore then lands after it, and a frame written later is refused.
pub(crate) fn emergency() {
    let _writing = writing(Some(WRITE_WAIT));
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut buffer = [0u8; RESTORE_MOST];
    let _ = write_all(restore(&mut buffer, super::rows_to_move_up()));
    super::set_rows_above(0);
    restore_modes();
}

/// Takes [`WRITING`]: waiting as long as it takes (`None`), or at most
/// `most`, after which the caller goes on without it.
fn writing(most: Option<Duration>) -> Option<MutexGuard<'static, ()>> {
    let Some(most) = most else {
        return Some(
            WRITING
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    };
    let deadline = Instant::now() + most;
    loop {
        match WRITING.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => std::thread::yield_now(),
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

unsafe extern "system" fn on_control(event: u32) -> windows_sys::core::BOOL {
    if event == CTRL_C_EVENT && passing_over() {
        // Handled: the program waiting for a child goes on.
        return TRUE;
    }
    emergency();
    // Handed to the program, which ends itself: Windows ends the process
    // when this returns from a close, log-off or shutdown event, so it
    // waits for the program, at most 3 s, on its own thread.
    let code = match event {
        CTRL_C_EVENT => 130,
        CTRL_BREAK_EVENT => 143,
        _ => 129,
    };
    // The close that follows a console taken as gone already (`gone`) is
    // that same end, waited for alike.
    if super::hand_over(code) || super::hung_up_already(code) {
        wake();
        let ended = ended_event();
        if ended != 0 {
            // SAFETY: the event lives for the life of the process.
            unsafe { WaitForSingleObject(ended as HANDLE, 3000) };
        }
        return if super::has_ended() { TRUE } else { FALSE };
    }
    // Not handled: the next handler, in the end the system's, ends the
    // process as the event asks.
    FALSE
}

/// The event the program sets once it has ended what a signal asked.
static ENDED_EVENT: AtomicUsize = AtomicUsize::new(0);

fn ended_event() -> usize {
    ENDED_EVENT.load(Ordering::SeqCst)
}

/// Makes the event [`on_control`] waits on, once: before the handler that
/// waits is installed.
pub(crate) fn prepare_hand_off() {
    if ENDED_EVENT.load(Ordering::SeqCst) != 0 {
        return;
    }
    // SAFETY: an unnamed manual-reset event, not signalled, default
    // security.
    let event = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, ptr::null()) };
    if !event.is_null() {
        ENDED_EVENT.store(event as usize, Ordering::SeqCst);
    }
}

/// The program has ended what the signal asked: the handler's wait ends.
pub(crate) fn signal_ended() {
    let event = ENDED_EVENT.load(Ordering::SeqCst);
    if event != 0 {
        // SAFETY: the event lives for the life of the process.
        unsafe { SetEvent(event as HANDLE) };
    }
}

fn wake_event() -> io::Result<HANDLE> {
    let existing = WAKE.load(Ordering::SeqCst);
    if existing != 0 {
        return Ok(existing as HANDLE);
    }
    // SAFETY: an unnamed auto-reset event, not signalled, default security.
    let event = unsafe { CreateEventW(ptr::null(), FALSE, FALSE, ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    match WAKE.compare_exchange(0, event as isize, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => Ok(event),
        // Another thread made one first; this one is never closed, which is
        // one handle for the life of the process.
        Err(theirs) => Ok(theirs as HANDLE),
    }
}

/// Wakes [`wait`] from another thread.
pub(crate) fn wake() {
    WOKEN.store(true, Ordering::SeqCst);
    let event = WAKE.load(Ordering::SeqCst);
    if event != 0 {
        // SAFETY: the event lives for the life of the process.
        unsafe { SetEvent(event as HANDLE) };
    }
}

/// Whether [`on_control`] passes over Ctrl+C now, and whether it did
/// before the guard that set it.
pub(crate) type Interrupts = bool;

/// Ctrl+C is passed over by [`on_control`] while this is set.
static PASSING_OVER: AtomicBool = AtomicBool::new(false);

/// Makes the process pass over Ctrl+C: the layer's own handler answers it
/// as handled. Not the console's "ignore" attribute, which a program started
/// meanwhile would inherit; a handler is not inherited. Ctrl+Break still
/// ends the process.
pub(crate) fn ignore_interrupts() -> io::Result<Interrupts> {
    if !HANDLER.swap(true, Ordering::SeqCst) {
        // SAFETY: the handler is a function with the signature Windows
        // asks for, and stays for the life of the process.
        if unsafe { SetConsoleCtrlHandler(Some(on_control), TRUE) } == 0 {
            HANDLER.store(false, Ordering::SeqCst);
            return Err(io::Error::last_os_error());
        }
    }
    START.get_or_init(Instant::now);
    Ok(PASSING_OVER.swap(true, Ordering::SeqCst))
}

/// Puts back what [`ignore_interrupts`] found.
///
/// The console runs a handler on a thread it starts for the event, which on
/// a loaded machine can run after the guard is gone: so a Ctrl+C that comes
/// within [`GRACE`] of the guard's end is passed over too. By then the
/// interface has entered the terminal again, where Ctrl+C is a key and not
/// an event.
pub(crate) fn restore_interrupts(previous: &Interrupts) {
    let since = START.get_or_init(Instant::now).elapsed();
    ENDED_AT.store(
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX),
        Ordering::SeqCst,
    );
    PASSING_OVER.store(*previous, Ordering::SeqCst);
}

/// How long after a guard ends a Ctrl+C is still taken as sent while it
/// lived.
const GRACE: Duration = Duration::from_secs(1);
/// When the process first used a guard, for [`ENDED_AT`].
static START: OnceLock<Instant> = OnceLock::new();
/// When the last guard ended, in milliseconds after [`START`]; `u64::MAX`
/// when none has.
static ENDED_AT: AtomicU64 = AtomicU64::new(u64::MAX);

/// Whether a Ctrl+C now is passed over: a guard lives, or one has just
/// ended.
fn passing_over() -> bool {
    if PASSING_OVER.load(Ordering::SeqCst) {
        return true;
    }
    let ended = ENDED_AT.load(Ordering::SeqCst);
    let Some(start) = START.get() else {
        return false;
    };
    let now = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    ended != u64::MAX && now.saturating_sub(ended) < u64::try_from(GRACE.as_millis()).unwrap_or(0)
}

/// Stops the program, which Windows has no way to do: nothing happens.
pub(crate) fn suspend() {}

/// The window's size, columns then rows.
pub(crate) fn size() -> io::Result<(u16, u16)> {
    // SAFETY: a zeroed structure of plain integers, which the call fills.
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { GetConsoleScreenBufferInfo(handle(STD_OUTPUT_HANDLE), &raw mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let window = info.srWindow;
    let cols = u16::try_from(i32::from(window.Right) - i32::from(window.Left) + 1).unwrap_or(1);
    let rows = u16::try_from(i32::from(window.Bottom) - i32::from(window.Top) + 1).unwrap_or(1);
    Ok((cols, rows))
}

/// Writes all of `bytes` to standard output: one call, unless the console
/// takes only part of it.
///
/// Once the console has been put back (a panic on another thread, a control
/// event), nothing more is written: a frame would turn modes on again in a
/// console that is no longer the program's, or show as text in one whose
/// virtual-terminal processing is off again.
pub(crate) fn write(bytes: &[u8]) -> io::Result<()> {
    let _writing = writing(None);
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(io::Error::other("the console has been put back"));
    }
    write_all(bytes)
}

fn write_all(bytes: &[u8]) -> io::Result<()> {
    let output = handle(STD_OUTPUT_HANDLE);
    let mut rest = bytes;
    while !rest.is_empty() {
        let length = u32::try_from(rest.len()).unwrap_or(u32::MAX);
        let mut written = 0;
        // SAFETY: the pointer and length are those of a live slice, and
        // `written` is a local the call fills.
        let ok = unsafe {
            WriteFile(
                output,
                rest.as_ptr(),
                length,
                &raw mut written,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        rest = &rest[usize::try_from(written).unwrap_or(rest.len())..];
    }
    Ok(())
}

/// Waits for input, a resize or a wake, for at most `timeout` (`None`: as
/// long as it takes). Input is added to `into` as UTF-8.
pub(crate) fn wait(timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
    let input = handle(STD_INPUT_HANDLE);
    let wake = wake_event()?;
    // A deadline, not a wait's length: a pass that brought nothing to report
    // (mouse moves with no button, a key's release) waits only for what is
    // left of it (the review of step 25).
    let deadline = timeout.and_then(|timeout| std::time::Instant::now().checked_add(timeout));
    let handles = [input, wake];
    loop {
        if RESIZED.swap(false, Ordering::SeqCst) {
            return Ok(Ready::Resize);
        }
        if WOKEN.swap(false, Ordering::SeqCst) {
            return Ok(Ready::Woken);
        }
        let millis = super::millis_left(deadline, std::time::Instant::now());
        // SAFETY: an array of two live handles, and its length.
        let result = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), FALSE, millis) };
        if result == WAIT_OBJECT_0 + 1 {
            continue;
        }
        if result != WAIT_OBJECT_0 {
            if result == 0x102 {
                // WAIT_TIMEOUT
                return Ok(if WOKEN.swap(false, Ordering::SeqCst) {
                    Ready::Woken
                } else {
                    Ready::Timeout
                });
            }
            return gone(io::Error::last_os_error());
        }
        let mut count = 0;
        // SAFETY: `count` is a local the call fills.
        if unsafe { GetNumberOfConsoleInputEvents(input, &raw mut count) } == 0 {
            return gone(io::Error::last_os_error());
        }
        if count == 0 {
            continue;
        }
        // SAFETY: zeroed records are plain data, filled by the call.
        let mut records: [INPUT_RECORD; 64] = unsafe { std::mem::zeroed() };
        let mut read = 0;
        // SAFETY: the array and its length; `read` is a local the call
        // fills.
        let ok = unsafe {
            ReadConsoleInputW(
                input,
                records.as_mut_ptr(),
                u32::try_from(records.len()).unwrap_or(0),
                &raw mut read,
            )
        };
        if ok == 0 {
            return gone(io::Error::last_os_error());
        }

        let before = into.len();
        for record in &records[..usize::try_from(read).unwrap_or(0)] {
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    // SAFETY: the record says it holds a key event.
                    let key = unsafe { record.Event.KeyEvent };
                    // SAFETY: with virtual-terminal input every key event
                    // carries a character; the union holds it as UTF-16.
                    let unit = unsafe { key.uChar.UnicodeChar };
                    if key.bKeyDown != 0 && unit != 0 {
                        for _ in 0..key.wRepeatCount.max(1) {
                            push_unit(unit, into);
                        }
                    }
                }
                // Kept until it is reported: a read that also brought keys
                // reports them first, and the resize on the next wait.
                WINDOW_BUFFER_SIZE_EVENT => RESIZED.store(true, Ordering::SeqCst),
                // The console's mouse input is never turned on (since step
                // 25's follow-up: with QuickEdit off, a pseudo-console asks
                // the terminal for the mouse, whose reports then come as
                // typed keys), so none comes; a record is still read as the
                // report it stands for, for step 33
                // (outstanding/the-mouse-under-windows-terminal.md).
                MOUSE_EVENT => {
                    // SAFETY: the record says it holds a mouse event.
                    let mouse = unsafe { record.Event.MouseEvent };
                    push_mouse(&mouse, into);
                }
                _ => {}
            }
        }
        if into.len() > before {
            return Ok(Ready::Input);
        }
        if RESIZED.swap(false, Ordering::SeqCst) {
            return Ok(Ready::Resize);
        }
    }
}

/// The console could not be read, `error` saying why. A signal already
/// handed over is what the program ends for; otherwise, while signals are
/// handed over, a console that has gone ends the program as its close
/// would (code 129), with a watchdog of its own, since no close event may
/// follow (the review of step 23, the twin of Unix's hang-up). The error
/// stands when nothing is handed over.
fn gone(error: io::Error) -> io::Result<Ready> {
    if super::ended_by().is_some() {
        super::hung_up_while_ending();
        return Ok(Ready::Woken);
    }
    if super::hang_up() {
        emergency();
        // The watchdog: the process ends 3 s on unless the program has
        // ended by then.
        let watchdog = std::thread::Builder::new()
            .name("cineris-watchdog".to_owned())
            .spawn(|| {
                let ended = ended_event();
                if ended != 0 {
                    // SAFETY: the event lives for the life of the process.
                    unsafe { WaitForSingleObject(ended as HANDLE, 3000) };
                } else {
                    // No event to wait on: the watchdog's time all the same.
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "the watchdog's own thread waits its time out"
                    )]
                    std::thread::sleep(Duration::from_secs(3));
                }
                if !super::has_ended() {
                    #[expect(clippy::exit, reason = "the watchdog ends a program that did not")]
                    std::process::exit(129);
                }
            });
        drop(watchdog);
        return Ok(Ready::Woken);
    }
    Err(error)
}

/// Puts back what was kept for signals handed to the program: nothing on
/// Windows, whose console handler stays for the life of the process.
pub(crate) fn release_signals() {}

/// Adds one UTF-16 unit of a key to `into` as UTF-8, joining the two halves
/// of a character outside the Basic Multilingual Plane.
fn push_unit(unit: u16, into: &mut Vec<u8>) {
    let mut buffer = [0; 4];
    match unit {
        0xd800..=0xdbff => HIGH_SURROGATE.store(u32::from(unit), Ordering::SeqCst),
        0xdc00..=0xdfff => {
            let high = HIGH_SURROGATE.swap(0, Ordering::SeqCst);
            let c = u16::try_from(high)
                .ok()
                .filter(|high| *high != 0)
                .and_then(|high| char::decode_utf16([high, unit]).next())
                .and_then(Result::ok)
                .unwrap_or('\u{fffd}');
            into.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
        }
        _ => {
            let c = char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}');
            into.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
        }
    }
}

/// A description of the console's modes and code pages, for tests that
/// check they were put back.
#[cfg(feature = "pty")]
pub(crate) fn modes() -> io::Result<String> {
    let input = console_mode(handle(STD_INPUT_HANDLE)).ok_or_else(io::Error::last_os_error)?;
    let output = console_mode(handle(STD_OUTPUT_HANDLE)).ok_or_else(io::Error::last_os_error)?;
    // SAFETY: both read nothing.
    let (input_cp, output_cp) = unsafe { (GetConsoleCP(), GetConsoleOutputCP()) };
    Ok(format!(
        "input={input:x} output={output:x} input-cp={input_cp} output-cp={output_cp}"
    ))
}

/// Turns virtual-terminal processing of this program's console output on
/// or off, for a test that plays the old console host, where it starts off.
#[cfg(feature = "pty")]
pub(crate) fn set_vt_output(on: bool) -> io::Result<()> {
    let output = handle(STD_OUTPUT_HANDLE);
    let mode = console_mode(output).ok_or_else(io::Error::last_os_error)?;
    let mode = if on {
        mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING
    } else {
        mode & !ENABLE_VIRTUAL_TERMINAL_PROCESSING
    };
    if set_mode(output, mode) {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Pseudo-consoles, for tests.
#[cfg(feature = "pty")]
pub(crate) mod pty {
    #![allow(
        clippy::disallowed_types,
        reason = "test support: it reads and writes a pseudo-console's pipes as files"
    )]

    use std::fs::File;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::ptr;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Console::{
        COORD, CTRL_BREAK_EVENT, ClosePseudoConsole, CreatePseudoConsole, GenerateConsoleCtrlEvent,
        HPCON, ResizePseudoConsole,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        GetExitCodeProcess, InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject,
    };

    use super::FALSE;

    /// A program running in a pseudo-console of its own.
    #[derive(Debug)]
    pub(crate) struct Pty {
        /// The pseudo-console, or 0 once [`Pty::hang_up`] has closed it.
        console: HPCON,
        input: OwnedHandle,
        output: OwnedHandle,
        process: OwnedHandle,
        pid: u32,
    }

    // SAFETY: a pseudo-console handle may be used from any thread.
    unsafe impl Send for Pty {}

    fn last() -> io::Error {
        io::Error::last_os_error()
    }

    fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
        let (mut read, mut write): (HANDLE, HANDLE) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: two locals the call fills, default security.
        if unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) } == 0 {
            return Err(last());
        }
        // SAFETY: both handles were just made, and are ours alone.
        Ok(unsafe {
            (
                OwnedHandle::from_raw_handle(read),
                OwnedHandle::from_raw_handle(write),
            )
        })
    }

    fn coord(cols: u16, rows: u16) -> COORD {
        COORD {
            X: i16::try_from(cols).unwrap_or(i16::MAX),
            Y: i16::try_from(rows).unwrap_or(i16::MAX),
        }
    }

    /// One argument as `CommandLineToArgvW` and the C runtime read it back.
    fn quote(argument: &str, line: &mut String) {
        if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
            line.push_str(argument);
            return;
        }
        line.push('"');
        let mut backslashes = 0;
        for c in argument.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    line.push('"');
                    backslashes = 0;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    line.push(c);
                    backslashes = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n('\\', backslashes * 2));
        line.push('"');
    }

    impl Pty {
        pub(crate) fn spawn(
            program: &str,
            arguments: &[String],
            env: &[(String, String)],
            cols: u16,
            rows: u16,
        ) -> io::Result<Self> {
            if !env.is_empty() {
                return Err(io::Error::other(
                    "a program in a pseudo-console takes its environment from the test",
                ));
            }
            let (console_input, input) = pipe()?;
            let (output, console_output) = pipe()?;
            let mut console: HPCON = 0;
            // SAFETY: the two pipe ends are live; the console duplicates
            // them, so ours are closed when this function returns.
            let result = unsafe {
                CreatePseudoConsole(
                    coord(cols, rows),
                    console_input.as_raw_handle(),
                    console_output.as_raw_handle(),
                    0,
                    &raw mut console,
                )
            };
            if result < 0 {
                return Err(io::Error::from_raw_os_error(result));
            }
            drop((console_input, console_output));

            let mut line = String::new();
            quote(program, &mut line);
            for argument in arguments {
                line.push(' ');
                quote(argument, &mut line);
            }
            let mut line: Vec<u16> = line.encode_utf16().chain([0]).collect();

            let mut size = 0;
            // SAFETY: asks how large a list of one attribute is; it fails
            // with that size, by design.
            unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
            let mut list = vec![0u64; size.div_ceil(8)];
            let attributes = list.as_mut_ptr().cast();
            // SAFETY: the buffer is at least `size` bytes, and aligned.
            if unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &raw mut size) } == 0 {
                let error = last();
                // SAFETY: the console was made above and nothing uses it.
                unsafe { ClosePseudoConsole(console) };
                return Err(error);
            }
            // SAFETY: the attribute's value is the console handle itself,
            // passed as a pointer-sized value, as Windows documents.
            let updated = unsafe {
                UpdateProcThreadAttribute(
                    attributes,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    console as *const core::ffi::c_void,
                    size_of::<HPCON>(),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            // SAFETY: zeroed plain data, with its size and fields set below.
            let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
            startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(0);
            startup.lpAttributeList = attributes;
            // The child must not take this process's standard handles, which
            // a test runner has redirected: invalid ones make it use the
            // console.
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
            startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
            // SAFETY: zeroed plain data the call fills.
            let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            let created = updated != 0
                // SAFETY: the command line is a writable, terminated UTF-16
                // string; the structures are complete; no handle is
                // inherited.
                && unsafe {
                    CreateProcessW(
                        ptr::null(),
                        line.as_mut_ptr(),
                        ptr::null(),
                        ptr::null(),
                        FALSE,
                        EXTENDED_STARTUPINFO_PRESENT,
                        ptr::null(),
                        ptr::null(),
                        &raw const startup.StartupInfo,
                        &raw mut info,
                    )
                } != 0;
            let error = last();
            // SAFETY: the list was initialised above.
            unsafe { DeleteProcThreadAttributeList(attributes) };
            drop(list);
            if !created {
                // SAFETY: the console was made above, and nothing runs in it.
                unsafe { ClosePseudoConsole(console) };
                return Err(error);
            }
            // SAFETY: both handles were just made for us.
            let (process, thread) = unsafe {
                (
                    OwnedHandle::from_raw_handle(info.hProcess),
                    OwnedHandle::from_raw_handle(info.hThread),
                )
            };
            drop(thread);
            Ok(Self {
                console,
                input,
                output,
                process,
                pid: info.dwProcessId,
            })
        }

        pub(crate) fn reader(&self) -> io::Result<File> {
            Ok(File::from(self.output.try_clone()?))
        }

        pub(crate) fn writer(&self) -> io::Result<File> {
            Ok(File::from(self.input.try_clone()?))
        }

        pub(crate) fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
            if self.console == 0 {
                return Err(io::Error::other("the pseudo-console has been closed"));
            }
            // SAFETY: the console lives as long as `self`, or until it is
            // closed, which is checked above.
            let result = unsafe { ResizePseudoConsole(self.console, coord(cols, rows)) };
            if result < 0 {
                Err(io::Error::from_raw_os_error(result))
            } else {
                Ok(())
            }
        }

        pub(crate) fn pid(&self) -> u32 {
            self.pid
        }

        pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            // SAFETY: the process handle lives as long as `self`.
            if unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } != WAIT_OBJECT_0 {
                return Ok(None);
            }
            let mut code = 0;
            // SAFETY: `code` is a local the call fills.
            if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &raw mut code) } == 0 {
                return Err(last());
            }
            Ok(Some(ExitStatus::from_raw(code)))
        }

        pub(crate) fn kill(&mut self) -> io::Result<()> {
            // SAFETY: the process handle lives as long as `self`.
            if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
                return Err(last());
            }
            Ok(())
        }
    }

    impl Pty {
        /// Closes the pseudo-console while the program runs in it, as a
        /// terminal window that is closed does: the console host sends the
        /// program `CTRL_CLOSE_EVENT` and ends it when its handler returns
        /// or the host's time runs out.
        ///
        /// `ClosePseudoConsole` may wait for the host, which waits for the
        /// program's handler, so it runs on a thread of its own: this
        /// returns at once, and the program's end is seen with
        /// [`try_wait`](Self::try_wait). A reader of the output sees its
        /// end once the console is gone.
        pub(crate) fn hang_up(&mut self) -> io::Result<()> {
            let console = std::mem::replace(&mut self.console, 0);
            if console == 0 {
                return Ok(());
            }
            let closing = std::thread::Builder::new()
                .name("close-pseudo-console".to_owned())
                .spawn(move || {
                    // SAFETY: the handle was `self`'s and is taken out of it
                    // above, so nothing else uses it or closes it again.
                    unsafe { ClosePseudoConsole(console) };
                });
            match closing {
                Ok(_) => Ok(()),
                Err(error) => {
                    // Not closed: `self` keeps it, and `Drop` closes it.
                    self.console = console;
                    Err(error)
                }
            }
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            if self.console != 0 {
                // SAFETY: closed once, here (a console closed by
                // `hang_up` is 0). A reader of the output sees its end once
                // the console is gone.
                unsafe { ClosePseudoConsole(self.console) };
            }
        }
    }

    /// Whether a read error on the output means the other side has gone.
    pub(crate) fn is_end(error: &io::Error) -> bool {
        // ERROR_BROKEN_PIPE
        error.raw_os_error() == Some(109)
    }

    unsafe extern "system" fn ignore(_: u32) -> windows_sys::core::BOOL {
        super::TRUE
    }

    /// Makes this process stay through console control events: its handler
    /// says each one was handled.
    pub(crate) fn ignore_control_events() {
        // SAFETY: the handler is a function with the signature Windows asks
        // for, and stays for the life of the process.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(ignore), super::TRUE)
        };
    }

    /// Sends a Ctrl+Break event to every process on this program's console.
    pub(crate) fn signal(_pid: u32, _signal: i32) -> io::Result<()> {
        // SAFETY: reads nothing but its arguments.
        if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0) } == 0 {
            return Err(last());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(x: i16, y: i16, buttons: u32, flags: u32, keys: u32) -> MOUSE_EVENT_RECORD {
        MOUSE_EVENT_RECORD {
            dwMousePosition: windows_sys::Win32::System::Console::COORD { X: x, Y: y },
            dwButtonState: buttons,
            dwControlKeyState: keys,
            dwEventFlags: flags,
        }
    }

    /// The console's mouse records become the SGR reports a terminal would
    /// have sent (since the review of step 25): a press, a drag and a
    /// release, a move with no button left out, the wheel each way and its
    /// modifiers, and the right button with `Ctrl`; the decoder then reads
    /// them as it reads a terminal's.
    #[test]
    fn mouse_records_become_the_reports_a_terminal_sends() {
        BUTTONS.store(0, Ordering::SeqCst);
        let up = 120u32 << 16;
        let down = u32::from(0xff88u16) << 16;
        let mut out = Vec::new();
        for record in [
            mouse(4, 2, 0x1, 0, 0),
            mouse(5, 2, 0x1, 0x1, 0),
            mouse(5, 2, 0, 0, 0),
            mouse(5, 2, 0, 0x1, 0),
            mouse(1, 1, up, 0x4, 0),
            mouse(1, 1, down, 0x4, 0x10),
            mouse(0, 0, 0x2, 0, 0x8),
            mouse(0, 0, 0, 0, 0x8),
        ] {
            push_mouse(&record, &mut out);
        }
        assert_eq!(
            String::from_utf8(out.clone()).unwrap(),
            "\x1b[<0;5;3M\x1b[<32;6;3M\x1b[<0;6;3m\x1b[<64;2;2M\x1b[<69;2;2M\x1b[<18;1;1M\x1b[<18;1;1m"
        );
        let mut decoder = crate::Decoder::new();
        decoder.set_mouse(true);
        let mut events = Vec::new();
        decoder.feed(&out, &mut events);
        let kinds: Vec<crate::MouseKind> = events
            .iter()
            .map(|event| match event {
                crate::Event::Mouse(mouse) => mouse.kind,
                other => panic!("not a mouse event: {other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                crate::MouseKind::Press(crate::Button::Left),
                crate::MouseKind::Drag(crate::Button::Left),
                crate::MouseKind::Release(Some(crate::Button::Left)),
                crate::MouseKind::Wheel(crate::Wheel::Up),
                crate::MouseKind::Wheel(crate::Wheel::Down),
                crate::MouseKind::Press(crate::Button::Right),
                crate::MouseKind::Release(Some(crate::Button::Right)),
            ]
        );
    }

    #[test]
    fn the_modes_turn_on_what_is_needed_and_off_what_gets_in_the_way() {
        let output = our_output_mode(0x3);
        assert_ne!(output & ENABLE_VIRTUAL_TERMINAL_PROCESSING, 0);
        let input = our_input_mode(
            ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_PROCESSED_INPUT
                | ENABLE_MOUSE_INPUT
                | 0x40,
        );
        assert_ne!(input & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
        assert_ne!(input & ENABLE_WINDOW_INPUT, 0);
        assert_eq!(
            input & (ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT),
            0
        );
        assert_ne!(input & 0x40, 0, "a flag it does not know is kept");
    }

    #[test]
    fn nothing_is_written_while_the_console_is_not_the_programs() {
        assert!(!ACTIVE.load(Ordering::SeqCst));
        assert!(write(b"\x1b[?25l").is_err());
    }

    #[test]
    fn surrogate_halves_are_joined() {
        let mut out = Vec::new();
        for unit in "a😀é".encode_utf16() {
            push_unit(unit, &mut out);
        }
        assert_eq!(String::from_utf8(out).unwrap(), "a😀é");
        let mut out = Vec::new();
        push_unit(0xdc00, &mut out);
        assert_eq!(String::from_utf8(out).unwrap(), "\u{fffd}");
    }
}
