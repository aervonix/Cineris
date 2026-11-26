//! Custom commands (step 28): the user's, a project's and an extension's,
//! as `commands/list` gives them for the session open
//! (`docs/architecture/09-tui.md`, "Slash commands";
//! `docs/architecture/12-extensions.md`).
//!
//! - **The list** is asked for once the session is open, and again each
//!   time the `/` menu opens: it is a read beside the lane, and cheap. The
//!   last one answered is kept; one that fails leaves what was there. A
//!   command named like a built-in, or like one of 09-tui's commands this
//!   build does not have yet, in any case of ASCII letters (`Help`), is not
//!   offered: the built-in wins, typed or chosen, and nothing is said.
//! - **Not used.** The commands the list says are not used, and why, are
//!   said once a session, one line each, and `/help` lists them, marked.
//! - **The menu** lists them after the built-ins, each with its
//!   description; a project's (a file's or a project extension's) is noted
//!   `(project's)` before it, so that a cut row never loses the note. So
//!   does `/help`.
//! - **Sending.** `/name rest`, `name` in the last list, used or not, goes
//!   as that command (`turn/start`'s `command`, `input` empty), as a prompt
//!   goes: steering or queued while a turn runs, its typed text given back
//!   with the core's sentence when it is refused (one not used is refused
//!   with why, and never reaches the model as text). So does any `/a:b`,
//!   an extension's command's form, listed or not: one held, changed or
//!   turned off is in neither list, and the core refuses it with why. With
//!   no list of the session's (the first failed, or did not answer within
//!   [`COMMANDS_WAIT`]), every `/name` that is not a built-in's goes as a
//!   command, for the core to judge. After a list, a name in neither of its
//!   lists is text, as before; a name with a `/` (a path) always is. The
//!   interrupt key while its `turn/start` is on its way (an extension's
//!   command still running) interrupts, as while a turn runs; the refusal
//!   that follows (`command_failed`, `interrupted`) gives the typed text
//!   back, says nothing, since the person stopped it, and stops the
//!   spinner. A command is never sent again: refused `turn_running`, it
//!   comes back with why, and held at an interrupt it comes back unsent,
//!   since its script would run (again, or after the stop).
//! - **Given back.** A prompt a command made that comes back into the
//!   editor, a hook's refusal (`prompt_returned`), an interrupt's
//!   undelivered steering message or a rewind's prompt, comes back as it
//!   was typed, `/name args`, in place of the command's text.
//! - **Waiting for the list.** A prompt that may be a custom command, held
//!   while the session opened (the command line's among them) or typed
//!   before its first list answered, waits for that list, at most
//!   [`COMMANDS_WAIT`] on the application's clock, so that it is sent as
//!   what it is: until then the list stands where the held prompt on its
//!   way does (`held_sent`), and what waits for that waits for it. At the
//!   bound they go all the same, and the list that answers later changes
//!   the menu only. An interrupt meanwhile gives them back, none having
//!   been sent.

use std::time::{Duration, Instant};

use cineris_protocol::error::code;
use cineris_protocol::item::PromptCommand;
use cineris_protocol::message::Part;
use cineris_protocol::methods::{
    CommandCall, CommandInfo, CommandNotUsed, CommandsListParams, Op, OpResult,
};
use cineris_protocol::{ErrorData, ErrorKind, SessionId};

use super::super::items::{owner_note, typed, whose};
use super::{App, COMMANDS, NOT_YET, Pending, Sent, Ticket, wire};
use crate::complete::{Command, Kind};
use crate::input::Delivery;

/// How long a prompt that may be a custom command waits for the session's
/// first list, on the application's clock (the loop's, or a test's): a
/// read beside the lane answers well within it, and a core that cannot
/// answer it holds nobody's prompt for longer. The prompt then goes all
/// the same, a `/name` as a command for the core to judge.
pub(crate) const COMMANDS_WAIT: Duration = Duration::from_secs(3);

