//! The application's state, and what each event, key and result does to it.
//!
//! Plain data and plain functions of it: no terminal, no thread, no clock
//! of its own. The loop ([`super::run`]) gives it each event with the time
//! it was read, asks it for each frame, and writes what it gives; the
//! tests drive it the same way with a scripted [`Link`].

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use cortex_protocol::event::{Event, Notice, TurnOutcome};
use cortex_protocol::item::Delivery as ItemDelivery;
use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::message::Part;
use cortex_protocol::methods::{
    Disposition, ModelsListResult, Op, OpResult, ServerRequest, ServerResponse, SessionForkParams,
    SessionListParams, SessionNewParams, SessionRef, TurnInterruptParams, TurnStartParams,
};
use cortex_protocol::model::{ModelInfo, ModelRole};
use cortex_protocol::session::{SandboxStatus, SessionSettings, SessionState};
use cortex_protocol::usage::{CostBasis, SessionUsage, UsageTotals};
use cortex_protocol::{ErrorData, SessionEvent, SessionId, Speed};
use cortex_term::{Capabilities, Cursor, Event as TermEvent, Line};

mod commands;

use commands::{Picker, Shown};

use super::dialog::{Dialog, Keyed};
use super::items::Mapper;
use super::link::{Inbound, Link, Ticket};
use super::{Exit, Open, Settings, Start};
use crate::complete::Command;
use crate::editor::Fragment;
use crate::focus::Target;
use crate::input::{Delivery, Input, Outcome, SendMode};
use crate::transcript::{Block, Transcript};
use crate::view::{self, Header, Hint, HintRow, Look, RunningKeys};

/// The window in which a second interrupt is "stop now"
/// (`docs/architecture/02-runtime.md`, "Cancellation").
pub const SECOND_INTERRUPT: Duration = Duration::from_secs(2);

/// What the application asks the loop to do that it cannot do itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Draw the live region again from nothing (`Ctrl+L`).
    Redraw,
    /// Stop the program as a shell's job.
    Suspend,
    /// Put this text on the terminal's clipboard (`/copy`).
    Copy(String),
}

/// What an operation's result is for.
#[derive(Debug, Clone)]
enum Pending {
    /// The session this app opens.
    Open,
    /// The list of this workspace's sessions, for `--continue`: the newest
    /// is opened, or a new one when there is none.
    Continue {
        workspace: String,
        settings: SessionSettings,
        ephemeral: bool,
    },
    /// The session `--fork` forks, opened first: the fork follows.
    ForkFrom(SessionId),
    /// The list of models, for the hint row and the context's share.
    Models,
    /// A prompt: put back in the editor if it is refused.
    Prompt { text: String, started: bool },
    /// An interrupt: its steering prompts that were never delivered come
    /// back.
    Interrupt,
    /// Anything whose failure is said, and whose success needs nothing.
    Said,
    /// `/sessions`: the list, for its picker.
    Sessions,
    /// `/undo`.
    Undo,
    /// `/permissions`: the list, for its picker.
    Permissions,
    /// `/status` or `/config`.
    Config(Shown),
    /// A change of the session's settings, from a picker.
    Set,
}

/// The slash commands this build has, with what each does.
const COMMANDS: &[(&str, &str)] = &[
    (
        "compact",
        "Compact the conversation now, keeping what you name",
    ),
    ("config", "The configuration in effect"),
    ("copy", "Copy the last answer to the clipboard"),
    ("effort", "Choose the model's effort, and fast mode"),
    ("help", "The commands and the keys"),
    ("mode", "Choose how much is asked before it is done"),
    ("model", "Choose the model"),
    ("new", "Start a new session"),
    ("permissions", "The approvals stored here, to remove"),
    ("quit", "Quit"),
    ("sessions", "Switch to another session of this workspace"),
    ("status", "The session's model, mode, sandbox and limits"),
    ("theme", "Choose the colours, for this run"),
    ("undo", "Undo the last turn, its files put back"),
    ("view", "Open the session in a full-screen view"),
];

/// The slash commands of 09-tui that this build does not have yet: one is
/// said to be missing, not sent to the model as a prompt.
const NOT_YET: &[&str] = &[
    "rewind",
    "lease",
    "context",
    "usage",
    "plan",
    "project",
    "profile",
    "verify",
    "reload",
    "extend",
    "mcp",
    "extensions",
    "export",
];

