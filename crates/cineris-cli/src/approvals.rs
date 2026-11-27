//! `cineris approvals …`, and the password typed at a terminal for the
//! subcommands that sign (the signed approvals step;
//! `docs/architecture/06-permissions-and-sandbox.md`, "Signed approvals").
//!
//! Every approval the harness keeps for the person is signed with a key
//! made from their password, each time they type it, and dropped once it
//! has signed (`cineris_core::signed`). Here it is typed:
//!
//! - **At a terminal, never anywhere else.** The password is read with the
//!   terminal's prompt (`cineris_tui::secret::PasswordTerminal`): echo off,
//!   type-ahead dropped, the terminal closed to other programs while it
//!   asks. Standard input and standard output must both be terminals: with
//!   either redirected nothing is asked and nothing approved, and the
//!   refusal names the command to run at a terminal. A password is never
//!   read from standard input, an argument or a variable.
//! - **Not while a debugger is attached** ([`traced`], the process
//!   hardening's check): it could read the password as it is typed.
//! - **`setup` and `reset` take the terminal before anything slow** (the
//!   core's start), as `auth login` does, so that a password typed early is
//!   not echoed by the terminal's own line mode.
//! - **A subcommand that signs** ([`key_at_terminal`]) shows what it
//!   approves first, then asks for the password with lines that name it:
//!   `Enter` is the yes, `Esc` the no. With no key set up, the yes sets one
//!   up: a password chosen, typed twice. A password that is not the
//!   person's asks again, [`TRIES`] times in all.
//! - **`status` only reads:** it types no password and checks nothing.

use std::io::{self, IsTerminal};
use std::sync::Arc;

use cineris_base::Password;
use cineris_config::{Env, LoadOptions};
use cineris_core::signed::PersonKey;
use cineris_core::{ApprovalsHeld, Core, CoreOptions, HeldKey, KeyRefusal, TokioTimer};
use cineris_tui::secret::{PasswordAsk, PasswordTerminal, Purpose};

use crate::args::ApprovalsCommand;
use crate::exit;

/// A finished subcommand: its exit code.
type Done = u8;

/// How many passwords a subcommand that signs takes before it gives up.
pub(crate) const TRIES: usize = 3;

/// Whether a debugger or a tracer is attached to this process, which could
/// read the password as it is typed: the process hardening's check
/// (`cineris_sandbox::harden::traced`, which on Linux also fails closed
/// when the process's status cannot be read). Every prompt for the
/// password asks it here first, so that the check is wired in one place.
pub(crate) fn traced() -> bool {
    cineris_sandbox::harden::traced()
}

/// Why no password is asked while a debugger is attached.
pub(crate) const TRACED: &str = "a debugger is attached to Cineris, and could read the password \
                                 as it is typed: no password is asked while one is";

/// Refused with [`TRACED`] when `traced` says a debugger is attached.
pub(crate) fn untraced(traced: impl FnOnce() -> bool) -> Result<(), String> {
    if traced() {
        Err(TRACED.to_owned())
    } else {
        Ok(())
    }
}

/// Whether standard input and standard output are both terminals: where
/// the password can be asked.
pub(crate) fn at_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Why nothing is approved without a terminal, for the command `command`
/// run as `name`: what is said after "was not …:".
pub(crate) fn no_terminal(name: &str, command: &str) -> String {
    format!(
        "an approval is signed with your approvals password, typed at a terminal, and standard \
         input and output are not both one here. Run `{name} {command}` in your own terminal. A \
         hash given with --hash, --extension or --hook checks what is shown, and is not an \
         approval on its own"
    )
}

/// Why the person's key was not made at the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotSigned {
    /// `Esc` or `Ctrl+C`: no.
    Declined,
    /// It could not be asked or made: why, after a colon.
    Refused(String),
    /// A signal came while it was asked: the code to end with.
    Signal(u8),
}

