//! The Unix half of the terminal layer: raw mode, the window's size, waiting
//! for input, and putting the terminal back on every way out.
//!
//! The standard library can tell whether a stream is a terminal, and no
//! more. Raw mode (`termios`), the window size, `poll`, and signal handlers
//! go through `libc` here, and every `unsafe` block says why it is sound.
//!
//! # Signals
//!
//! A signal handler may only call functions that are safe in one. The ones
//! here use `tcsetattr`, `write`, `signal`, `sigprocmask`, `raise` and atomic
//! loads and stores, all of which POSIX lists as async-signal-safe, and read
//! state that was put in place before the handlers were installed and is
//! never moved after.

#![allow(
    unsafe_code,
    reason = "raw mode, the window size and signal handlers are only reachable through the \
              C library; cortex-term is where such calls live"
)]

use std::io;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use super::{RESTORE_MOST, Ready, restore};

const STDIN: libc::c_int = 0;
const STDOUT: libc::c_int = 1;

/// The terminal's settings as they were before raw mode was first turned
/// on, which every way out puts back. Set once, before any handler that
/// reads it is installed.
static ORIGINAL: OnceLock<libc::termios> = OnceLock::new();
/// The raw settings, for putting raw mode back after a suspension.
static RAW: OnceLock<libc::termios> = OnceLock::new();
/// Whether raw mode is on now.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The writing end of the pipe that wakes a wait: written by a waker and by
/// the handlers of SIGWINCH and SIGCONT. -1 until it exists.
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);
/// The reading end of the same pipe.
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
/// Set by the SIGWINCH handler.
static RESIZED: AtomicBool = AtomicBool::new(false);
/// Set when the program was stopped and has been continued.
static RESUMED: AtomicBool = AtomicBool::new(false);
/// Set by a waker.
static WOKEN: AtomicBool = AtomicBool::new(false);
/// Set when the program is putting the terminal back for good: a
/// suspension that ends after it does not turn raw mode on again.
static LEAVING: AtomicBool = AtomicBool::new(false);
/// How long a program a signal was handed to has to end itself.
const WATCHDOG_SECONDS: libc::c_uint = 3;

/// The handlers that were in place before ours, put back on leaving.
static PREVIOUS: Mutex<Vec<(libc::c_int, libc::sigaction)>> = Mutex::new(Vec::new());

/// The signals that end the program, after which the terminal is put back.
/// SIGABRT since step 23: an allocation that failed, or a panic while a
/// panic unwinds, aborts the process, and no panic hook runs then.
const ENDING: [libc::c_int; 5] = [
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGABRT,
];

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// Whether standard input and standard output are both a terminal.
pub(crate) fn is_terminal() -> bool {
    // SAFETY: `isatty` reads nothing but its argument.
    unsafe { libc::isatty(STDIN) == 1 && libc::isatty(STDOUT) == 1 }
}

/// Turns raw mode on and installs the handlers that put the terminal back.
pub(crate) fn enter() -> io::Result<()> {
    if !is_terminal() {
        return Err(io::Error::other(
            "standard input and standard output must both be a terminal",
        ));
    }
    let mut current = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `tcgetattr` fills the structure it is given, which is large
    // enough because it has the type the function asks for.
    check(unsafe { libc::tcgetattr(STDIN, current.as_mut_ptr()) })?;
    // SAFETY: `tcgetattr` succeeded, so the structure is filled.
    let current = unsafe { current.assume_init() };
    let original = *ORIGINAL.get_or_init(|| current);
    let mut raw = original;
    // SAFETY: `cfmakeraw` changes the flags of the structure it is given.
    unsafe { libc::cfmakeraw(&raw mut raw) };
    let raw = *RAW.get_or_init(|| raw);
    wake_pipe()?;
    install_handlers()?;
    // SAFETY: `raw` is a complete `termios` made from the terminal's own.
    check(unsafe { libc::tcsetattr(STDIN, libc::TCSADRAIN, &raw const raw) })?;
    LEAVING.store(false, Ordering::SeqCst);
    ACTIVE.store(true, Ordering::SeqCst);
    Ok(())
}

/// Puts the terminal's settings back, and the handlers that were there
/// before. Safe to call more than once.
///
/// While signals are handed to the program, the handlers of the signals
/// that end it stay until [`release_signals`] (since the review of step
/// 23): a signal that comes after the terminal is left, while the program
/// ends (its history, its core) or while an external editor has the
/// terminal, is still the program's to end, with its watchdog, and not
/// the default action's, which would end it part-way.
pub(crate) fn leave() {
    restore_modes();
    let keep = super::hand_off();
    let mut previous = PREVIOUS.lock().unwrap_or_else(PoisonError::into_inner);
    let guarding = GUARDING.load(Ordering::SeqCst);
    let mut kept = Vec::new();
    for (signal, action) in previous.drain(..) {
        if keep && ENDING.contains(&signal) && !(guarding && is_interrupt(signal)) {
            kept.push((signal, action));
            continue;
        }
        if guarding && is_interrupt(signal) {
            // An interrupt guard lives: its handler stays, and what it puts
            // back when it ends is what was there before the terminal.
            if let Some(entry) = GUARDED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter_mut()
                .find(|(guarded, _)| *guarded == signal)
            {
                entry.1 = action;
            }
            continue;
        }
        // SAFETY: `action` is what `sigaction` gave back for this signal.
        unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) };
    }
    *previous = kept;
}

