//! Custom commands (step 28): the user's, a project's and an extension's,
//! as `commands/list` gives them for the session open
//! (`docs/architecture/09-tui.md`, "Slash commands";
//! `docs/architecture/12-extensions.md`).
//!
//! - **The list** is asked for once the session is open, and again each
//!   time the `/` menu opens: it is a read beside the lane, and cheap. The
//!   last one answered is kept; one that fails leaves what was there. A
//!   command named like a built-in, or like one of 09-tui's commands this
//!   build does not have yet, is not offered: the built-in wins, typed or
//!   chosen, and nothing is said.
//! - **The menu** lists them after the built-ins, each with its
//!   description; a project's (a file's or a project extension's) is noted
//!   `(project's)` before it, so that a cut row never loses the note. So
//!   does `/help`.
//! - **Sending.** `/name rest`, `name` in the last list, goes as that
//!   command (`turn/start`'s `command`, `input` empty), as a prompt goes:
//!   steering or queued while a turn runs, its typed text given back with
//!   the core's sentence when it is refused. Anything else goes as before.
//! - **Held prompts.** A prompt held while the session opened (the command
//!   line's among them) that may be a custom command waits for the
//!   session's first list, so that it is sent as what it is: until then the
//!   list stands where the held prompt on its way does (`held_sent`), and
//!   what waits for that waits for it. An interrupt meanwhile gives them
//!   back, none having been sent.

use std::time::Instant;

use cineris_protocol::methods::{CommandCall, CommandInfo, CommandsListParams, Op, OpResult};
use cineris_protocol::{ErrorData, SessionId};

use super::super::items::owner_note;
use super::{App, COMMANDS, NOT_YET, Pending, Ticket};
use crate::complete::{Command, Kind};

/// What the application knows of the session's custom commands.
#[derive(Debug, Default)]
pub(super) struct Custom {
    /// The last list answered for the session open, the names of built-ins
    /// left out.
    list: Vec<CommandInfo>,
    /// The session whose list was answered, well or not: a held prompt
    /// that may be one of its commands waits until then.
    answered: Option<SessionId>,
    /// The list asked for and not yet answered: one at a time.
    asked: Option<Ticket>,
    /// Whether the `/` menu was open at the last frame: one that opens asks
    /// again.
    menu_open: bool,
    /// The held prompts wait for the list on its way, `held_sent` being its
    /// ticket: whether the next is sent as the first.
    held: Option<bool>,
}

/// Whether `name` is a built-in's, or one of 09-tui's commands this build
/// does not have yet: a custom command never takes it.
fn built_in(name: &str) -> bool {
    COMMANDS.iter().any(|(known, _)| *known == name) || NOT_YET.contains(&name)
}

/// The name a `/` prompt names, and the rest of its text.
fn named(text: &str) -> Option<(&str, &str)> {
    let command = text.strip_prefix('/')?;
    let (name, rest) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    (!name.is_empty()).then_some((name, rest))
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
        self.custom.answered = None;
        self.custom.asked = None;
        if had {
            self.input.set_commands(self.menu_commands());
        }
    }

    /// `commands/list` answered. The list of the session open is kept, and
    /// the menu given it when it changed; a failure keeps what was there,
    /// and says nothing, since nobody asked. The held prompts that waited
    /// for it go.
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
            }
            self.custom.answered = Some(session.clone());
        }
        if self.held_sent != Some(ticket) {
            return;
        }
        self.held_sent = None;
        let first = self.custom.held.take().unwrap_or(false);
        if open {
            self.send_held(first, now);
            // Every one held was a built-in: no turn is on its way.
            if self.held_sent.is_none() {
                self.stop_running_unless_a_turn_runs();
            }
        } else {
            self.give_back_held();
            self.stop_running_unless_a_turn_runs();
        }
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

    /// `/help`'s lines for the custom commands, as the built-ins' are.
    pub(super) fn custom_help(&self) -> Vec<String> {
        self.custom
            .list
            .iter()
            .map(|command| format!("/{:<12} {}", command.name, note(command)))
            .collect()
    }

    /// The command `text` sends, when it names one of the last list.
    pub(super) fn custom_call(&self, text: &str) -> Option<CommandCall> {
        let (name, rest) = named(text)?;
        (!built_in(name) && self.custom.list.iter().any(|command| command.name == name)).then(
            || CommandCall {
                name: name.to_owned(),
                args: rest.trim().to_owned(),
            },
        )
    }

    /// Whether the held prompt `text` waits for the session's first list,
    /// which is on its way: it may be one of its commands. It then stands
    /// where the held prompt on its way does, and the held prompts go,
    /// `first` or not, when it is answered.
    pub(super) fn held_waits_for_commands(&mut self, text: &str, first: bool) -> bool {
        let Some(asked) = self.custom.asked else {
            return false;
        };
        let answered = self
            .custom
            .answered
            .as_ref()
            .is_some_and(|session| self.is_open(session));
        if answered || named(text).is_none_or(|(name, _)| built_in(name)) {
            return false;
        }
        self.held_sent = Some(asked);
        self.custom.held = Some(first);
        true
    }

    /// An interrupt while the held prompts wait for the list: none was
    /// sent, so they come back, as those held before the session opened
    /// do. Whether they waited.
    pub(super) fn held_for_commands_back(&mut self) -> bool {
        if self.custom.held.take().is_none() {
            return false;
        }
        self.held_sent = None;
        self.give_back_held();
        true
    }
}