/// The person's key, from the password typed at the terminal, for a
/// change of their approvals that is signed, after what it approves has
/// been shown: `lines` name it under the prompt's sentence. With no key
/// set up, the yes sets one up first (a password chosen, typed twice); a
/// password that is not the person's asks again, [`TRIES`] in all. Asked
/// on the terminal the subcommand took before anything slow ([`Taken`]),
/// which stays taken. Drop the key as soon as it has signed.
///
/// # Errors
///
/// [`NotSigned`]: declined, refused (no terminal, a debugger, the key file,
/// too many tries), or a signal.
pub(crate) fn key_at_terminal(
    core: &Core,
    taken: Option<&mut Taken>,
    lines: &[String],
) -> Result<PersonKey, NotSigned> {
    untraced(traced).map_err(NotSigned::Refused)?;
    let Some(taken) = taken else {
        return Err(NotSigned::Refused(
            "the terminal could not be taken to ask for your approvals' password".to_owned(),
        ));
    };
    let set_up = core
        .approval_key()
        .map_err(|why| NotSigned::Refused(why.to_string()))?
        .is_none();
    let Some(terminal) = taken.terminal.as_mut() else {
        return Err(NotSigned::Refused(
            "the terminal could not be taken to ask for your approvals' password".to_owned(),
        ));
    };
    let key = (|| {
        if set_up {
            let mut lines = lines.to_vec();
            lines.push(cineris_tui::app::approvals::SETS_UP.to_owned());
            let password = read(terminal, &PasswordAsk::new(Purpose::Choose, lines))?;
            return core
                .set_up_approvals(password.expose_bytes())
                .map_err(|why| NotSigned::Refused(why.to_string()));
        }
        let mut warning: Option<String> = None;
        for _ in 0..TRIES {
            let mut ask = PasswordAsk::new(Purpose::Sign, lines.to_vec());
            if let Some(warning) = warning.take() {
                ask = ask.warning(warning);
            }
            let password = read(terminal, &ask)?;
            match core.person_key(password.expose_bytes()) {
                Ok(key) => return Ok(key),
                Err(refusal @ (KeyRefusal::NotThePassword { .. } | KeyRefusal::NotTheOpenKey)) => {
                    warning = Some(refusal.to_string());
                }
                Err(other) => return Err(NotSigned::Refused(other.to_string())),
            }
        }
        Err(NotSigned::Refused(format!(
            "{TRIES} passwords were typed, and none was the password of your approvals"
        )))
    })();
    if let Some(code) = cineris_tui::app::signalled() {
        cineris_tui::app::signal_ended();
        return Err(NotSigned::Signal(code));
    }
    key
}

/// Whether the terminal is taken for the approvals' password, in raw mode:
/// what the subcommand writes meanwhile goes with `\r\n` for each line
/// break ([`crate::print_out`], [`crate::print_err`]).
static RAW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the terminal is taken in raw mode now.
pub(crate) fn raw() -> bool {
    RAW.load(std::sync::atomic::Ordering::SeqCst)
}

/// The terminal a subcommand that may ask for the approvals' password took
/// before anything slow (the core's start, the key file's read), so that a
/// password typed early is never echoed by the terminal's own line mode.
/// While it is taken, the terminal echoes nothing, and what the subcommand
/// writes is written as raw mode needs. Dropped, it is put back.
pub(crate) struct Taken {
    terminal: Option<PasswordTerminal>,
    overrides: cineris_tui::app::Overrides,
}

impl Taken {
    /// The terminal, taken with `overrides`, when standard input and
    /// standard output are both terminals; `None` otherwise, or when it
    /// cannot be taken.
    pub(crate) fn take(overrides: cineris_tui::app::Overrides) -> Option<Self> {
        if !at_terminal() {
            return None;
        }
        let terminal = PasswordTerminal::enter(overrides).ok()?;
        RAW.store(true, std::sync::atomic::Ordering::SeqCst);
        Some(Self {
            terminal: Some(terminal),
            overrides,
        })
    }