/// The application. See the [module documentation](self).
pub struct App {
    link: Box<dyn Link>,
    settings: Settings,
    look: Look,
    input: Input,
    transcript: Transcript,
    mapper: Mapper,
    version: String,
    folder: String,
    /// The session to open after the first frame.
    open: Option<Open>,
    /// The workspace, for `/new`.
    workspace: String,
    /// The terminal's capabilities, for the look of a theme chosen later.
    caps: Capabilities,
    /// The picker the application opened, and what its rows stand for.
    picker: Option<(String, Picker)>,
    /// Every line committed to the scrollback so far, for the viewer.
    kept: Vec<Line>,
    /// The viewer, while it is open: how many rows up from the end it is
    /// scrolled.
    viewer: Option<usize>,
    /// The settings `/new` opens a session with, and whether it is kept in
    /// memory only: as the command line asked for the first.
    fresh: (SessionSettings, bool),
    /// The session `--fork` forked, still open on this client until the
    /// fork is: it is closed then.
    forked_from: Option<SessionId>,
    /// Whether the open is under way: its events are held until it returns.
    opening: bool,
    held: Vec<SessionEvent>,
    /// Prompts sent before the session was open.
    held_prompts: VecDeque<String>,
    pending: HashMap<Ticket, Pending>,
    session: Option<SessionState>,
    models: Vec<ModelInfo>,
    usage: Option<SessionUsage>,
    dialogs: VecDeque<Dialog>,
    /// When the running turn began, from this app's point of view.
    running: Option<Instant>,
    last_interrupt: Option<Instant>,
    /// When the last key was read: a dialog that appears while the person
    /// is typing waits before it takes keys.
    last_key: Option<Instant>,
    /// The last turn's cost and the session's, shown until the next prompt.
    last_cost: Option<(String, String)>,
    /// Whether this session has had a turn, here or before.
    had_turn: bool,
    effects: Vec<Effect>,
    exit: Option<Exit>,
    width: usize,
    rows: usize,
    began: Instant,
    tick: usize,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("session", &self.session.as_ref().map(|s| &s.session_id))
            .field("running", &self.running.is_some())
            .field("dialogs", &self.dialogs.len())
            .finish_non_exhaustive()
    }
}

impl App {
    /// The application for a terminal with `caps`, `cols` columns a line
    /// may use and `rows` rows, at `now`.
    #[must_use]
    pub fn new(
        link: Box<dyn Link>,
        start: Start,
        caps: &Capabilities,
        width: usize,
        rows: usize,
        now: Instant,
    ) -> Self {
        let highlighter = start.highlighter.clone();
        let look = super::look(&start.settings, caps);
        let mut input = Input::new(start.keymap, start.history);
        input.set_width(view::input::text_width(width));
        input.set_commands(
            COMMANDS
                .iter()
                .map(|(name, description)| Command {
                    name: (*name).to_owned(),
                    description: (*description).to_owned(),
                })
                .collect(),
        );
        let mut transcript = Transcript::new();
        for notice in start.notices {
            transcript.push(Block::Notice(notice));
        }
        let mut app = Self {
            link,
            settings: start.settings,
            look,
            input,
            transcript,
            mapper: Mapper::new(highlighter),
            version: start.version,
            folder: start.folder,
            fresh: match &start.open {
                Open::New {
                    settings,
                    ephemeral,
                    ..
                }
                | Open::Continue {
                    settings,
                    ephemeral,
                    ..
                } => (settings.clone(), *ephemeral),
                Open::Resume(_) | Open::Fork(_) => (SessionSettings::default(), false),
            },
            open: Some(start.open),
            workspace: start.workspace,
            caps: *caps,
            picker: None,
            kept: Vec::new(),
            viewer: None,
            forked_from: None,
            opening: false,
            held: Vec::new(),
            held_prompts: VecDeque::new(),
            pending: HashMap::new(),
            session: None,
            models: Vec::new(),
            usage: None,
            dialogs: VecDeque::new(),
            running: None,
            last_interrupt: None,
            last_key: None,
            last_cost: None,
            had_turn: false,
            effects: Vec::new(),
            exit: None,
            width,
            rows,
            began: now,
            tick: 0,
        };
        if let Some(prompt) = start.prompt.filter(|prompt| !prompt.trim().is_empty()) {
            app.held_prompts.push_back(prompt);
            app.running = Some(now);
            app.input.set_running(true);
        }
        app
    }

    // ---------------------------------------------------------- the loop

    /// The terminal's capabilities changed (a reply to a query): colours
    /// and glyphs follow.
    pub fn set_caps(&mut self, caps: &Capabilities) {
        self.caps = *caps;
        self.look = super::look(&self.settings, caps);
        for dialog in &mut self.dialogs {
            dialog.restyled();
        }
    }

    /// The window changed size.
    pub fn resize(&mut self, width: usize, rows: usize) {
        self.width = width;
        self.rows = rows;
        self.input.set_width(view::input::text_width(width));
    }

    /// A frame was written at `at`: the first opens the session (15-efficiency,
    /// "Start-up": after the first frame), and a dialog's first starts its
    /// guard.
    pub fn drawn(&mut self, at: Instant) {
        if let Some(open) = self.open.take() {
            self.send_open(open);
        }
        if let Some(dialog) = self.dialogs.front_mut() {
            dialog.drawn(at);
        }
    }

    /// The theme chosen for the run (`tui.theme`, or `/theme`).
    #[must_use]
    pub fn theme(&self) -> super::ThemeChoice {
        self.settings.theme
    }

    /// Whether the viewer is open: the loop then draws [`viewer_frame`]
    /// on the alternate screen, and nothing is committed meanwhile.
    ///
    /// [`viewer_frame`]: Self::viewer_frame
    #[must_use]
    pub fn viewing(&self) -> bool {
        self.viewer.is_some()
    }