/// What the application knows of the session's custom commands.
#[derive(Debug, Default)]
pub(super) struct Custom {
    /// The last list answered for the session open, the names of built-ins
    /// left out.
    list: Vec<CommandInfo>,
    /// The commands the last list says are not used, and why: typed, one
    /// goes as the command, for the core to refuse with why.
    not_used: Vec<CommandNotUsed>,
    /// The session whose commands not used were said: once a session.
    not_used_said: Option<SessionId>,
    /// The session a list was answered well for: a name in neither of its
    /// lists is then text. Until then (the first failed, or has not
    /// answered) a `/name` goes as a command, for the core to judge. A list
    /// that fails after it leaves it, as it leaves the menu's.
    listed: Option<SessionId>,
    /// The session whose prompts no longer wait for its list: it was
    /// answered, well or not, or a wait for it reached its bound.
    waited: Option<SessionId>,
    /// The list asked for and not yet answered: one at a time.
    asked: Option<Ticket>,
    /// Whether the `/` menu was open at the last frame: one that opens asks
    /// again.
    menu_open: bool,
    /// The prompts that wait for the list on its way, `held_sent` being its
    /// ticket: how the first of them goes, and until when they wait.
    held: Option<Held>,
}

/// A wait for the session's list.
#[derive(Debug)]
struct Held {
    /// The session whose list it is.
    session: SessionId,
    /// How the first prompt that waits goes.
    next: Next,
    /// Its bound, [`COMMANDS_WAIT`] after it began: they go then, answered
    /// or not.
    until: Instant,
}

/// How the first prompt that waits for the list goes.
#[derive(Debug, Clone, Copy)]
enum Next {
    /// Held while the session opened: as the first of them, or behind it.
    Opening { first: bool },
    /// Typed once it was open: as its key sent it.
    Typed(Delivery),
}

/// Whether `name` is a built-in's, or one of 09-tui's commands this build
/// does not have yet, in any case of ASCII letters: a custom command never
/// takes it.
fn built_in(name: &str) -> bool {
    COMMANDS
        .iter()
        .any(|(known, _)| known.eq_ignore_ascii_case(name))
        || NOT_YET.iter().any(|known| known.eq_ignore_ascii_case(name))
}

/// What a `/name` is whatever the list says: never a command when the name
/// is a built-in's, or holds a `/` (a path: no command's name has one, a
/// file's or an extension's); always when it has an extension's form,
/// `a:b`, since an extension not used (held, changed, turned off) is in
/// neither list and the core says why. `None` when the list decides.
fn regardless(name: &str) -> Option<bool> {
    if built_in(name) || name.contains('/') {
        Some(false)
    } else if name
        .split_once(':')
        .is_some_and(|(extension, command)| !extension.is_empty() && !command.is_empty())
    {
        Some(true)
    } else {
        None
    }
}

/// The name a `/` prompt names, and the rest of its text.
fn named(text: &str) -> Option<(&str, &str)> {
    let command = text.strip_prefix('/')?;
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    (!name.is_empty()).then_some((name, rest))
}

/// What a prompt handed back puts in the editor, and how many of its parts
/// it leaves out: its text parts, one to a line, and not the rest (images);
/// or, for one a command made, what was typed, `/name args`, in place of
/// the command's text, which leaves nothing out.
pub(super) fn given_back(parts: &[Part], command: Option<&PromptCommand>) -> (String, usize) {
    if let Some(command) = command {
        return (typed(command), 0);
    }
    let text: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    let left_out = parts.len() - text.len();
    (text.join("\n"), left_out)
}

/// Whether a prompt's refusal is a command's that the person stopped
/// (`command_failed`, `interrupted`): its text comes back, and nothing is
/// said.
pub(super) fn stopped_command(sent: Option<&CommandCall>, error: &ErrorData) -> bool {
    sent.is_some() && error.kind == ErrorKind::Interrupted && error.code == code::COMMAND_FAILED
}

/// How a command not used is named: `/name`, or its file when it has no
/// name.
fn not_used_name(command: &CommandNotUsed) -> String {
    match &command.path {
        _ if !command.name.is_empty() => format!("/{}", command.name),
        Some(path) => path.clone(),
        None => "(no name)".to_owned(),
    }
}

/// A custom command's note in the menu and in `/help`: its description,
/// `(project's)` before it when it is a project's.
fn note(command: &CommandInfo) -> String {
    match owner_note(&command.source) {
        Some(owner) if command.description.is_empty() => format!("({owner})"),
        Some(owner) => format!("({owner}) {}", command.description),
        None => command.description.clone(),
    }
}

impl App {
    /// Asks for the session's custom commands, unless an ask is on its
    /// way.
    pub(super) fn ask_commands(&mut self) {
        if self.custom.asked.is_some() {
            return;
        }
        let Some(session) = self.session_id() else {
            return;
        };
        let ticket = self.link.send(Op::CommandsList(CommandsListParams {
            session_id: session.clone(),
        }));
        self.pending.insert(ticket, Pending::Commands(session));
        self.custom.asked = Some(ticket);
    }