/// Puts back the handlers [`leave`] kept for signals handed to the program:
/// it has ended, and is about to exit. Safe to call more than once.
pub(crate) fn release_signals() {
    let mut previous = PREVIOUS.lock().unwrap_or_else(PoisonError::into_inner);
    // Only once the terminal is left: while it is the program's, its
    // handlers are what put it back.
    if ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    for (signal, action) in previous.drain(..) {
        // SAFETY: `action` is what `sigaction` gave back for this signal.
        unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) };
    }
}

/// Whether `signal` is one an interrupt guard passes over.
fn is_interrupt(signal: libc::c_int) -> bool {
    signal == libc::SIGINT || signal == libc::SIGQUIT
}

/// Whether raw mode is on: the terminal is the program's.
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Writes the restore sequence and puts the original settings back, if
/// raw mode is on. Async-signal-safe: see the module documentation.
pub(crate) fn emergency() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut buffer = [0u8; RESTORE_MOST];
    let rows = super::rows_to_move_up();
    let bytes = restore(&mut buffer, rows);
    // SAFETY: the pointer and length are those of a buffer on this stack.
    unsafe { libc::write(STDOUT, bytes.as_ptr().cast(), bytes.len()) };
    // The live region is erased, unless a frame was cut short: its write
    // sends the restore again, with the frame's count, once it has landed.
    if rows > 0 {
        super::set_rows_above(0);
    }
    if let Some(original) = ORIGINAL.get() {
        // SAFETY: `original` is the complete `termios` the terminal had.
        unsafe { libc::tcsetattr(STDIN, libc::TCSADRAIN, original) };
    }
}

/// Puts the original settings back without writing anything: for a normal
/// exit, where the presenter has already finished.
///
/// Raw mode is marked off only once the settings are back: a signal that
/// comes in between still finds it on, and puts the terminal back itself.
fn restore_modes() {
    // A Ctrl+Z from here on must not put raw mode back when it continues.
    LEAVING.store(true, Ordering::SeqCst);
    if ACTIVE.load(Ordering::SeqCst)
        && let Some(original) = ORIGINAL.get()
    {
        // SAFETY: `original` is the complete `termios` the terminal had.
        unsafe { libc::tcsetattr(STDIN, libc::TCSADRAIN, original) };
    }
    ACTIVE.store(false, Ordering::SeqCst);
}