    /// A yes or no that comes before the password (`permissions trust`'s, of
    /// a project's extensions): asked in the terminal's own line mode, its
    /// answer echoed, since it is no secret, and the terminal taken again
    /// before anything else is read (the key file, the password). A yes is
    /// `y` or `yes`; anything else, an empty line included, is no.
    pub(crate) fn confirm(&mut self, question: &str) -> bool {
        if let Some(terminal) = self.terminal.take() {
            let _ = terminal.leave();
        }
        RAW.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::print_out(&format!("{question} [y/N] "));
        let mut line = String::new();
        let yes = io::stdin().read_line(&mut line).is_ok()
            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
        self.terminal = PasswordTerminal::enter(self.overrides).ok();
        RAW.store(self.terminal.is_some(), std::sync::atomic::Ordering::SeqCst);
        yes
    }
}

impl Drop for Taken {
    fn drop(&mut self) {
        if let Some(terminal) = self.terminal.take() {
            let _ = terminal.leave();
        }
        RAW.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// What a headless run says when it has approvals to use and no terminal to
/// ask for the password at.
pub(crate) const NO_TERMINAL: &str = "no terminal to ask your approvals' password at: your \
     extensions and this project's trust are not used in this run";

/// What a headless run says when the person went on without their
/// approvals.
pub(crate) const WITHOUT: &str = "this run goes on without your approvals: your extensions and \
     this project's trust are not used in it";

/// A headless run's approvals, once its core has started and before its
/// session opens (the signed approvals step): when something carries a
/// signature to use ([`ApprovalsHeld::to_use`]) and the run took the
/// terminal for it (`terminal`, taken before anything slow), the password
/// is asked for with `Unlock` and the approvals opened with the key it
/// makes. The terminal is put back before this returns. What to say on
/// standard error once, when the run goes on without them: no terminal,
/// `Esc`, a debugger, or the passwords typed were not the person's.
pub(crate) fn open_at_start(
    core: &Core,
    project: Option<&cineris_core::Project>,
    terminal: Option<PasswordTerminal>,
) -> Option<String> {
    let to_use = core.approvals_held(project).is_ok_and(|held| held.to_use());
    let mut terminal = match terminal {
        _ if !to_use => return None,
        None => return Some(NO_TERMINAL.to_owned()),
        Some(terminal) => terminal,
    };
    let said = (|| {
        untraced(traced).map_err(|why| {
            format!("{why}: your extensions and this project's trust are not used in this run")
        })?;
        let mut warning: Option<String> = None;
        for _ in 0..TRIES {
            let mut ask = PasswordAsk::new(
                Purpose::Unlock,
                vec![cineris_tui::app::approvals::UNLOCKS.to_owned()],
            );
            if let Some(warning) = warning.take() {
                ask = ask.warning(warning);
            }
            let password = match read(&mut terminal, &ask) {
                Ok(password) => password,
                Err(NotSigned::Declined | NotSigned::Signal(_)) => return Err(WITHOUT.to_owned()),
                Err(NotSigned::Refused(why)) => return Err(format!("{why}; {WITHOUT}")),
            };
            match core.person_key(password.expose_bytes()) {
                Ok(key) => {
                    return core
                        .open_approvals(&key)
                        .map_err(|why| format!("{why}; {WITHOUT}"));
                }
                Err(refusal @ (KeyRefusal::NotThePassword { .. } | KeyRefusal::NotTheOpenKey)) => {
                    warning = Some(refusal.to_string());
                }
                Err(other) => return Err(format!("{other}; {WITHOUT}")),
            }
        }
        Err(format!(
            "{TRIES} passwords were typed, and none was the password of your approvals; \
             {WITHOUT}"
        ))
    })()
    .err();
    let _ = terminal.leave();
    said
}

/// One password read on `terminal`: `Esc` or `Ctrl+C` is no.
fn read(terminal: &mut PasswordTerminal, ask: &PasswordAsk) -> Result<Password, NotSigned> {
    match terminal.read(ask) {
        Ok(Some(password)) => Ok(password),
        Ok(None) => match terminal.ending() {
            Some(code) => Err(NotSigned::Signal(code)),
            None => Err(NotSigned::Declined),
        },
        Err(error) => Err(NotSigned::Refused(format!(
            "the terminal could not be read: {error}"
        ))),
    }
}

fn say(name: &str, text: &str) {
    eprintln!("{name}: {}", crate::visible(text));
}

/// `cineris approvals …`
pub(crate) fn approvals(command: ApprovalsCommand, name: &'static str) -> Done {
    let done =
        match command {
            ApprovalsCommand::Status => status(name),
            ApprovalsCommand::Setup => crate::not_under_a_session(name, "approvals setup")
                .and_then(|()| choose(name, false)),
            ApprovalsCommand::Reset => crate::not_under_a_session(name, "approvals reset")
                .and_then(|()| choose(name, true)),
        };
    match done {
        Ok(code) | Err(code) => code,
    }
}

/// A core as the other subcommands start one, given `loaded` when the
/// configuration was read already (held just before it starts by
/// `cineris_base::hook::hold_core`, for the tests of type-ahead).
fn core(name: &str, loaded: Option<cineris_config::Loaded>) -> Result<Core, Done> {
    let mut options = CoreOptions::new(Env::from_process(), Arc::new(TokioTimer));
    options.loaded = loaded;
    cineris_base::hook::hold_core();
    Core::start(options).map_err(|error| {
        say(name, &error.message);
        exit::USAGE
    })
}

/// `approvals setup`, or with `reset` `approvals reset`: a password chosen
/// at the terminal, taken before the core starts.
fn choose(name: &str, reset: bool) -> Result<Done, Done> {
    let what = if reset {
        "approvals reset"
    } else {
        "approvals setup"
    };
    let refused = |why: &str| {
        say(
            name,
            &format!(
                "{}: {why}",
                if reset {
                    "no new password was set, and nothing was changed"
                } else {
                    "no password was set"
                }
            ),
        );
        exit::USAGE
    };
    untraced(traced).map_err(|why| refused(&why))?;
    if !at_terminal() {
        return Err(refused(&format!(
            "the password is typed at a terminal, never read from standard input or an \
             argument, and standard input and output are not both one here. Run `{name} {what}` \
             in your own terminal"
        )));
    }
    let env = Env::from_process();
    let loaded = cineris_config::load(&LoadOptions::new(env.clone())).map_err(|failure| {
        say(name, &failure.render(&env));
        exit::USAGE
    })?;
    let overrides = crate::interactive::overrides(&loaded.settings.terminal);
    // Taken before the core starts: from here the terminal echoes nothing.
    let mut terminal =
        Taken::take(overrides).ok_or_else(|| refused("the terminal could not be taken"))?;
    let core = match core(name, Some(loaded)) {
        Ok(core) => core,
        Err(code) => {
            drop(terminal);
            return Err(code);
        }
    };
    if !reset {
        match core.approval_key() {
            Ok(None) => {}
            Ok(Some(_)) => {
                drop(terminal);
                return Err(refused(&KeyRefusal::KeyExists.to_string()));
            }
            Err(why) => {
                drop(terminal);
                return Err(refused(&why.to_string()));
            }
        }
    }
    let ask = if reset {
        PasswordAsk::new(
            Purpose::Choose,
            vec!["A new password for your approvals, for one you have forgotten.".to_owned()],
        )
        .warning(RESET_WARNING)
    } else {
        PasswordAsk::new(
            Purpose::Choose,
            vec![
                "Every approval you give from now on is signed with it: a project's trust, and \
                 each extension you add."
                    .to_owned(),
            ],
        )
    };
    let read = match terminal.terminal.as_mut() {
        Some(taken) => read(taken, &ask),
        None => Err(NotSigned::Refused(
            "the terminal could not be taken".to_owned(),
        )),
    };
    drop(terminal);
    if let Some(code) = cineris_tui::app::signalled() {
        cineris_tui::app::signal_ended();
        return Err(code);
    }
    let password = match read {
        Ok(password) => password,
        Err(NotSigned::Signal(code)) => return Err(code),
        Err(NotSigned::Declined) => return Err(refused("you went back")),
        Err(NotSigned::Refused(why)) => return Err(refused(&why)),
    };
    if reset {
        // Neither the key nor the password is kept past the reset.
        let (_, report) = core
            .reset_approvals(password.expose_bytes())
            .map_err(|why| refused(&why.to_string()))?;
        drop(password);
        crate::print_out(&reset_text(&key_said(&core), &report));
    } else {
        core.set_up_approvals(password.expose_bytes())
            .map_err(|why| refused(&why.to_string()))?;
        drop(password);
        crate::print_out(&format!(
            "Your approvals password is set ({}). Each approval you give from now on asks for \
             it, and a session that has approvals to use asks for it when it opens.\n",
            key_said(&core)
        ));
    }
    Ok(exit::COMPLETED)
}

/// What `approvals reset` warns of before the new password is chosen.
pub(crate) const RESET_WARNING: &str = "Every approval you gave asks again: each project's trust \
     and each extension you approved. What they let run stays held back until you approve it \
     again.";

/// The key the file is of, as `key 0123abcd…, made …`.
fn key_said(core: &Core) -> String {
    match core.approval_key() {
        Ok(Some(file)) => format!("key {}, made {}", file.fingerprint(), file.made_at()),
        _ => "its key could not be read back".to_owned(),
    }
}

/// What `approvals reset` says it did: and each store it left as it was
/// (`ResetReport::left`, a sentence of the core's that names its file and
/// why), shown as text, which counts for nothing under the new key.
fn reset_text(key: &str, report: &cineris_core::ResetReport) -> String {
    let mut text = format!(
        "A new password is set for your approvals ({key}). The signature was taken off {} of \
         your own pins and {} projects' trust: each asks again when it is next used, and what \
         it let run stays held back until you approve it again.\n",
        report.pins, report.projects
    );
    for left in &report.left {
        text.push_str(&format!("Left as it was: {}\n", crate::visible(left)));
    }
    text
}

/// `approvals status`: what the key file and the stores hold, checking
/// nothing.
fn status(name: &str) -> Result<Done, Done> {
    let core = core(name, None)?;
    let held = core.approvals_held(None).map_err(|why| {
        say(name, &why.to_string());
        exit::INTERNAL
    })?;
    crate::print_out(&crate::visible(&status_text(&held, name)));
    Ok(exit::COMPLETED)
}

/// What `approvals status` prints, run as `name`.
pub(crate) fn status_text(held: &ApprovalsHeld, name: &str) -> String {
    let mut text = match &held.key {
        HeldKey::None => format!(
            "Approvals password: not set. `{name} approvals setup` sets one, and so does the \
             first approval you give.\n"
        ),
        HeldKey::Made {
            fingerprint,
            made_at,
        } => format!("Approvals password: set, key {fingerprint}, made {made_at}.\n"),
        HeldKey::Unreadable(why) => {
            format!("Approvals password: its key file could not be read: {why}\n")
        }
    };
    text.push_str(&match (held.pins, held.pins_carry_signature) {
        (0, _) => "Your own pins (pins.toml): none.\n".to_owned(),
        (pins, true) => {
            format!("Your own pins (pins.toml): {pins}, carrying a signature, not checked.\n")
        }
        (pins, false) => {
            format!("Your own pins (pins.toml): {pins}, carrying no signature: each asks again.\n")
        }
    });
    text.push_str(
        &match (held.projects_carrying, held.projects_not_carrying) {
            (0, 0) => "Projects' trust (trust.toml): none.\n".to_owned(),
            (carrying, not) => format!(
                "Projects' trust (trust.toml): {carrying} carrying a signature, not checked; {not} \
             carrying none, which ask again.\n"
            ),
        },
    );
    text.push_str(
        "This status types no password and checks no signature: a signature counts only when \
         the key your password makes checks it, in a session or at an approval.\n",
    );
    text
}

#[cfg(test)]
mod tests;