    /// Before a frame: the `/` menu that has just opened asks for the list
    /// again.
    pub(super) fn ask_commands_for_menu(&mut self) {
        let open = self
            .input
            .menu()
            .is_some_and(|menu| menu.kind == Kind::Commands);
        if open && !self.custom.menu_open {
            self.ask_commands();
        }
        self.custom.menu_open = open;
    }

    /// The session left: its commands are not the next one's.
    pub(super) fn forget_commands(&mut self) {
        let had = !self.custom.list.is_empty();
        self.custom.list.clear();
        self.custom.not_used.clear();
        self.custom.listed = None;
        self.custom.waited = None;
        self.custom.asked = None;
        if had {
            self.input.set_commands(self.menu_commands());
        }
    }

    /// `commands/list` answered. The list of the session open is kept, and
    /// the menu given it when it changed; a failure keeps what was there,
    /// and says nothing, since nobody asked. The commands not used are said
    /// the first time a list of the session has some. The held prompts that
    /// waited for it go.
    pub(super) fn commands_listed(
        &mut self,
        ticket: Ticket,
        session: &SessionId,
        result: Result<OpResult, ErrorData>,
        now: Instant,
    ) {
        if self.custom.asked == Some(ticket) {
            self.custom.asked = None;
        }
        let open = self.is_open(session);
        if open {
            if let Ok(OpResult::CommandsList(listed)) = result {
                let list: Vec<CommandInfo> = listed
                    .commands
                    .into_iter()
                    .filter(|command| !built_in(&command.name))
                    .collect();
                if list != self.custom.list {
                    self.custom.list = list;
                    self.input.set_commands(self.menu_commands());
                }
                self.custom.not_used = listed.not_used;
                if !self.custom.not_used.is_empty()
                    && self.custom.not_used_said.as_ref() != Some(session)
                {
                    self.custom.not_used_said = Some(session.clone());
                    let lines: Vec<String> = self
                        .custom
                        .not_used
                        .iter()
                        .map(|command| {
                            format!(
                                "{} ({}) is not used: {}",
                                not_used_name(command),
                                whose(&command.source, command.extension.as_deref()),
                                command.reason
                            )
                        })
                        .collect();
                    for line in lines {
                        self.say(&line);
                    }
                }
                self.custom.listed = Some(session.clone());
            }
            self.custom.waited = Some(session.clone());
        }
        if self.held_sent != Some(ticket) {
            return;
        }
        self.held_sent = None;
        let next = self
            .custom
            .held
            .take()
            .map_or(Next::Opening { first: false }, |held| held.next);
        self.wait_over(next, open, now);
    }

    /// The application's clock at `now`: a wait for the list that reached
    /// its bound unanswered is over, and the prompts that waited go, a
    /// `/name` as a command for the core to judge; the list's answer, when
    /// it comes, changes the menu only. Whether one was.
    pub(super) fn commands_wait_over(&mut self, now: Instant) -> bool {
        let Some(held) = self.custom.held.take_if(|held| held.until <= now) else {
            return false;
        };
        self.held_sent = None;
        let open = self.is_open(&held.session);
        if open {
            self.custom.waited = Some(held.session);
        }
        self.wait_over(held.next, open, now);
        true
    }

    /// When a wait for the list reaches its bound, if one is under way: the
    /// loop wakes for it.
    pub(super) fn commands_wait_ends(&self) -> Option<Instant> {
        self.custom.held.as_ref().map(|held| held.until)
    }

    /// A wait for the list is over, answered or not: the prompts that
    /// waited go, the first as `next` says, or come back when the session
    /// was left meanwhile.
    fn wait_over(&mut self, next: Next, open: bool, now: Instant) {
        if open {
            match next {
                Next::Opening { first } => self.send_held(first, now),
                Next::Typed(delivery) => self.send_typed(delivery),
            }
            // Every one held was a built-in: no turn is on its way.
            if self.held_sent.is_none() {
                self.stop_running_unless_a_turn_runs();
            }
        } else {
            self.give_back_held();
            self.stop_running_unless_a_turn_runs();
        }
    }

    /// Sends the prompt typed while the list was on its way, the first of
    /// those held, as its key sent it; those typed after it follow it, as
    /// held prompts do.
    fn send_typed(&mut self, delivery: Delivery) {
        let Some(text) = self.held_prompts.pop_front() else {
            return;
        };
        let Some(session) = self.session_id() else {
            self.held_prompts.push_front(text);
            self.give_back_held();
            return;
        };
        let sent = Sent {
            session,
            command: self.custom_call(&text),
            text,
            started: delivery == Delivery::Prompt,
            stopped: false,
            again: false,
        };
        self.held_sent = Some(self.send_prompt(sent, wire(delivery)));
    }