/// Makes the pipe that wakes a wait, once.
fn wake_pipe() -> io::Result<()> {
    if WAKE_READ.load(Ordering::SeqCst) >= 0 {
        return Ok(());
    }
    let mut fds = [0; 2];
    // SAFETY: `pipe` fills an array of two descriptors.
    check(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    for fd in fds {
        // SAFETY: both descriptors were just made; the flags only change how
        // they behave.
        unsafe {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    WAKE_READ.store(fds[0], Ordering::SeqCst);
    WAKE_WRITE.store(fds[1], Ordering::SeqCst);
    Ok(())
}

/// Writes one byte to the wake pipe. Async-signal-safe.
fn poke() {
    let fd = WAKE_WRITE.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: one byte from a local; a full pipe refuses it, which is
        // fine: a wake is already waiting.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

/// Wakes [`wait`] from another thread.
pub(crate) fn wake() {
    WOKEN.store(true, Ordering::SeqCst);
    poke();
}

extern "C" fn on_resize(_: libc::c_int) {
    RESIZED.store(true, Ordering::SeqCst);
    poke();
}

extern "C" fn on_ending(signal: libc::c_int) {
    // An interrupt guard lives, or has just ended: the key was for a child,
    // or came as the terminal was taken back from one, and another thread
    // took it while the guard was ending.
    if is_interrupt(signal) && (GUARDING.load(Ordering::SeqCst) || guard_just_ended()) {
        return;
    }
    emergency();
    // Handed to the program, which ends itself; the watchdog, whose default
    // action ends the process, does so if it has not within 3 s. A second
    // signal finds the first taken, and ends the process now.
    // SIGABRT is not handed over: an abort ends the process as it asks.
    if signal != libc::SIGABRT && super::hand_over(128 + u8::try_from(signal).unwrap_or(0)) {
        // SAFETY: `alarm` and the waker's atomics and pipe write are
        // async-signal-safe.
        unsafe { libc::alarm(WATCHDOG_SECONDS) };
        wake();
        return;
    }
    // The terminal hung up, and that was taken as SIGHUP's end already:
    // this SIGHUP is the same end (the kernel's, or a shell's resent to its
    // jobs), not a second signal that ends the process at once. Asked only
    // once the hand-over is found taken, as Windows's handler asks it: the
    // loop's read can find the terminal gone, and take the hand-over, on
    // another thread while this one puts the terminal back, and a SIGHUP
    // asked before that was then taken for a second signal (since the third
    // pull request of step 23's review fixes).
    if signal == libc::SIGHUP && super::hung_up_already(129) {
        return;
    }
    // SAFETY: putting the default action back and raising the signal again
    // ends the process as the signal would have.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

extern "C" fn on_stop(_: libc::c_int) {
    let was_active = ACTIVE.load(Ordering::SeqCst);
    emergency();
    // SAFETY: stop with the default action, which needs SIGTSTP unblocked
    // inside its own handler; when the program is continued the stop
    // returns, and the handler and raw mode are put back.
    unsafe {
        libc::signal(libc::SIGTSTP, libc::SIG_DFL);
        let mut set = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        libc::sigaddset(set.as_mut_ptr(), libc::SIGTSTP);
        libc::sigprocmask(libc::SIG_UNBLOCK, set.as_ptr(), std::ptr::null_mut());
        libc::raise(libc::SIGTSTP);
        // Continued: the handler back with its mask, then raw mode, marked
        // on first so that a signal in between puts the terminal back.
        let action = action(on_stop);
        libc::sigaction(libc::SIGTSTP, &raw const action, std::ptr::null_mut());
        if was_active
            && !LEAVING.load(Ordering::SeqCst)
            && let Some(raw) = RAW.get()
        {
            ACTIVE.store(true, Ordering::SeqCst);
            libc::tcsetattr(STDIN, libc::TCSADRAIN, raw);
        }
    }
    RESUMED.store(true, Ordering::SeqCst);
    poke();
}

/// The action that runs `handler`, with every other signal it shares the
/// restore with blocked while it runs: a second signal cannot find the
/// terminal half put back and end the process before the first has
/// finished. Async-signal-safe: `sigemptyset` and `sigaddset` only.
fn action(handler: extern "C" fn(libc::c_int)) -> libc::sigaction {
    // SAFETY: a zeroed `sigaction` is valid plain data; the handler, the
    // flags and the mask are set before it is used.
    unsafe {
        let mut action = MaybeUninit::<libc::sigaction>::zeroed().assume_init();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        let mask = &raw mut action.sa_mask;
        libc::sigemptyset(mask);
        for other in ENDING.into_iter().chain([libc::SIGTSTP, libc::SIGWINCH]) {
            libc::sigaddset(mask, other);
        }
        action
    }
}

fn install(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) -> io::Result<()> {
    let action = action(handler);
    let mut old = MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: `action` is complete, and `old` is filled by the call.
    let mut old = unsafe {
        check(libc::sigaction(signal, &raw const action, old.as_mut_ptr()))?;
        old.assume_init()
    };
    // Entered while an interrupt guard lives: what to put back on leaving is
    // what was there before the guard, not the guard's own handler.
    if is_passing_over(&old)
        && let Some((_, before)) = GUARDED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(guarded, _)| *guarded == signal)
    {
        old = *before;
    }
    PREVIOUS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((signal, old));
    Ok(())
}

/// Installs the handlers that are not in place: all of them, or, entering
/// again after [`leave`] kept those of the ending signals, the rest.
fn install_handlers() -> io::Result<()> {
    let installed: Vec<libc::c_int> = PREVIOUS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|(signal, _)| *signal)
        .collect();
    let handlers: [(libc::c_int, extern "C" fn(libc::c_int)); 2] =
        [(libc::SIGWINCH, on_resize), (libc::SIGTSTP, on_stop)];
    for (signal, handler) in handlers.into_iter().chain(
        ENDING
            .into_iter()
            .map(|signal| (signal, on_ending as extern "C" fn(_))),
    ) {
        if !installed.contains(&signal) {
            install(signal, handler)?;
        }
    }
    Ok(())
}

/// The actions SIGINT and SIGQUIT had before [`ignore_interrupts`].
pub(crate) type Interrupts = Vec<(libc::c_int, libc::sigaction)>;

/// The handler of SIGINT and SIGQUIT while they are passed over: nothing.
/// A handler, not "ignore": a program started meanwhile has its own
/// default again, since `exec` resets a caught signal and keeps an ignored
/// one.
extern "C" fn on_interrupt_passed_over(_: libc::c_int) {}

/// Whether a guard lives: the terminal's own handler of SIGINT and SIGQUIT
/// passes over them too, should the terminal be entered again meanwhile.
static GUARDING: AtomicBool = AtomicBool::new(false);
/// What SIGINT and SIGQUIT did before the guard: what the terminal records
/// as their previous action if it is entered while the guard lives, in
/// place of the guard's own handler.
static GUARDED: Mutex<Vec<(libc::c_int, libc::sigaction)>> = Mutex::new(Vec::new());

/// Catches SIGINT and SIGQUIT with a handler that does nothing, and
/// returns what they did before.
pub(crate) fn ignore_interrupts() -> io::Result<Interrupts> {
    GUARDING.store(true, Ordering::SeqCst);
    let mut previous = Interrupts::new();
    for signal in [libc::SIGINT, libc::SIGQUIT] {
        let ignore = action(on_interrupt_passed_over);
        let mut old = MaybeUninit::<libc::sigaction>::zeroed();
        // SAFETY: `ignore` is complete, and `old` is filled by the call.
        let set = unsafe { libc::sigaction(signal, &raw const ignore, old.as_mut_ptr()) };
        if let Err(error) = check(set) {
            restore_interrupts(&previous);
            return Err(error);
        }
        // SAFETY: the call succeeded, so it filled `old`.
        previous.push((signal, unsafe { old.assume_init() }));
    }
    GUARDED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone_from(&previous);
    Ok(previous)
}

/// When the last guard ended, in nanoseconds of the monotonic clock; 0 for
/// never.
static GUARD_ENDED: AtomicU64 = AtomicU64::new(0);

/// How long after a guard ends the terminal's handler still passes the
/// interrupts over: a signal another thread took from the queue as the
/// guard was ending. The terminal is in raw mode again by then, so `Ctrl+C`
/// is a key, and a signal can only come from outside.
const GUARD_GRACE_NANOS: u64 = 1_000_000_000;

/// The monotonic clock, in nanoseconds. Async-signal-safe.
fn monotonic() -> u64 {
    let mut now = MaybeUninit::<libc::timespec>::zeroed();
    // SAFETY: `clock_gettime` fills the structure it is given, and is
    // async-signal-safe.
    let read = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, now.as_mut_ptr()) };
    if read != 0 {
        return 0;
    }
    // SAFETY: the call succeeded, so it filled `now`.
    let now = unsafe { now.assume_init() };
    u64::try_from(now.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::try_from(now.tv_nsec).unwrap_or(0))
}

/// Whether a guard ended less than [`GUARD_GRACE_NANOS`] ago.
fn guard_just_ended() -> bool {
    let ended = GUARD_ENDED.load(Ordering::SeqCst);
    ended != 0 && monotonic().saturating_sub(ended) < GUARD_GRACE_NANOS
}

/// Whether `action` is the guard's do-nothing handler.
fn is_passing_over(action: &libc::sigaction) -> bool {
    action.sa_sigaction == on_interrupt_passed_over as *const () as libc::sighandler_t
}

/// Puts back what [`ignore_interrupts`] found.
///
/// A signal sent to the process while the guard lived may still be
/// pending: the kernel hands it to whichever thread runs next, which on a
/// loaded machine can be after the guard is gone, when the signal would
/// end the process. So the signals are first blocked on this thread and
/// any pending one taken, with the do-nothing handler still in place.
pub(crate) fn restore_interrupts(previous: &Interrupts) {
    // What to put back: what was there before the guard, or, if the
    // terminal was left meanwhile, what was there before the terminal.
    let previous: Interrupts = {
        let guarded = GUARDED.lock().unwrap_or_else(PoisonError::into_inner);
        if guarded.is_empty() {
            previous.clone()
        } else {
            guarded.clone()
        }
    };
    let previous = &previous;
    let mut set = MaybeUninit::<libc::sigset_t>::uninit();
    let mut old = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `set` is initialised by `sigemptyset` before it is read, and
    // `old` is filled by `pthread_sigmask` before it is read; `sigtimedwait`
    // with a zero timeout only takes signals of `set`, which are blocked.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for (signal, _) in previous {
            libc::sigaddset(set.as_mut_ptr(), *signal);
        }
        let blocked = libc::pthread_sigmask(libc::SIG_BLOCK, set.as_ptr(), old.as_mut_ptr()) == 0;
        if blocked {
            let now = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            loop {
                let taken = libc::sigtimedwait(set.as_ptr(), std::ptr::null_mut(), &raw const now);
                if taken > 0 {
                    continue;
                }
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
        }
        for (signal, action) in previous.iter().rev() {
            // Put back only where the guard's handler is still in place:
            // the terminal, entered again meanwhile, has its own, and has
            // recorded `action` as what to put back when it leaves.
            let mut current = MaybeUninit::<libc::sigaction>::zeroed();
            if libc::sigaction(*signal, std::ptr::null(), current.as_mut_ptr()) == 0
                && is_passing_over(&current.assume_init())
            {
                // `action` is what `sigaction` gave back for this signal.
                libc::sigaction(*signal, action, std::ptr::null_mut());
            }
        }
        GUARDED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        GUARD_ENDED.store(monotonic(), Ordering::SeqCst);
        GUARDING.store(false, Ordering::SeqCst);
        if blocked {
            libc::pthread_sigmask(libc::SIG_SETMASK, old.as_ptr(), std::ptr::null_mut());
        }
    }
}