    /// The viewer's screen, `rows` rows of `width` columns: the session as
    /// committed so far, then what is live, from where it is scrolled, and
    /// a row of its keys.
    pub fn viewer_frame(&mut self, rows: usize, width: usize, now: Instant) -> Vec<Line> {
        let since = now.saturating_duration_since(self.began).as_millis();
        self.tick = usize::try_from(since / super::SPIN.as_millis()).unwrap_or(0);
        let mut all = self.kept.clone();
        all.extend(self.transcript.live(width.max(1), &self.look, self.tick));
        let body = rows.saturating_sub(1).max(1);
        let most = all.len().saturating_sub(body);
        let offset = self.viewer.unwrap_or(0).min(most);
        self.viewer = Some(offset);
        let end = all.len() - offset;
        let start = end.saturating_sub(body);
        let mut lines: Vec<Line> = all[start..end].to_vec();
        while lines.len() < body {
            lines.push(Line::new());
        }
        lines.push(Line::styled(
            format!(
                "  up down pgup pgdn home end · q or esc closes · {} of {} lines",
                end,
                all.len()
            ),
            self.look.theme.style(crate::theme::Token::Muted),
        ));
        lines
    }

    /// A key while the viewer is open: it scrolls, or closes it. Global
    /// keys keep their meaning.
    fn viewer_key(&mut self, key: cortex_term::Key, at: Instant) -> bool {
        use cortex_term::KeyCode;
        if let Some(action @ (crate::keys::Action::Interrupt | crate::keys::Action::Redraw)) =
            self.input.keymap().lookup(&[], key)
        {
            let outcome = self.input.act(action);
            self.outcome(outcome, at);
            return true;
        }
        let page = self.rows.saturating_sub(2).max(1);
        let offset = self.viewer.unwrap_or(0);
        self.viewer = match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(offset + 1),
            KeyCode::Down | KeyCode::Char('j') => Some(offset.saturating_sub(1)),
            KeyCode::PageUp => Some(offset + page),
            KeyCode::PageDown => Some(offset.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => Some(usize::MAX),
            KeyCode::End | KeyCode::Char('G') => Some(0),
            KeyCode::Esc | KeyCode::Char('q') => None,
            KeyCode::Char('o') if key.mods.contains(cortex_term::Mods::CTRL) => None,
            _ => Some(offset),
        };
        true
    }