    /// The commands the `/` menu offers: the built-ins, then the custom
    /// ones of the last list.
    pub(super) fn menu_commands(&self) -> Vec<Command> {
        COMMANDS
            .iter()
            .map(|(name, description)| Command {
                name: (*name).to_owned(),
                description: (*description).to_owned(),
            })
            .chain(self.custom.list.iter().map(|command| Command {
                name: command.name.clone(),
                description: note(command),
            }))
            .collect()
    }

    /// `/help`'s lines for the custom commands, as the built-ins' are, then
    /// those not used, marked, with why.
    pub(super) fn custom_help(&self) -> Vec<String> {
        let used = self
            .custom
            .list
            .iter()
            .map(|command| format!("/{:<12} {}", command.name, note(command)));
        let not_used = self.custom.not_used.iter().map(|command| {
            let owner = owner_note(&command.source)
                .map(|owner| format!("({owner}) "))
                .unwrap_or_default();
            format!(
                "{:<13} {owner}not used: {}",
                not_used_name(command),
                command.reason
            )
        });
        used.chain(not_used).collect()
    }

    /// The command `text` sends, when it names one of the last list: one
    /// it says is not used too, which the core refuses with why, rather
    /// than the model being sent `/name args` as text. A name typed is
    /// never empty, so one not used that has none is never it. A name of
    /// an extension's form, `a:b`, is sent listed or not, and so is every
    /// `/name` while the session has no list (see [`regardless`]): the core
    /// judges it.
    pub(super) fn custom_call(&self, text: &str) -> Option<CommandCall> {
        let (name, rest) = named(text)?;
        let known = self.custom.list.iter().any(|command| command.name == name)
            || self
                .custom
                .not_used
                .iter()
                .any(|command| command.name == name);
        let unlisted = regardless(name).unwrap_or_else(|| !self.listed());
        ((!built_in(name) && known) || unlisted).then(|| CommandCall {
            name: name.to_owned(),
            args: rest.trim().to_owned(),
        })
    }

    /// Whether a list was answered well for the session open.
    fn listed(&self) -> bool {
        self.custom
            .listed
            .as_ref()
            .is_some_and(|session| self.is_open(session))
    }

    /// Whether the held prompt `text` waits for the session's first list,
    /// which is on its way: it may be one of its commands. It then stands
    /// where the held prompt on its way does, and the held prompts go,
    /// `first` or not, when it is answered, or at the wait's bound.
    pub(super) fn held_waits_for_commands(
        &mut self,
        text: &str,
        first: bool,
        now: Instant,
    ) -> bool {
        self.waits_for_commands(text, Next::Opening { first }, now)
    }

    /// Whether `text`, typed at `at` with the session open and no prompt
    /// held, waits for the session's first list, as a held one does: it
    /// may be one of its commands. It then goes as `delivery` says when the
    /// list is answered, or at the wait's bound; it is the caller's to hold.
    pub(super) fn typed_waits_for_commands(
        &mut self,
        text: &str,
        delivery: Delivery,
        at: Instant,
    ) -> bool {
        self.waits_for_commands(text, Next::Typed(delivery), at)
    }

    /// Whether `text` waits for the session's first list, which is on its
    /// way, a wait that began at `now` and goes first as `next` says.
    fn waits_for_commands(&mut self, text: &str, next: Next, now: Instant) -> bool {
        let Some(asked) = self.custom.asked else {
            return false;
        };
        let Some(session) = self.session_id() else {
            return false;
        };
        let waited = self.custom.waited.as_ref() == Some(&session);
        if waited || !named(text).is_some_and(|(name, _)| regardless(name).is_none()) {
            return false;
        }
        self.held_sent = Some(asked);
        self.custom.held = Some(Held {
            session,
            next,
            until: now + COMMANDS_WAIT,
        });
        true
    }

    /// An interrupt while the held prompts (or one typed) wait for the
    /// list: none was sent, so they come back, as those held before the
    /// session opened do. Whether they waited.
    pub(super) fn held_for_commands_back(&mut self) -> bool {
        if self.custom.held.take().is_none() {
            return false;
        }
        self.held_sent = None;
        self.give_back_held();
        true
    }
}