/// Stops the program as Ctrl+Z would, with the terminal put back while it
/// is stopped and raw mode put back when it is continued.
pub(crate) fn suspend() {
    // SAFETY: SIGTSTP goes to the handler above, or, if none is installed,
    // stops the process as it would anyway.
    unsafe { libc::raise(libc::SIGTSTP) };
}

/// The window's size, columns then rows.
/// The console's mouse input, which only Windows has: a terminal is asked
/// for the mouse by the sequences alone.
pub(crate) fn set_mouse_input(_on: bool) {}

pub(crate) fn size() -> io::Result<(u16, u16)> {
    let mut size = MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: `TIOCGWINSZ` fills a `winsize`.
    check(unsafe { libc::ioctl(STDOUT, libc::TIOCGWINSZ, size.as_mut_ptr()) })?;
    // SAFETY: zeroed, then filled by the call.
    let size = unsafe { size.assume_init() };
    Ok((size.ws_col, size.ws_row))
}

/// Writes all of `bytes` to standard output: one call, unless the terminal
/// takes only part of it.
///
/// Once the terminal has been put back (a panic on another thread, a
/// signal), nothing more is written: a frame would turn modes on again in a
/// terminal that is no longer the program's.
pub(crate) fn write(bytes: &[u8]) -> io::Result<()> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return Err(io::Error::other("the terminal has been put back"));
    }
    super::set_in_flight(true);
    let written = write_all(bytes);
    super::set_in_flight(false);
    // Put back while this frame was being written, by a signal on this
    // thread or a panic on another: the frame may have landed after the
    // restore, so it goes again, now that the frame's rows are known.
    if !ACTIVE.load(Ordering::SeqCst) {
        let mut buffer = [0u8; RESTORE_MOST];
        let _ = write_all(restore(&mut buffer, super::rows_to_move_up()));
        super::set_rows_above(0);
    }
    written
}