    /// When the application must draw again whatever else happens: the end
    /// of an open dialog's grace, when its answers appear, until a frame
    /// with them has been drawn. The time may have passed, and a frame is
    /// then due at once. `None` while nothing is due, so that an idle
    /// interface sleeps.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.dialogs.front().and_then(Dialog::deadline)
    }

    /// What to do before the next frame, taken.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Why the application ended, once it has.
    #[must_use]
    pub fn exit(&self) -> Option<&Exit> {
        self.exit.as_ref()
    }

    /// Whether something moves on the screen by itself: the spinner while
    /// a turn runs.
    #[must_use]
    pub fn animating(&self) -> bool {
        self.running.is_some() && !self.look.reduced_motion
    }

    /// Whether a turn runs.
    #[must_use]
    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// The session, once it is open.
    #[must_use]
    pub fn session(&self) -> Option<&SessionState> {
        self.session.as_ref()
    }

    /// The input: the editor, its menus and the focus.
    #[must_use]
    pub fn input(&self) -> &Input {
        &self.input
    }

    /// The last answer's text, for `/copy`.
    #[must_use]
    pub fn last_answer(&self) -> &str {
        self.mapper.last_answer()
    }

    // ----------------------------------------------------------- terminal

    /// Keys that arrived together with the next one, read at `at`, are
    /// typing, though a stall held them back: the next key is judged as
    /// following one read at `at`, so that it does not answer a dialog.
    pub fn burst(&mut self, at: Instant) {
        self.last_key = Some(at);
    }

    /// A terminal event, read at `at`. Says whether it was input (a key or a
    /// paste), whose frame is drawn at once.
    pub fn terminal(&mut self, event: &TermEvent, at: Instant) -> bool {
        if self.viewer.is_some()
            && let TermEvent::Key(key) = event
        {
            return self.viewer_key(*key, at);
        }
        match event {
            TermEvent::Key(key) => {
                // The key before this one: a dialog takes a key only after a
                // pause, so that it never takes typing meant for the editor.
                let previous = self.last_key.replace(at);
                if let Some(dialog) = self.dialogs.front_mut() {
                    match dialog.key(*key, at, previous, self.input.keymap()) {
                        Keyed::Pass => {}
                        Keyed::Global(action) => {
                            let outcome = self.input.act(action);
                            self.outcome(outcome, at);
                            return true;
                        }
                        Keyed::Answer(response) => {
                            self.answer(response);
                            return true;
                        }
                        Keyed::Changed | Keyed::Nothing => return true,
                    }
                }
                let outcome = self.input.handle(event);
                self.outcome(outcome, at);
                true
            }
            // A paste never answers a dialog: it is a note being written, or
            // it goes to the editor.
            TermEvent::Paste(text)
                if self
                    .dialogs
                    .front_mut()
                    .is_some_and(|dialog| dialog.paste(text)) =>
            {
                true
            }
            TermEvent::Paste(_) => {
                let outcome = self.input.handle(event);
                self.outcome(outcome, at);
                true
            }
            TermEvent::Resize { .. } | TermEvent::Resumed | TermEvent::Reply(_) => false,
            TermEvent::Focus(_) => false,
        }
    }

    fn outcome(&mut self, outcome: Outcome, at: Instant) {
        match outcome {
            Outcome::Submit { text, delivery } => self.submit(text, delivery, at),
            Outcome::Interrupt => self.interrupt(at),
            Outcome::Quit => self.exit = Some(Exit::Quit),
            Outcome::Redraw => self.effects.push(Effect::Redraw),
            Outcome::Suspend => self.effects.push(Effect::Suspend),
            Outcome::ToggleRound => {
                self.mapper
                    .toggle_round(&mut self.transcript, None, &self.look);
            }
            Outcome::Activate(Target::ToolBox(id)) => {
                self.mapper
                    .toggle_round(&mut self.transcript, Some(id), &self.look);
            }
            Outcome::Picked(name, index) => self.picked(&name, index),
            Outcome::Toggled(name, on) => self.toggled(&name, on),
            Outcome::Removed(name, index) => self.removed(&name, index),
            Outcome::PickerClosed(_) => self.picker = None,
            Outcome::OpenViewer => self.viewer = Some(0),
            _ => {}
        }
    }

    /// A prompt was sent from the editor.
    fn submit(&mut self, text: String, delivery: Delivery, at: Instant) {
        if let Some(command) = text.strip_prefix('/') {
            let (name, rest) = command
                .split_once(char::is_whitespace)
                .unwrap_or((command, ""));
            if COMMANDS.iter().any(|(known, _)| *known == name) {
                self.command(name, rest.trim(), at);
                return;
            }
            if NOT_YET.contains(&name) {
                self.say(&format!(
                    "/{name} is not in this build yet. /help lists the commands there are."
                ));
                self.give_back(&text);
                return;
            }
        }
        self.last_cost = None;
        let Some(session) = self.session.as_ref().map(|s| s.session_id.clone()) else {
            if self.opening || self.open.is_some() {
                // The session is being opened: the prompt goes when it is.
                self.held_prompts.push_back(text);
                self.start_running(at);
            } else {
                self.say("No session is open: /new starts one.");
                self.give_back(&text);
            }
            return;
        };
        let started = delivery == Delivery::Prompt;
        if started {
            self.start_running(at);
        }
        let delivery = match delivery {
            Delivery::Prompt => None,
            Delivery::Steer => Some(ItemDelivery::Steer),
            Delivery::Queue => Some(ItemDelivery::Queue),
        };
        let ticket = self.link.send(Op::TurnStart(TurnStartParams {
            session_id: session,
            input: vec![Part::text(text.clone())],
            delivery,
        }));
        self.pending
            .insert(ticket, Pending::Prompt { text, started });
    }

    fn start_running(&mut self, at: Instant) {
        if self.running.is_none() {
            self.running = Some(at);
        }
        self.input.set_running(true);
    }

    fn stop_running(&mut self) {
        self.running = None;
        self.input.set_running(false);
    }

    /// The interrupt key while a turn runs; a second within
    /// [`SECOND_INTERRUPT`] stops running commands at once.
    fn interrupt(&mut self, at: Instant) {
        let now = self
            .last_interrupt
            .is_some_and(|last| at.saturating_duration_since(last) <= SECOND_INTERRUPT);
        self.last_interrupt = Some(at);
        let Some(session) = self.session.as_ref().map(|s| s.session_id.clone()) else {
            // Nothing was sent yet: the held prompts are not sent, and come
            // back into the editor.
            self.give_back_held();
            self.stop_running();
            return;
        };
        let ticket = self.link.send(Op::TurnInterrupt(TurnInterruptParams {
            session_id: session,
            now,
        }));
        self.pending.insert(ticket, Pending::Interrupt);
    }

    fn answer(&mut self, response: ServerResponse) {
        let Some(dialog) = self.dialogs.pop_front() else {
            return;
        };
        if let Some(call) = dialog.call() {
            self.mapper.answered(&mut self.transcript, call);
        }
        self.link.respond(dialog.id, response);
    }

    // ------------------------------------------------------------- core

    fn send_open(&mut self, open: Open) {
        self.opening = true;
        let (op, pending) = match open {
            Open::New {
                workspace,
                settings,
                ephemeral,
            } => (new_session(workspace, settings, ephemeral), Pending::Open),
            Open::Resume(id) => (
                Op::SessionOpen(SessionRef { session_id: id }),
                Pending::Open,
            ),
            // A session is forked from its log, so it is opened first; the
            // fork follows its result.
            Open::Fork(id) => (
                Op::SessionOpen(SessionRef {
                    session_id: id.clone(),
                }),
                Pending::ForkFrom(id),
            ),
            Open::Continue {
                workspace,
                settings,
                ephemeral,
            } => (
                // The newest session here first; the open follows its result.
                Op::SessionList(SessionListParams {
                    workspace: Some(workspace.clone()),
                    limit: Some(1),
                    cursor: None,
                }),
                Pending::Continue {
                    workspace,
                    settings,
                    ephemeral,
                },
            ),
        };
        let ticket = self.link.send(op);
        self.pending.insert(ticket, pending);
        let ticket = self
            .link
            .send(Op::ModelsList(cortex_protocol::methods::Empty {}));
        self.pending.insert(ticket, Pending::Models);
    }

    /// Takes what has arrived, at most `most` events of it. Says whether
    /// anything changed, and whether more is waiting.
    pub fn drain(&mut self, now: Instant, most: usize) -> Drained {
        let mut drained = Drained::default();
        let mut taken = 0;
        while taken < most {
            let Some(inbound) = self.link.next() else {
                return drained;
            };
            taken += 1;
            drained.changed = true;
            match inbound {
                Inbound::Event(event) => {
                    if self.opening {
                        self.held.push(event);
                    } else {
                        self.event(event, now);
                    }
                }
                Inbound::Request(id, request) => {
                    drained.request = true;
                    self.request(id, request);
                }
                Inbound::Done(ticket, result) => self.done(ticket, result, now),
                Inbound::Gone(error) => {
                    self.exit = Some(Exit::Gone(error));
                    return drained;
                }
            }
        }
        drained.more = true;
        drained
    }

    fn request(&mut self, id: RequestId, request: ServerRequest) {
        let dialog = match request {
            ServerRequest::Approval(approval) => {
                self.mapper.waiting(&mut self.transcript, &approval.call_id);
                let confined = self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.sandbox == SandboxStatus::Enforcing);
                Dialog::approval(id, approval, confined)
            }
            // Asked while the session is being opened, before anything of
            // it is shown.
            ServerRequest::Trust(trust) => Dialog::trust(id, trust),
            ServerRequest::Question(question) => Dialog::question(id, question),
        };
        self.dialogs.push_back(dialog);
    }

    fn done(&mut self, ticket: Ticket, result: Result<OpResult, ErrorData>, now: Instant) {
        let Some(pending) = self.pending.remove(&ticket) else {
            return;
        };
        match (pending, result) {
            (
                Pending::Open,
                Ok(
                    OpResult::SessionNew(result)
                    | OpResult::SessionOpen(result)
                    | OpResult::SessionFork(result),
                ),
            ) => {
                self.opened(result.session, now);
            }
            (Pending::Open, Err(error)) => self.open_failed(Some(&error), now),
            (Pending::ForkFrom(original), Ok(OpResult::SessionOpen(_))) => {
                let ticket = self.link.send(Op::SessionFork(SessionForkParams {
                    session_id: original.clone(),
                    at: None,
                }));
                self.pending.insert(ticket, Pending::Open);
                self.forked_from = Some(original);
            }
            (Pending::ForkFrom(_), result) => self.open_failed(result.err().as_ref(), now),
            (Pending::Continue { .. }, Err(error)) => self.open_failed(Some(&error), now),
            (
                Pending::Continue {
                    workspace,
                    settings,
                    ephemeral,
                },
                result,
            ) => {
                let latest = match result {
                    Ok(OpResult::SessionList(list)) => list.sessions.into_iter().next(),
                    _ => None,
                };
                let op = match latest {
                    Some(latest) => Op::SessionOpen(SessionRef {
                        session_id: latest.session_id,
                    }),
                    None => {
                        self.say("There is no session to continue here, so this is a new one.");
                        new_session(workspace, settings, ephemeral)
                    }
                };
                let ticket = self.link.send(op);
                self.pending.insert(ticket, Pending::Open);
            }
            (Pending::Models, Ok(OpResult::ModelsList(ModelsListResult { models }))) => {
                self.models = models;
            }
            (Pending::Prompt { text, started }, Err(error)) => {
                if started && self.running.is_some() {
                    self.stop_running();
                }
                self.error(&error);
                self.give_back(&text);
            }
            (Pending::Prompt { .. }, Ok(OpResult::TurnStart(result))) => {
                if result.disposition == Disposition::Started {
                    self.had_turn = true;
                }
            }
            (Pending::Interrupt, Ok(OpResult::TurnInterrupt(result))) => {
                for parts in result.undelivered {
                    let text: Vec<&str> = parts
                        .iter()
                        .filter_map(|part| match part {
                            Part::Text(text) => Some(text.text.as_str()),
                            _ => None,
                        })
                        .collect();
                    self.give_back(&text.join("\n"));
                }
            }
            (Pending::Sessions, Ok(OpResult::SessionList(list))) => {
                self.sessions_listed(list.sessions);
            }
            (Pending::Undo, Ok(OpResult::SessionUndo(result))) => {
                self.session = Some(result.session);
                self.say(
                    "The last turn is undone: it is out of the conversation, and the files it \
                     changed are as they were before it.",
                );
            }
            (Pending::Permissions, Ok(OpResult::PermissionsList(list))) => {
                self.permissions_listed(list.approvals);
            }
            (Pending::Config(shown), Ok(OpResult::ConfigGet(result))) => {
                self.config_shown(shown, &result.config);
            }
            (Pending::Set, Ok(OpResult::SessionSet(result))) => {
                self.session = Some(result.session);
            }
            (_, Err(error)) => self.error(&error),
            _ => {}
        }
    }

    /// The session could not be opened: what was held while it was being
    /// opened is shown or given back, and why is said. A fork's original,
    /// opened for it, is closed.
    fn open_failed(&mut self, error: Option<&ErrorData>, now: Instant) {
        self.opening = false;
        if let Some(original) = self.forked_from.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: original,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        // What was held is of a session that is not this client's: a
        // fork's original, the one `/new` closed, or one that failed to open.
        // Only its notices are shown.
        let held = std::mem::take(&mut self.held);
        for event in held {
            if matches!(event.event, Event::Notice(_)) {
                self.event(event, now);
            }
        }
        self.give_back_held();
        self.stop_running();
        if let Some(error) = error {
            self.error(error);
        }
    }

    /// The prompts held for a session that will not open come back into
    /// the editor.
    fn give_back_held(&mut self) {
        let held: Vec<String> = self.held_prompts.drain(..).collect();
        for text in held {
            self.give_back(&text);
        }
    }

    /// `/new`: the session open now is closed, and a new one opened in the
    /// workspace, with its own header.
    fn new_session(&mut self) {
        if self.running.is_some() {
            self.say("A turn is running: wait for it to end, or stop it with esc.");
            return;
        }
        if self.opening {
            self.say("A session is being opened: /new can start another once it is.");
            return;
        }
        if let Some(session) = self.session.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: session.session_id,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        self.had_turn = false;
        self.last_cost = None;
        self.usage = None;
        // As the command line asked for the first: its settings, and kept
        // in memory only if it was.
        let (settings, ephemeral) = self.fresh.clone();
        self.send_open(Open::New {
            workspace: self.workspace.clone(),
            settings,
            ephemeral,
        });
    }

    /// A prompt that was not delivered goes back into the editor, after
    /// whatever is there.
    fn give_back(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut now = self.input.editor().expanded();
        if !now.is_empty() {
            now.push('\n');
        }
        now.push_str(text);
        self.input.external_result(Some(Fragment::text(now)));
    }

    fn opened(&mut self, state: SessionState, now: Instant) {
        self.opening = false;
        if let Some(original) = self.forked_from.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: original,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        let main = state.roles.get(&ModelRole::Main);
        self.transcript.push(Block::Header(Header {
            version: self.version.clone(),
            model: main
                .map(|role| role.model.as_str().to_owned())
                .unwrap_or_default(),
            effort: main.and_then(|role| role.effort_in_effect.clone().or(role.effort.clone())),
            folder: self.folder.clone(),
            branch: None,
            profile: None,
        }));
        self.session = Some(state);
        let held = std::mem::take(&mut self.held);
        for event in held {
            self.event(event, now);
        }
        let prompts: Vec<String> = self.held_prompts.drain(..).collect();
        let mut first = true;
        for text in prompts {
            let delivery = if first {
                Delivery::Prompt
            } else {
                Delivery::Queue
            };
            first = false;
            self.submit_open(text, delivery);
        }
    }

    /// Sends a prompt held while the session opened.
    fn submit_open(&mut self, text: String, delivery: Delivery) {
        let Some(session) = self.session.as_ref().map(|s| s.session_id.clone()) else {
            return;
        };
        let ticket = self.link.send(Op::TurnStart(TurnStartParams {
            session_id: session,
            input: vec![Part::text(text.clone())],
            delivery: match delivery {
                Delivery::Prompt => None,
                Delivery::Steer => Some(ItemDelivery::Steer),
                Delivery::Queue => Some(ItemDelivery::Queue),
            },
        }));
        self.pending.insert(
            ticket,
            Pending::Prompt {
                text,
                started: delivery == Delivery::Prompt,
            },
        );
    }

    fn event(&mut self, event: SessionEvent, now: Instant) {
        if let Some(session) = &self.session
            && session.session_id != event.session_id
        {
            return;
        }
        match &event.event {
            Event::TurnStarted(_) => {
                self.had_turn = true;
                self.start_running(now);
            }
            Event::TurnCompleted(completed) => {
                self.mapper.turn_done(&mut self.transcript, now);
                if let Some(error) = &completed.error {
                    self.error(error);
                } else if completed.outcome == TurnOutcome::Interrupted {
                    self.say("Interrupted.");
                }
                let took = completed
                    .duration_ms
                    .map(Duration::from_millis)
                    .or_else(|| {
                        self.running
                            .map(|began| now.saturating_duration_since(began))
                    })
                    .unwrap_or_default();
                if self.look.turn_summary {
                    self.transcript.push(Block::Summary {
                        took,
                        tokens: Some(tokens(&completed.usage)),
                        cost: cost(&completed.usage),
                    });
                }
                let session = self.usage.as_ref().and_then(|usage| cost(&usage.totals));
                if let Some(turn) = cost(&completed.usage) {
                    self.last_cost = Some((turn, session.unwrap_or_default()));
                }
                self.dialogs.clear();
                self.stop_running();
                self.last_interrupt = None;
            }
            // The session open on this client changed. An update of another,
            // while none is open, is not taken for it.
            Event::SessionUpdated(updated) => {
                if self.session.is_some() {
                    self.session = Some(updated.session.clone());
                }
            }
            Event::UsageUpdated(updated) => self.usage = Some(updated.usage.clone()),
            Event::Notice(notice) => {
                if let Some(message) = notice_text(notice) {
                    self.transcript.push(Block::Notice(message));
                }
            }
            Event::RequestResolved(resolved) => {
                if let Some(at) = self
                    .dialogs
                    .iter()
                    .position(|dialog| dialog.id == resolved.request_id)
                    && let Some(dialog) = self.dialogs.remove(at)
                    && let Some(call) = dialog.call()
                {
                    self.mapper.answered(&mut self.transcript, call);
                }
            }
            _ => self.mapper.event(&mut self.transcript, &event.event, now),
        }
    }

    fn say(&mut self, text: &str) {
        self.transcript.push(Block::Notice(text.to_owned()));
    }

    fn error(&mut self, error: &ErrorData) {
        let (message, hint) = super::wording::error(error);
        self.transcript.push(Block::Error { message, hint });
    }

    // ------------------------------------------------------------- frame

    /// The frame for `now`: the lines that became final, the live region,
    /// and where the cursor goes in it.
    pub fn frame(&mut self, now: Instant) -> (Vec<Line>, Vec<Line>, Cursor) {
        let width = self.width.max(1);
        // The spinner's frame follows the clock, one every `SPIN`.
        let since = now.saturating_duration_since(self.began).as_millis();
        self.tick = usize::try_from(since / super::SPIN.as_millis()).unwrap_or(0);
        self.mapper.tick(&mut self.transcript, now);
        self.transcript
            .set_now(now.saturating_duration_since(self.began));
        let boxes = self.mapper.live_rounds(&mut self.transcript);
        self.input.focus_mut().update(&boxes, &[]);
        let commit = self.transcript.commit(width, &self.look);
        self.kept.extend(commit.iter().cloned());
        let mut live = self.transcript.live(width, &self.look, self.tick);

        // The bottom: a dialog in the input box's place, or the input box
        // with the menu or the hint row under it.
        let mut bottom = Vec::new();
        let cursor_row;
        let mut cursor_col = 0;
        let half = (self.rows / 2).max(1);
        if !self.dialogs.is_empty() {
            // The dialog fits the window whole, with a blank row and the hint
            // row under it while there is room for them: its detail is what
            // gives way, and says so.
            let chrome = self.rows >= 6;
            let rows = if chrome {
                self.rows - 2
            } else {
                self.rows.max(1)
            };
            let keymap = self.input.keymap();
            let (lines, keys) = match self.dialogs.front_mut() {
                Some(dialog) => (
                    dialog.view(keymap, rows, width, &self.look, now),
                    dialog.hint_keys(keymap),
                ),
                None => (Vec::new(), Vec::new()),
            };
            bottom.extend(lines);
            if chrome {
                bottom.push(Line::new());
                bottom.push(self.hint_row(Hint::Keys(keys), now));
            }
            cursor_row = None;
        } else {
            let drawn = view::input::input(&self.input, &self.look, width);
            let top = bottom.len();
            bottom.extend(drawn.lines);
            cursor_row = Some(top + drawn.cursor.row);
            cursor_col = drawn.cursor.col;
            match view::menu::menu(&self.input, &self.look, width) {
                Some(menu) => bottom.extend(menu),
                None => {
                    let left = self.hint_left(now);
                    bottom.push(self.hint_row(left, now));
                }
            }
        }

        // The live region is at most half the window, and at least the
        // bottom: what streams is cut from its top, with a mark.
        let room = half.max(bottom.len()).saturating_sub(bottom.len());
        if live.len() > room {
            let cut = live.len() - room + 1;
            live.drain(..cut.min(live.len()));
            if room > 0 {
                live.insert(
                    0,
                    Line::styled(
                        format!("  {}", self.look.glyphs.ellipsis),
                        self.look.theme.style(crate::theme::Token::Muted),
                    ),
                );
            }
        }
        let offset = live.len();
        live.extend(bottom);
        let cursor = match cursor_row {
            Some(row) => Cursor {
                row: offset + row,
                col: cursor_col,
            },
            // A dialog has no text cursor: it rests at the region's end.
            None => Cursor {
                row: live.len().saturating_sub(1),
                col: 0,
            },
        };
        (commit, live, cursor)
    }

    fn hint_left(&self, now: Instant) -> Hint {
        if self.input.quit_armed() {
            return Hint::QuitArmed;
        }
        if let Some(began) = self.running {
            return Hint::Running {
                elapsed: now.saturating_duration_since(began),
                tick: self.tick,
                keys: match self.input.send_mode() {
                    SendMode::Steer => RunningKeys::Steer,
                    SendMode::Queue => RunningKeys::Queue,
                },
            };
        }
        if let Some((turn, session)) = &self.last_cost {
            return Hint::Cost {
                turn: turn.clone(),
                session: session.clone(),
            };
        }
        if self.had_turn {
            Hint::Empty
        } else {
            Hint::FirstSessions
        }
    }

    fn hint_row(&self, left: Hint, now: Instant) -> Line {
        let left = match (left, self.running) {
            // A dialog's keys while a turn runs follow the spinner.
            (Hint::Keys(keys), Some(began)) => Hint::Running {
                elapsed: now.saturating_duration_since(began),
                tick: self.tick,
                keys: RunningKeys::Of(keys),
            },
            (left, _) => left,
        };
        let main = self
            .session
            .as_ref()
            .and_then(|session| session.roles.get(&ModelRole::Main));
        let speed = main.and_then(|role| {
            (role.speed != Speed::Standard).then(|| view::Speed {
                name: role.speed.as_str().to_owned(),
                cooldown_until: role
                    .speed_cooldown_until
                    .as_ref()
                    .filter(|_| role.speed_in_effect != role.speed)
                    .map(|until| clock_time(until.as_str())),
            })
        });
        let window = main.and_then(|role| {
            self.models
                .iter()
                .find(|model| model.id == role.model && model.provider == role.provider)
                .map(|model| model.context_window)
        });
        let context = match (self.usage.as_ref().and_then(|u| u.context_tokens), window) {
            (Some(tokens), Some(window)) if window > 0 => {
                let share = tokens.saturating_mul(100) / window;
                (share >= 50).then(|| u8::try_from(share.min(100)).unwrap_or(100))
            }
            _ => None,
        };
        let access = self
            .session
            .as_ref()
            .and_then(|session| match session.level.as_str() {
                "unrestricted" => Some("unrestricted".to_owned()),
                "autonomous" if session.sandbox.as_str() == "off" => Some("sandbox off".to_owned()),
                _ => None,
            });
        let mut extras = Vec::new();
        if let Some(session) = &self.session
            && session.queue_length > 0
        {
            extras.push(format!("{} queued", session.queue_length));
        }
        view::hint_row(
            &HintRow {
                left,
                model: main
                    .map(|role| short_model(role.model.as_str()))
                    .unwrap_or_default(),
                effort: main.and_then(|role| role.effort_in_effect.clone().or(role.effort.clone())),
                speed,
                context,
                budget: None,
                limit: None,
                access,
                extras,
            },
            self.width,
            &self.look,
        )
    }
}