fn write_all(bytes: &[u8]) -> io::Result<()> {
    let mut rest = bytes;
    while !rest.is_empty() {
        // SAFETY: the pointer and length are those of a live slice.
        let written = unsafe { libc::write(STDOUT, rest.as_ptr().cast(), rest.len()) };
        match usize::try_from(written) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => rest = &rest[count..],
            Err(_) => {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

/// Waits for input, a resize, a resumption or a wake, for at most
/// `timeout` (`None`: as long as it takes). Input is added to `into`.
pub(crate) fn wait(timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
    let millis = timeout.map_or(-1, |timeout| {
        let rounded_up = timeout.as_nanos().div_ceil(1_000_000);
        libc::c_int::try_from(rounded_up).unwrap_or(libc::c_int::MAX)
    });
    let wake = WAKE_READ.load(Ordering::SeqCst);
    loop {
        if let Some(ready) = flags() {
            return Ok(ready);
        }
        let mut fds = [
            libc::pollfd {
                fd: STDIN,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: an array of two `pollfd`, and its length.
        let count = unsafe { libc::poll(fds.as_mut_ptr(), 2, millis) };
        if count == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if count == 0 {
            return Ok(flags().unwrap_or(Ready::Timeout));
        }
        if fds[1].revents != 0 {
            drain(wake);
        }
        if fds[0].revents != 0 {
            let mut buffer = [0u8; 4096];
            // SAFETY: reads into a local buffer of its own length.
            let read = unsafe { libc::read(STDIN, buffer.as_mut_ptr().cast(), buffer.len()) };
            match usize::try_from(read) {
                // In raw mode a read gives nothing only when the terminal
                // has hung up.
                Ok(0) => return gone(io::ErrorKind::UnexpectedEof.into()),
                Ok(count) => {
                    into.extend_from_slice(&buffer[..count]);
                    return Ok(Ready::Input);
                }
                Err(_) => {
                    let error = io::Error::last_os_error();
                    if !matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) {
                        return gone(error);
                    }
                }
            }
        }
    }
}

/// The terminal could not be read, `error` saying why: closed (end of
/// input, EIO), or the connection to it dropped. A signal already handed
/// over is what the program ends for; otherwise, while signals are handed
/// over, it ends as SIGHUP's end would have it, with its watchdog (since the
/// review of step 23): the window or the SSH connection closing reaches the
/// reader before, or instead of, the hang-up's signal. Then the wait is
/// woken, and [`super::ended_by`] says the code. The error stands when
/// nothing is handed over.
fn gone(error: io::Error) -> io::Result<Ready> {
    if super::ended_by().is_some() {
        super::hung_up_while_ending();
        return Ok(Ready::Woken);
    }
    if super::hang_up() {
        emergency();
        // SAFETY: `alarm` reads nothing but its argument.
        unsafe { libc::alarm(WATCHDOG_SECONDS) };
        return Ok(Ready::Woken);
    }
    Err(error)
}

fn flags() -> Option<Ready> {
    if RESUMED.swap(false, Ordering::SeqCst) {
        RESIZED.store(false, Ordering::SeqCst);
        return Some(Ready::Resumed);
    }
    if RESIZED.swap(false, Ordering::SeqCst) {
        return Some(Ready::Resize);
    }
    if WOKEN.swap(false, Ordering::SeqCst) {
        return Some(Ready::Woken);
    }
    None
}

fn drain(fd: libc::c_int) {
    let mut buffer = [0u8; 64];
    // SAFETY: reads into a local buffer of its own length from a
    // non-blocking pipe, until it is empty.
    while unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) } > 0 {}
}

/// A description of the terminal's settings on standard input, for tests
/// that check they were put back: every field of its `termios`.
#[cfg(feature = "pty")]
pub(crate) fn modes() -> io::Result<String> {
    let mut current = MaybeUninit::<libc::termios>::uninit();
    // SAFETY: as in `enter`.
    check(unsafe { libc::tcgetattr(STDIN, current.as_mut_ptr()) })?;
    // SAFETY: filled by the call.
    let t = unsafe { current.assume_init() };
    Ok(format!(
        "iflag={:x} oflag={:x} cflag={:x} lflag={:x} cc={:?}",
        t.c_iflag, t.c_oflag, t.c_cflag, t.c_lflag, t.c_cc
    ))
}

/// Pseudo-terminals, for tests.
#[cfg(feature = "pty")]
pub(crate) mod pty {
    #![allow(
        clippy::disallowed_types,
        reason = "test support: it starts a process in a pseudo-terminal, and reads and writes \
                  the terminal's other end as a file"
    )]

    use std::fs::File;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, ExitStatus, Stdio};

    use super::check;

    /// A program running in a pseudo-terminal of its own.
    #[derive(Debug)]
    pub(crate) struct Pty {
        master: OwnedFd,
        child: Child,
    }

    fn set_size(fd: &OwnedFd, cols: u16, rows: u16) -> io::Result<()> {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: `TIOCSWINSZ` reads a `winsize`.
        check(unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &raw const size) })?;
        Ok(())
    }

    impl Pty {
        pub(crate) fn spawn(
            program: &str,
            arguments: &[String],
            env: &[(String, String)],
            cols: u16,
            rows: u16,
        ) -> io::Result<Self> {
            Self::spawn_as(program, arguments, env, (cols, rows), true)
        }

        /// As [`spawn`](Self::spawn); `controlling`: the pseudo-terminal is
        /// the program's controlling terminal, in a session it leads. Without
        /// it the program reads and writes the terminal and is sent no
        /// signal when it closes: its reads alone say so.
        pub(crate) fn spawn_as(
            program: &str,
            arguments: &[String],
            env: &[(String, String)],
            (cols, rows): (u16, u16),
            controlling: bool,
        ) -> io::Result<Self> {
            // SAFETY: `posix_openpt` returns a new descriptor or -1.
            let master = check(unsafe {
                libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC)
            })?;
            // SAFETY: the descriptor was just made, and is ours alone.
            let master = unsafe { OwnedFd::from_raw_fd(master) };
            // SAFETY: both act on the master just made.
            unsafe {
                check(libc::grantpt(master.as_raw_fd()))?;
                check(libc::unlockpt(master.as_raw_fd()))?;
                check(libc::fcntl(
                    master.as_raw_fd(),
                    libc::F_SETFD,
                    libc::FD_CLOEXEC,
                ))?;
            }
            let mut name = [0 as libc::c_char; 128];
            // SAFETY: the buffer and its length.
            let failed =
                unsafe { libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) };
            if failed != 0 {
                return Err(io::Error::from_raw_os_error(failed));
            }
            // SAFETY: `ptsname_r` wrote a string ending in a zero.
            let slave = check(unsafe {
                libc::open(
                    name.as_ptr(),
                    libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
                )
            })?;
            // SAFETY: the descriptor was just opened, and is ours alone.
            let slave = unsafe { OwnedFd::from_raw_fd(slave) };
            set_size(&master, cols, rows)?;

            let mut command = Command::new(program);
            command
                .args(arguments)
                .env("TERM", "xterm-256color")
                .envs(env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            // SAFETY: between fork and exec only `setsid` and `ioctl` run,
            // both async-signal-safe: a session of its own, with the
            // pseudo-terminal (its standard input by now) as its controlling
            // terminal.
            unsafe {
                command.pre_exec(move || {
                    check(libc::setsid())?;
                    if controlling {
                        check(libc::ioctl(0, libc::TIOCSCTTY, 0))?;
                    }
                    Ok(())
                });
            }
            let child = command.spawn()?;
            Ok(Self { master, child })
        }

        pub(crate) fn reader(&self) -> io::Result<File> {
            Ok(File::from(self.master.try_clone()?))
        }

        pub(crate) fn writer(&self) -> io::Result<File> {
            Ok(File::from(self.master.try_clone()?))
        }

        pub(crate) fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
            set_size(&self.master, cols, rows)
        }

        pub(crate) fn pid(&self) -> u32 {
            self.child.id()
        }

        pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.child.try_wait()
        }

        pub(crate) fn kill(&mut self) -> io::Result<()> {
            self.child.kill()
        }

        /// What the kernel does when the terminal hangs up under the
        /// session that has it: SIGHUP, then SIGCONT (so that a stopped
        /// program sees the hang-up), to the session's foreground process
        /// group. The program leads its own session (`setsid` in
        /// [`spawn`](Self::spawn)), so its process group is its pid.
        ///
        /// The kernel's own hang-up, closing every descriptor of the
        /// terminal's master, cannot be made here: the driver's reader holds
        /// one, blocked in a read, and a descriptor stays open while a read
        /// on it does. So the signals are sent as the kernel would; a write
        /// to the terminal afterwards still succeeds, where after a real
        /// hang-up it fails.
        pub(crate) fn hang_up(&mut self) -> io::Result<()> {
            // A program that has ended and been waited for has no group to
            // hang up, and its id may be another's now.
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            let group = libc::pid_t::try_from(self.child.id()).map_err(io::Error::other)?;
            for signal in [libc::SIGHUP, libc::SIGCONT] {
                // SAFETY: `kill` reads nothing but its arguments; a negative
                // pid is a process group.
                match check(unsafe { libc::kill(-group, signal) }) {
                    Ok(_) => {}
                    // Everything in the group has gone: nothing to hang up.
                    Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }
    }

    extern "C" fn hangup_passed_over(_: libc::c_int) {}

    /// Makes this process stay through SIGHUP: a handler that does nothing,
    /// which, unlike "ignore", a program this one starts does not inherit.
    pub(crate) fn pass_over_hangup() -> io::Result<()> {
        let mut action = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
        // SAFETY: a zeroed `sigaction` is valid plain data; the handler and
        // the flags are set before the call, which only reads it.
        unsafe {
            let action = action.as_mut_ptr();
            (*action).sa_sigaction = hangup_passed_over as *const () as libc::sighandler_t;
            (*action).sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&raw mut (*action).sa_mask);
            check(libc::sigaction(libc::SIGHUP, action, std::ptr::null_mut()))?;
        }
        Ok(())
    }

    /// Makes this process the reaper of its descendants' orphans, on Linux.
    pub(crate) fn adopt_orphans() -> io::Result<()> {
        #[cfg(target_os = "linux")]
        // SAFETY: `prctl` with this option reads nothing but its arguments.
        check(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) })?;
        Ok(())
    }

    /// Whether a read error on the master means the other side has gone:
    /// Linux reports that as EIO once every program in the terminal has
    /// closed it.
    pub(crate) fn is_end(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::EIO)
    }

    /// The seconds left before the watchdog's alarm, 0 when none is set;
    /// the alarm is left as it was.
    pub(crate) fn watchdog_left() -> u32 {
        // SAFETY: `alarm` reads nothing but its argument; the second call
        // sets back what the first took off.
        unsafe {
            let left = libc::alarm(0);
            if left > 0 {
                libc::alarm(left);
            }
            left
        }
    }

    /// Sends `signal` to process `pid`.
    pub(crate) fn signal(pid: u32, signal: i32) -> io::Result<()> {
        let pid = libc::pid_t::try_from(pid).map_err(io::Error::other)?;
        // SAFETY: `kill` reads nothing but its arguments.
        check(unsafe { libc::kill(pid, signal) })?;
        Ok(())
    }

    /// A signal set holding SIGINT alone.
    fn interrupt_set() -> libc::sigset_t {
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: `sigemptyset` initialises the set before `sigaddset` reads
        // it, and both only write the set they are given.
        unsafe {
            libc::sigemptyset(set.as_mut_ptr());
            libc::sigaddset(set.as_mut_ptr(), libc::SIGINT);
            set.assume_init()
        }
    }

    /// Blocks SIGINT on the calling thread, sends it to that thread, and
    /// returns the thread's mask before: the signal is then pending on the
    /// thread, as one the kernel has not handed to a thread yet is.
    pub(crate) fn interrupt_pending_here() -> io::Result<libc::sigset_t> {
        let set = interrupt_set();
        let mut previous = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: `set` is initialised, and `previous` is filled by the call
        // before it is read.
        let blocked = unsafe {
            libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, previous.as_mut_ptr())
        };
        if blocked != 0 {
            return Err(io::Error::from_raw_os_error(blocked));
        }
        // SAFETY: filled by the call above, which succeeded.
        let previous = unsafe { previous.assume_init() };
        // SAFETY: `pthread_self` is this thread, and `pthread_kill` sends it
        // a signal it blocks, which waits on it.
        let sent = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGINT) };
        if sent != 0 {
            // SAFETY: `previous` is the mask the call above returned.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &raw const previous, std::ptr::null_mut())
            };
            return Err(io::Error::from_raw_os_error(sent));
        }
        Ok(previous)
    }

    /// Whether SIGINT is pending on the calling thread or the process.
    pub(crate) fn interrupt_is_pending() -> bool {
        let mut pending = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: `sigpending` fills the set it is given, and `sigismember`
        // reads the filled set.
        unsafe {
            libc::sigpending(pending.as_mut_ptr()) == 0
                && libc::sigismember(pending.as_ptr(), libc::SIGINT) == 1
        }
    }

    /// Puts back the calling thread's mask `previous`: a signal it blocked
    /// that is still pending is delivered now.
    pub(crate) fn restore_mask(previous: &libc::sigset_t) {
        // SAFETY: `previous` is a complete mask.
        unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, previous, std::ptr::null_mut());
        }
    }

    /// Raises SIGINT on the calling thread: its handler has run when this
    /// returns.
    pub(crate) fn raise_interrupt() -> io::Result<()> {
        // SAFETY: `raise` reads nothing but its argument.
        check(unsafe { libc::raise(libc::SIGINT) })?;
        Ok(())
    }

    /// Raises SIGHUP on the calling thread: its handler has run when this
    /// returns.
    pub(crate) fn raise_hangup() -> io::Result<()> {
        // SAFETY: `raise` reads nothing but its argument.
        check(unsafe { libc::raise(libc::SIGHUP) })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_written_while_the_terminal_is_not_the_programs() {
        // The test process never entered raw mode: as after a restore.
        assert!(!ACTIVE.load(Ordering::SeqCst));
        assert!(write(b"\x1b[?25l").is_err());
    }

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_interrupt(_: libc::c_int) {
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    #[test]
    fn interrupts_are_ignored_while_the_guard_lives_and_handled_after() {
        // A handler of the test's own, so that a signal that is not ignored
        // is seen rather than ending the test.
        let mut before = Vec::new();
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            let action = action(on_interrupt);
            let mut old = MaybeUninit::<libc::sigaction>::zeroed();
            // SAFETY: `action` is complete, and `old` is filled by the call.
            unsafe { check(libc::sigaction(signal, &raw const action, old.as_mut_ptr())).unwrap() };
            // SAFETY: the call succeeded.
            before.push((signal, unsafe { old.assume_init() }));
        }
        let guard = crate::ignore_interrupts().unwrap();
        for signal in [libc::SIGINT, libc::SIGQUIT] {
            // SAFETY: the signal is ignored now: raising it does nothing.
            unsafe { libc::raise(signal) };
        }
        assert!(
            !INTERRUPTED.load(Ordering::SeqCst),
            "ignored while the guard lives"
        );
        drop(guard);
        // SAFETY: the test's handler is back, and only sets a flag.
        unsafe { libc::raise(libc::SIGINT) };
        assert!(
            INTERRUPTED.swap(false, Ordering::SeqCst),
            "handled again after it"
        );
        // SAFETY: as above.
        unsafe { libc::raise(libc::SIGQUIT) };
        assert!(INTERRUPTED.load(Ordering::SeqCst), "SIGQUIT too");
        restore_interrupts(&before);
    }

    #[test]
    fn the_terminal_entered_under_a_guard_keeps_its_handler_and_what_was_before() {
        let mut before = MaybeUninit::<libc::sigaction>::zeroed();
        let mine = action(on_interrupt);
        // SAFETY: `mine` is complete, and `before` is filled by the call.
        unsafe {
            check(libc::sigaction(
                libc::SIGINT,
                &raw const mine,
                before.as_mut_ptr(),
            ))
            .unwrap();
        }
        let guard = crate::ignore_interrupts().unwrap();
        // The terminal is entered again while the guard lives.
        install(libc::SIGINT, on_ending).unwrap();
        let recorded = PREVIOUS
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(signal, _)| *signal == libc::SIGINT)
            .map(|(_, action)| action.sa_sigaction);
        assert_eq!(
            recorded,
            Some(mine.sa_sigaction),
            "what was before the guard"
        );
        // A key while the guard lives is passed over by the terminal's
        // handler.
        // SAFETY: the handler returns at once while the guard lives.
        unsafe { libc::raise(libc::SIGINT) };
        drop(guard);
        let mut now = MaybeUninit::<libc::sigaction>::zeroed();
        // SAFETY: a null action reads the current one.
        unsafe {
            check(libc::sigaction(
                libc::SIGINT,
                std::ptr::null(),
                now.as_mut_ptr(),
            ))
            .unwrap();
        }
        // SAFETY: the call succeeded.
        let now = unsafe { now.assume_init() };
        assert_eq!(
            now.sa_sigaction, on_ending as *const () as libc::sighandler_t,
            "the terminal's handler stays"
        );
        PREVIOUS.lock().unwrap().clear();
        // SAFETY: putting back the action found at the start.
        unsafe { libc::sigaction(libc::SIGINT, before.as_ptr(), std::ptr::null_mut()) };
    }

    #[test]
    fn leaving_the_terminal_under_a_guard_keeps_the_guard_and_puts_back_what_was_before() {
        let mut before = MaybeUninit::<libc::sigaction>::zeroed();
        let mine = action(on_interrupt);
        // SAFETY: `mine` is complete, and `before` is filled by the call.
        unsafe {
            check(libc::sigaction(
                libc::SIGINT,
                &raw const mine,
                before.as_mut_ptr(),
            ))
            .unwrap();
        }
        // The terminal is entered, then the guard taken, then the terminal
        // left: as the external editor does.
        install(libc::SIGINT, on_ending).unwrap();
        let guard = crate::ignore_interrupts().unwrap();
        leave();
        let read = || {
            let mut now = MaybeUninit::<libc::sigaction>::zeroed();
            // SAFETY: a null action reads the current one.
            unsafe {
                check(libc::sigaction(
                    libc::SIGINT,
                    std::ptr::null(),
                    now.as_mut_ptr(),
                ))
                .unwrap();
                now.assume_init().sa_sigaction
            }
        };
        assert_eq!(
            read(),
            on_interrupt_passed_over as *const () as libc::sighandler_t,
            "the guard's handler stays when the terminal is left"
        );
        // SAFETY: the guard's handler does nothing.
        unsafe { libc::raise(libc::SIGINT) };
        assert!(!INTERRUPTED.load(Ordering::SeqCst));
        drop(guard);
        assert_eq!(
            read(),
            mine.sa_sigaction,
            "what was there before the terminal"
        );
        // SAFETY: putting back the action found at the start.
        unsafe { libc::sigaction(libc::SIGINT, before.as_ptr(), std::ptr::null_mut()) };
    }

    #[test]
    fn a_panic_with_the_guard_puts_the_actions_back() {
        let mut found = MaybeUninit::<libc::sigaction>::zeroed();
        // SAFETY: a null action reads the current one into `found`.
        unsafe {
            check(libc::sigaction(
                libc::SIGINT,
                std::ptr::null(),
                found.as_mut_ptr(),
            ))
            .unwrap()
        };
        // SAFETY: the call succeeded.
        let before = unsafe { found.assume_init() }.sa_sigaction;
        let unwound = std::panic::catch_unwind(|| {
            let _guard = crate::ignore_interrupts().unwrap();
            panic!("a panic while the editor runs");
        });
        assert!(unwound.is_err());
        // SAFETY: as above.
        unsafe {
            check(libc::sigaction(
                libc::SIGINT,
                std::ptr::null(),
                found.as_mut_ptr(),
            ))
            .unwrap()
        };
        // SAFETY: the call succeeded.
        assert_eq!(unsafe { found.assume_init() }.sa_sigaction, before);
        assert_ne!(before, libc::SIG_IGN);
    }
}