/// What [`App::drain`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drained {
    /// Something arrived.
    pub changed: bool,
    /// More is waiting.
    pub more: bool,
    /// A request arrived: a dialog to draw at once.
    pub request: bool,
}

fn new_session(workspace: String, settings: SessionSettings, ephemeral: bool) -> Op {
    Op::SessionNew(SessionNewParams {
        workspace,
        settings,
        title: None,
        config: serde_json::Map::new(),
        ephemeral,
    })
}

/// A notice in words: the message, with a compaction's counts when it does
/// not say them.
fn notice_text(notice: &Notice) -> Option<String> {
    let message = notice.message()?.to_owned();
    if let Notice::Compaction(compaction) = notice
        && let (Some(before), Some(after)) = (compaction.before_tokens, compaction.after_tokens)
        && !message.contains(&view::blocks::count(before))
    {
        return Some(format!(
            "{message} ({} to {} tokens)",
            view::blocks::count(before),
            view::blocks::count(after)
        ));
    }
    Some(message)
}

/// Every token a turn used.
fn tokens(usage: &UsageTotals) -> u64 {
    let tokens = &usage.tokens;
    tokens.input + tokens.output + tokens.cache_read + tokens.cache_write
}

/// A cost as the hint row shows it: `$0.42`, `~$0.42` for an estimate.
fn cost(usage: &UsageTotals) -> Option<String> {
    let cost = usage.costs.first()?;
    let dollars = cost.usd_nanos as f64 / 1e9;
    let mark = if cost.basis == CostBasis::Estimate {
        "~"
    } else {
        ""
    };
    Some(if dollars < 0.01 {
        format!("{mark}${dollars:.4}")
    } else {
        format!("{mark}${dollars:.2}")
    })
}

/// A model's short name: its id without the provider's family prefix
/// (`claude-opus-5-5` is `opus-5.5`).
fn short_model(id: &str) -> String {
    let id = id.strip_prefix("claude-").unwrap_or(id);
    // The last dash between two digits is the version's point.
    let bytes = id.as_bytes();
    let mut out = id.to_owned();
    for at in (1..bytes.len().saturating_sub(1)).rev() {
        if bytes[at] == b'-' && bytes[at - 1].is_ascii_digit() && bytes[at + 1].is_ascii_digit() {
            out.replace_range(at..=at, ".");
            break;
        }
    }
    out
}

/// A time of day from a timestamp, `14:32`, as the hint row shows a
/// cooldown's end. The timestamp is the protocol's RFC 3339 text in UTC.
fn clock_time(timestamp: &str) -> String {
    timestamp
        .split_once('T')
        .map(|(_, time)| time.chars().take(5).collect())
        .unwrap_or_else(|| timestamp.to_owned())
}
