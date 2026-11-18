//! The application's state, and what each event, key and result does to it.
//!
//! Plain data and plain functions of it: no terminal, no thread, no clock
//! of its own. The loop ([`super::run`]) gives it each event with the time
//! it was read, asks it for each frame, and writes what it gives; the
//! tests drive it the same way with a scripted [`Link`].
//!
//! **Prompts at a turn's edges** (step 27). The application chooses a
//! prompt's delivery from its own view of whether a turn runs, which lags
//! the core's; both links drive it, so what follows is decided here, once
//! (`docs/architecture/09-tui.md`, "Start-up" and "Running from the moment
//! a prompt is sent"):
//!
//! - **The prompts held while the session opened go one at a time,** each
//!   once the one before it was answered: the first as a new turn, or
//!   steering the turn the open says is running (`session/open`'s
//!   `running_turn`), as `Enter` would; the rest queued behind it. A prompt
//!   sent while they are on their way follows them, and the session is not
//!   left until the last is answered. A refused one comes back into the
//!   editor with those behind it, in order, and the spinner they started
//!   stops unless the core has said a turn runs. An interrupt sends those
//!   not sent yet at once, queued behind the one on its way and ahead of
//!   the interrupt, so that the core holds them in order, as it holds any
//!   queued prompt at an interrupt; the one on its way, refused or handed
//!   back undelivered, comes into the editor, and sent again it runs
//!   first, with them behind it. None overtakes another.
//! - **A prompt refused `turn_running` is sent again once, queued:** sent
//!   with no delivery in the moment between one turn's `turn.completed` and
//!   the `turn.started` of the next the core takes from its queue, or onto a
//!   turn another client began, it runs after the turn that took the gap.
//!   Nothing is said. A second refusal, or one after an interrupt, gives it
//!   back with why, as any refusal does.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use cineris_protocol::error::code;
use cineris_protocol::event::{Event, Notice, TurnOutcome};
use cineris_protocol::item::Delivery as ItemDelivery;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::message::Part;
use cineris_protocol::methods::{
    Disposition, ModelsListResult, Op, OpResult, ServerRequest, ServerResponse, SessionForkParams,
    SessionListParams, SessionNewParams, SessionRef, TurnInterruptParams, TurnStartParams,
};
use cineris_protocol::model::{ModelInfo, ModelRole};
use cineris_protocol::session::{SandboxStatus, SessionSettings, SessionState};
use cineris_protocol::usage::{CostBasis, SessionUsage, UsageTotals};
use cineris_protocol::{ErrorData, SessionEvent, SessionId, Speed};
use cineris_term::{Capabilities, Cursor, Event as TermEvent, Key, Line};

mod commands;
mod export;
mod panel;
mod recolour;
mod restyle;
mod rewind;
pub(super) mod screen;
mod viewer;

pub use restyle::{Restyle, Restyled};

use commands::{Picker, Shown};

use super::dialog::{Asks, Dialog, DialogId, Keyed, QUIET, REPEAT};
use super::items::Mapper;
use super::link::{Inbound, Link, Ticket};
use super::panels::{ItemEditFor, Panels, Seen};
use super::{Exit, Open, Settings, Start};
use crate::complete::Command;
use crate::editor::Fragment;
use crate::focus::Target;
use crate::input::{Delivery, Input, Outcome, SendMode};
use crate::transcript::kept::Kept;
use crate::transcript::{Block, Committed, Transcript};
use crate::view::{self, Hint, HintRow, Look, RunningKeys};

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
    /// A prompt: put back in the editor if it is refused, with the held
    /// prompts that wait for it.
    Prompt(Sent),
    /// An interrupt: its steering prompts that were never delivered come
    /// back.
    Interrupt,
    /// Anything whose failure is said, and whose success needs nothing.
    Said,
    /// `/sessions`: the list, for its picker.
    Sessions,
    /// `/undo` of this session: its result is the session's only while it
    /// is still the one open.
    Undo(SessionId),
    /// `/permissions`: the list, for its picker.
    Permissions,
    /// `/status` or `/config` of this session.
    Config(Shown, SessionId),
    /// A change of this session's settings, from a picker.
    Set(SessionId),
    /// A page of this stored session's history, to draw while it is the
    /// one being read.
    History(SessionId),
    /// The workspace's files of this session, for `@` completion.
    Files(SessionId),
    /// What a panel sent or asked for in this session (since step 24): its
    /// result is the panel's while the session is the one open, and its
    /// error is said.
    Panel { session: SessionId, panel: u64 },
    /// What a panel asked as the session opened.
    Opened { session: SessionId, panel: u64 },
    /// A file named with `@`, read ahead: nothing is said of its answer, a
    /// refusal included, since nothing the person asked for depends on it.
    Warm,
    /// `/reload`, or `/profile` with a name: what it did is said from its
    /// result, which holds what its `config.changed` events say.
    Reload,
    /// `/profile` with no name: the profiles this session's files define.
    Profiles(SessionId),
    /// A stored artifact of this session (a check's whole output), shown in
    /// the viewer under its label while the session is the one open.
    Artifact { session: SessionId, label: String },
    /// A look at the configuration's files, after a key or a focus-in:
    /// what it found is said by `config.changed`, and nothing of its own
    /// answer is.
    Look,
    /// The `tui` and `keys` sections, read again after a reload applied a
    /// key of either: `keys` when a binding changed, `theme` when the
    /// file's theme did.
    Restyle { keys: bool, theme: bool },
    /// A page of this session, read for `/export` (since step 25).
    Export(SessionId),
    /// A compaction's summary, read for the viewer (since step 25).
    Summary(SessionId),
    /// A page of this session's prompts, for the `/rewind` picker (since
    /// step 25).
    Points(SessionId),
    /// The prompt of the point chosen, read whole.
    RewindPrompt(SessionId),
    /// The plan of a rewind of this session.
    RewindPlan(SessionId),
    /// A rewind of this session, answered.
    Rewind(SessionId),
    /// A page of this session's active branch, read again for the viewer
    /// and the full screen after a rewind or an undo: which read it is.
    Reread(SessionId, u64),
    /// A fork of the session open here, to be opened in its place.
    Fork(Box<rewind::Forking>),
}

/// A prompt on its way to the core, for its answer.
#[derive(Debug, Clone)]
struct Sent {
    /// The session it was sent to.
    session: SessionId,
    /// Its text: given back if it is refused.
    text: String,
    /// It was sent as a new turn, and this app runs from then: a refusal
    /// stops that.
    started: bool,
    /// An interrupt was sent after it: a refusal gives it back, and is not
    /// sent again.
    stopped: bool,
    /// It was refused `turn_running` once and sent again queued: a refusal
    /// now gives it back.
    again: bool,
}

/// The last key read, for the dialogs' rules (`super::dialog`).
#[derive(Debug, Clone, Copy)]
struct LastKey {
    /// It was typing (it went to the editor or the viewer), not a key a
    /// dialog took: a repeat of it, and a dialog's key after it, are typing
    /// too.
    typing: bool,
}

/// What a reload did, as lines to say: the keys in force now, the core's
/// notes, then each error.
fn reload_lines(applied: &[String], errors: &[String], notes: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    if !applied.is_empty() {
        let keys: Vec<String> = applied.iter().map(|key| format!("`{key}`")).collect();
        lines.push(format!(
            "The configuration was read again: {} in force now.",
            keys.join(", ")
        ));
    }
    lines.extend(notes.iter().cloned());
    lines.extend(errors.iter().cloned());
    lines
}

/// The keys row while the transcript is read from under a dialog.
/// The way back to the dialog first, so that a narrow row keeps it.
fn reading_keys(keymap: &crate::keys::KeyMap) -> Vec<String> {
    let key = |action| {
        keymap
            .keys_of(action)
            .first()
            .map(|key| crate::keys::name(*key))
    };
    let mut keys = vec![reading_back(keymap), "reading the transcript".to_owned()];
    if let (Some(up), Some(down)) = (
        key(crate::keys::Action::ScreenPageUp),
        key(crate::keys::Action::ScreenPageDown),
    ) {
        keys.push(format!("{up}/{down} scroll"));
    }
    keys
}

/// How to go back to the dialog from reading.
fn reading_back(keymap: &crate::keys::KeyMap) -> String {
    let back = keymap
        .keys_of(crate::keys::Action::DialogTranscript)
        .first()
        .map_or_else(|| "esc".to_owned(), |key| crate::keys::name(*key));
    format!("{back} goes back")
}

/// The most files read ahead for one prompt: the core reads no more of one
/// `workspace/warm`.
const WARM_FILES: usize = 16;

/// The most items of a stored session's history read at a time.
const HISTORY_PAGE: u32 = 500;

/// The slash commands this build has, with what each does.
const COMMANDS: &[(&str, &str)] = &[
    (
        "compact",
        "Compact the conversation now, keeping what you name",
    ),
    (
        "config",
        "The configuration in effect, and where each value came from",
    ),
    (
        "context",
        "What the last request held, by part, against the window",
    ),
    ("copy", "Copy the last answer to the clipboard"),
    ("effort", "Choose the model's effort, and fast mode"),
    (
        "export",
        "Write the session to a file: Markdown, or JSON with json",
    ),
    ("help", "The commands and the keys"),
    ("mode", "Choose how much is asked before it is done"),
    ("model", "Choose the model"),
    ("new", "Start a new session"),
    ("permissions", "The approvals stored here, to remove"),
    ("plan", "The plan: skip, reword, add or move its items"),
    (
        "project",
        "What is kept of this project: accept, edit, turn off or promote",
    ),
    (
        "profile",
        "Load this session with a settings profile, or list them",
    ),
    ("quit", "Quit"),
    ("reload", "Read the configuration files again"),
    (
        "rewind",
        "Go back to before an earlier prompt, or fork from it",
    ),
    ("sessions", "Switch to another session of this workspace"),
    ("status", "The session's model, mode, sandbox and limits"),
    ("theme", "Choose the colours, for this run"),
    ("undo", "Undo the last turn, its files put back"),
    (
        "usage",
        "What this session and the days before used and cost, and the limits",
    ),
    ("verify", "Run the project's checks now"),
    ("view", "Open the session in a full-screen view"),
];

/// The slash commands of 09-tui that this build does not have yet: one is
/// said to be missing, not sent to the model as a prompt.
const NOT_YET: &[&str] = &["lease", "extend", "mcp", "extensions"];

/// The panels' block and the bottom of a frame, and where the cursor goes
/// in the bottom: a row of it and a column, or none for a dialog.
struct Bottom {
    block: Vec<Line>,
    lines: Vec<Line>,
    cursor: Option<(usize, usize)>,
    /// The panels in a column of their own beside the transcript, when the
    /// full screen is wide enough ([`App::panel_column`]; `block` is then
    /// empty).
    column: Vec<Line>,
}

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
    /// Where a new session's commands start, below the root (`Start::cwd`).
    cwd: Option<String>,
    /// The terminal's capabilities, for the look of a theme chosen later.
    caps: Capabilities,
    /// The picker the application opened, and what its rows stand for.
    picker: Option<(String, Picker)>,
    /// Every block committed so far, for the viewer and the full-screen
    /// presenter, which draw them again at the width they have (since step
    /// 25; until then their lines, at the width they committed at).
    kept: Kept,
    /// The time zone the viewer's title says the session's start in,
    /// read when the viewer first opens.
    zone: Option<cineris_base::Zone>,
    /// The first and the last turn the transcript holds, for the viewer's
    /// title.
    turns: Option<(u64, u64)>,
    /// A stored session's history is being read: its events wait, and
    /// those it already holds (`seq` up to the read's) are dropped.
    reading: bool,
    /// The items the read has drawn so far, in order: `None` for one drawn
    /// whole, the `seq` of its page for one drawn as it was then, in
    /// progress.
    history_items: Vec<(String, Option<u64>)>,
    /// The `seq` of the last page read.
    history_seq: Option<u64>,
    /// What the last read holds, for its session's events taken after it.
    read_mark: Option<ReadMark>,
    /// The viewer, while it is open.
    viewer: Option<viewer::Viewer>,
    /// The full-screen presenter's view, when the application draws with
    /// it (`tui.presenter`, `--fullscreen`; since step 25).
    screen: Option<screen::ScreenView>,
    /// The kept answers' code blocks handed to the highlighter again, their
    /// colours having been late (since step 25).
    recolouring: Vec<recolour::Recolouring>,
    /// Kept blocks to be drawn again, for a view that was not at hand when
    /// they changed (the full screen's, while its frame is made).
    redraw_blocks: Vec<usize>,
    /// The worker that filters `@` completion off the key's path, once the
    /// workspace's paths have come.
    file_search: Option<super::files::FileSearch>,
    /// The terminal's waker, which the worker wakes the loop with.
    waker: Option<cineris_term::Waker>,
    /// The generation of the last `@` query handed to the worker.
    files_asked: u64,
    /// The workspace's files have been asked of the core: once, on the
    /// first `@` typed (since the fixes of the review of step 23), so that
    /// a session that never completes a path holds no list of them.
    files_listing: bool,
    /// The files read ahead for the prompt being written (at most
    /// [`WARM_FILES`] a prompt, as the core reads).
    warmed: usize,
    /// The person asked for the external editor (`Ctrl+G`).
    editor_wanted: bool,
    /// The settings `/new` opens a session with, and whether it is kept in
    /// memory only: as the command line asked for the first.
    fresh: (SessionSettings, bool),
    /// The session `--fork` forked, still open on this client until the
    /// fork is: it is closed then.
    forked_from: Option<SessionId>,
    /// Whether the open is under way: its events are held until it returns.
    opening: bool,
    held: Vec<SessionEvent>,
    /// Prompts sent before the session was open, and those sent while they
    /// are on their way: sent one at a time once it is, each when the one
    /// before it was answered, so that their order holds (step 27).
    held_prompts: VecDeque<String>,
    /// The held prompt on its way, by its ticket: the next waits for its
    /// answer.
    held_sent: Option<Ticket>,
    pending: HashMap<Ticket, Pending>,
    session: Option<SessionState>,
    models: Vec<ModelInfo>,
    usage: Option<SessionUsage>,
    /// What reads the look and the key map from the configuration
    /// ([`App::set_restyle`]).
    restyle: Option<Restyle>,
    /// The program's writer of `/export`'s files (since step 25).
    writer: Option<crate::export::Writer>,
    /// `/rewind`'s points being read (since step 25).
    points: Option<rewind::Reading>,
    /// A rewind from its point chosen to its answer.
    rewinding: Option<rewind::Rewinding>,
    /// The transcript being read again after a rewind or an undo.
    rereading: Option<rewind::Rereading>,
    /// The inline rewind's notice, and the words it is kept in.
    rewound_notice: Option<(String, String)>,
    /// What is said once the session being opened is: a fork's notice.
    open_said: Option<String>,
    /// How many of its own questions the application has asked.
    own_dialogs: u64,
    /// How many times the transcript has been read again.
    rereads: u64,
    /// An export under way.
    exporting: Option<export::Exporting>,
    /// The full screen's transcript is being read from under the dialog
    /// with this request's id, which stays up (since step 25). Reading ends
    /// with that dialog however it goes: another one never opens read.
    reading_transcript: Option<DialogId>,
    /// Back from reading under the dialog with this request's id: until it
    /// is ready again, keys and pastes hold it and go nowhere, never to the
    /// hidden editor (the focused review of step 25).
    back_from_reading: Option<DialogId>,
    /// A look at the configuration's files is due after this frame.
    look_due: bool,
    /// When the last look was asked for.
    last_look: Option<Instant>,
    /// The theme chosen with `/theme` for this run, which a reload keeps
    /// unless the file's own theme changed.
    theme_picked: Option<crate::app::ThemeChoice>,
    dialogs: VecDeque<Dialog>,
    /// When the running turn began, from this app's point of view.
    running: Option<Instant>,
    /// Whether a turn runs, as the core last said: the open, the history's
    /// read and the turn's events. The spinner runs from a prompt sent,
    /// before the core says; giving back held prompts stops it only when
    /// no turn runs (step 27).
    turn_runs: bool,
    last_interrupt: Option<Instant>,
    /// The last key read: a dialog that appears while the person is typing
    /// waits before it takes keys, and takes no key held down.
    last_key: Option<LastKey>,
    /// A dialog closed from outside while a note was being written in it
    /// (its request ended, or another client answered it), and when the
    /// last key since was read: the note is in the editor as a draft, and an
    /// `Enter` before a pause of [`QUIET`] does not send it (the review of
    /// step 23).
    note_cut: Option<Instant>,
    /// When the last input no dialog took was read: a key or a paste that
    /// went to the editor or the viewer. A dialog is ready only a quiet
    /// second after it, so typing that stopped just before a dialog came
    /// counts as typing at it (the review of these fixes).
    untaken: Option<Instant>,
    /// When a dialog last took a key: the next dialog is ready no sooner
    /// than the repeat window after it.
    taken: Option<Instant>,
    /// The panels (step 24): the plan's, the review's, the project's and
    /// the usage's.
    panels: Panels,
    /// The time of day, for what the panels say of times (an entry's age,
    /// a limit's reset).
    clock: cineris_base::SharedClock,
    /// The panel drawn with the focus in the last frame.
    panel_drawn: Option<u64>,
    /// When the last input no panel took was read: a key or a paste that
    /// went to the editor, keys read together. A panel that takes the
    /// focus is ready only a quiet second after it, so that typing that
    /// runs on past `/plan`'s `Enter` never acts on it. Moving the focus
    /// (`Ctrl+P`) is not such input.
    panel_untaken: Option<Instant>,
    /// The last key a panel took, when it or a repeat of it was last read,
    /// and whether only a repeat the terminal marks is taken for it held:
    /// the same key again within [`REPEAT`] is that key held down, and goes
    /// where its press went (nowhere); after a key that began an item edit,
    /// what is typed is the item's, and only a marked repeat is held.
    panel_held: Option<(Key, Instant, bool)>,
    /// The panel an item edit in the editor is for, and what it is for.
    item_edit: Option<(u64, ItemEditFor)>,
    /// The rows the panels themselves had in the last inline frame, as if
    /// no menu were open: what `/plan` measures a panel against.
    panel_room: usize,
    /// The panel a key's change was not sent for, while an earlier one of
    /// its was on its way: the hint row says the last change is still
    /// being made.
    panel_dropped: Option<u64>,
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
        let full_screen = start.settings.presenter == super::Presenter::Fullscreen;
        let clock: cineris_base::SharedClock =
            std::sync::Arc::new(cineris_base::SystemClock::new());
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
        // Every finished block is kept, for the viewer and the full-screen
        // presenter (step 25).
        transcript.keep_finished();
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
            cwd: start.cwd,
            caps: *caps,
            picker: None,
            kept: Kept::new(),
            screen: full_screen.then(screen::ScreenView::default),
            recolouring: Vec::new(),
            redraw_blocks: Vec::new(),
            zone: None,
            turns: None,
            viewer: None,
            file_search: None,
            waker: None,
            files_asked: 0,
            files_listing: false,
            warmed: 0,
            editor_wanted: false,
            reading: false,
            history_items: Vec::new(),
            history_seq: None,
            read_mark: None,
            forked_from: None,
            opening: false,
            held: Vec::new(),
            held_prompts: VecDeque::new(),
            held_sent: None,
            pending: HashMap::new(),
            session: None,
            models: Vec::new(),
            usage: None,
            dialogs: VecDeque::new(),
            restyle: start.restyle,
            writer: start.export,
            points: None,
            rewinding: None,
            rereading: None,
            rewound_notice: None,
            open_said: None,
            own_dialogs: 0,
            rereads: 0,
            exporting: None,
            reading_transcript: None,
            back_from_reading: None,
            look_due: false,
            last_look: None,
            theme_picked: None,
            running: None,
            turn_runs: false,
            last_interrupt: None,
            last_key: None,
            note_cut: None,
            untaken: None,
            taken: None,
            panels: panel::registered(&clock, None),
            clock,
            panel_drawn: None,
            panel_untaken: None,
            panel_held: None,
            item_edit: None,
            panel_room: 0,
            panel_dropped: None,
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
        self.look_changed();
        for dialog in &mut self.dialogs {
            dialog.restyled();
        }
    }

    /// Kept block `block` is to be drawn again by the viewer and the full
    /// screen: its colours came, say.
    pub(super) fn forget_block_rows(&mut self, block: usize) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.forget_block(block);
        }
        match self.screen.as_mut() {
            Some(screen) => screen.forget_block(block),
            // The full screen's view is out while its frame is made.
            None if self.settings.presenter == super::Presenter::Fullscreen => {
                self.redraw_blocks.push(block);
            }
            None => {}
        }
    }

    /// The look changed (the terminal's capabilities, a theme, a reload):
    /// the rows the viewer and the full-screen view made with the old one
    /// are dropped (since step 25).
    pub(super) fn look_changed(&mut self) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.forget_rows();
        }
        if let Some(screen) = self.screen.as_mut() {
            screen.forget_rows();
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
        self.look_now(at);
        // A dialog is shown by the inline screen's frame alone: under the
        // viewer it waits, its grace not begun (the review of the second
        // pull request).
        if self.viewing() {
            return;
        }
        self.panel_drawn_at(at);
        if let Some(dialog) = self.dialogs.front_mut() {
            let first = dialog.shown().is_none();
            dialog.drawn(at);
            // Its first frame: what was typed just before it, and the key
            // that answered the dialog before it, hold it as they would had
            // they come while it was up.
            if first {
                if let Some(untaken) = self.untaken {
                    dialog.typed(untaken);
                }
                if let Some(taken) = self.taken {
                    dialog.not_before(taken + REPEAT);
                }
            }
        }
    }

    /// A signal that ends the program came, its exit code `code`: a running
    /// turn is stopped at once (`turn/interrupt` with `now`), and the
    /// application ends.
    pub fn signalled(&mut self, code: u8) {
        if self.running.is_some()
            && let Some(session) = self.session.as_ref().map(|s| s.session_id.clone())
        {
            let ticket = self.link.send(Op::TurnInterrupt(TurnInterruptParams {
                session_id: session,
                now: true,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        self.exit = Some(Exit::Signal(code));
    }

    /// Whether the person asked for the external editor, taken: the loop
    /// hands the terminal over, runs it on [`editor_text`], and gives the
    /// result to [`edited`].
    ///
    /// [`editor_text`]: Self::editor_text
    /// [`edited`]: Self::edited
    pub fn take_editor(&mut self) -> bool {
        std::mem::take(&mut self.editor_wanted)
    }

    /// The prompt to edit, with its chips.
    #[must_use]
    pub fn editor_text(&self) -> crate::editor::Fragment {
        self.input.external_text()
    }

    /// The folders an editor found by its name is never taken from: the
    /// session's write roots, where a command the agent ran could have put
    /// a program.
    #[must_use]
    pub fn write_roots(&self) -> Vec<std::path::PathBuf> {
        self.session
            .as_ref()
            .map(|session| {
                session
                    .write_roots
                    .iter()
                    .map(std::path::PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What the external editor gave back, or why it gave nothing.
    pub fn edited(
        &mut self,
        result: Result<crate::external::Edited, crate::external::EditorError>,
    ) {
        // What was typed while the editor had the terminal was not typed at
        // the dialog: it starts its grace again.
        if let Some(dialog) = self.dialogs.front_mut() {
            dialog.hide();
        }
        self.panels.restart();
        match result {
            Ok(crate::external::Edited::Changed(fragment)) => {
                self.input.external_result(Some(fragment));
            }
            Ok(crate::external::Edited::Unchanged) => self.input.external_result(None),
            Ok(crate::external::Edited::Empty) => {
                self.input.external_result(None);
                self.say("The editor left the prompt empty, so it is kept as it was.");
            }
            Err(error) => {
                self.input.external_result(None);
                self.say(&format!("The external editor gave nothing back: {error}"));
            }
        }
    }

    /// The theme chosen for the run (`tui.theme`, or `/theme`).
    #[must_use]
    pub fn theme(&self) -> super::ThemeChoice {
        self.settings.theme
    }

    /// The terminal's waker: the `@` worker wakes the loop with it.
    pub fn set_waker(&mut self, waker: cineris_term::Waker) {
        self.waker = Some(waker);
    }

    /// Hands the `@` query the text asks to the worker. On the first `@`,
    /// asks the core for the workspace's files, which the menu waits for.
    fn ask_files(&mut self) {
        if !self.files_listing
            && self.input.wants_files()
            && let Some(session) = self.session.as_ref().map(|s| s.session_id.clone())
        {
            self.files_listing = true;
            let ticket = self.link.send(Op::WorkspaceFiles(
                cineris_protocol::methods::WorkspaceFilesParams {
                    session_id: session.clone(),
                    limit: None,
                },
            ));
            self.pending.insert(ticket, Pending::Files(session));
        }
        if let Some(search) = &self.file_search
            && let Some(query) = self.input.take_file_query()
        {
            self.files_asked = query.generation;
            search.ask(query);
        }
    }

    /// Takes the rows the worker sent: the input keeps those of its latest
    /// query only. Whether there were any.
    fn take_file_rows(&mut self) -> bool {
        let Some(search) = &self.file_search else {
            return false;
        };
        let answers = search.answers();
        let any = !answers.is_empty();
        for (generation, rows) in answers {
            self.input.file_rows(generation, rows);
        }
        any
    }

    /// Waits at most `limit` for the worker's rows for the last query asked,
    /// taking each answer before them as the loop would: for tests, which
    /// have no loop to wake.
    #[cfg(test)]
    pub(crate) fn wait_for_files(&mut self, limit: Duration) -> bool {
        self.ask_files();
        while let Some((generation, rows)) = self
            .file_search
            .as_ref()
            .and_then(|search| search.wait(limit))
        {
            self.input.file_rows(generation, rows);
            if generation == self.files_asked {
                return true;
            }
        }
        false
    }

    /// When the application must draw again whatever else happens: the end
    /// of an open dialog's grace, when its answers appear, until a frame
    /// with them has been drawn. The time may have passed, and a frame is
    /// then due at once. `None` while nothing is due, so that an idle
    /// interface sleeps.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        // Under the viewer no dialog is drawn, so none of its times is due.
        if self.viewing() {
            return None;
        }
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
    /// typing, though a stall held them back: input no dialog takes, which
    /// holds the dialog shown a quiet second, so that none of them answers.
    pub fn burst(&mut self, at: Instant) {
        self.last_key = Some(LastKey { typing: true });
        self.not_taken(at);
    }

    /// A terminal event, read at `at`. Says whether it was input (a key or a
    /// paste), whose frame is drawn at once.
    pub fn terminal(&mut self, event: &TermEvent, at: Instant) -> bool {
        self.drop_stale_viewer();
        // A key, a paste, or the focus regained: the configuration's files
        // are looked at after this frame (step 24's hot reload).
        if matches!(
            event,
            TermEvent::Key(_) | TermEvent::Paste(_) | TermEvent::Focus(true)
        ) {
            self.look_wanted(at);
        }
        if let Some(viewer) = &self.viewer {
            match event {
                TermEvent::Key(key) | TermEvent::Repeat(key) => {
                    // Reading a dialog's request whole is not typing at it;
                    // a search typed in the viewer is, and so is any key in
                    // it with no dialog up (one that comes next waits).
                    let searching = viewer.searching();
                    let dialog_up = !self.dialogs.is_empty();
                    if !dialog_up || searching {
                        self.not_taken(at);
                    }
                    self.last_key = Some(LastKey { typing: true });
                    let taken = self.viewer_key(*key, at);
                    // Back from the viewer, a panel's readiness starts
                    // again.
                    if self.viewer.is_none() {
                        self.panels.restart();
                    }
                    // Back from the viewer, the dialog starts its grace
                    // again, as if just shown.
                    if dialog_up
                        && self.viewer.is_none()
                        && let Some(dialog) = self.dialogs.front_mut()
                    {
                        dialog.hide();
                    }
                    return taken;
                }
                // The viewer's, not the hidden editor's (the review of step
                // 23): its search, while one is being typed.
                TermEvent::Paste(text) => {
                    self.not_taken(at);
                    return self.viewer_paste(text);
                }
                // The mouse (since step 25): not typing, as in the full
                // screen. A gesture bound to closing the viewer closes it as
                // its key does: a panel's readiness and a waiting dialog's
                // grace start again.
                TermEvent::Mouse(mouse) => {
                    let taken = self.viewer_mouse(*mouse);
                    if self.viewer.is_none() {
                        self.panels.restart();
                        if let Some(dialog) = self.dialogs.front_mut() {
                            dialog.hide();
                        }
                    }
                    return taken;
                }
                _ => {}
            }
        }
        // The full screen's transcript read from under a dialog (since step
        // 25): no key or paste read there, or just back before the dialog is
        // ready again, reaches the dialog or the hidden editor.
        if self.full_screen()
            && let Some(dialog) = self.dialogs.front()
        {
            let id = dialog.id.clone();
            let ready = dialog.takes_keys(at);
            let note = dialog.writing_note();
            let unreadable = dialog.unreadable();
            let reading = self.reading_transcript.as_ref() == Some(&id);
            let back = self.back_from_reading.as_ref() == Some(&id) && !ready;
            if self.back_from_reading.is_some() && !back {
                self.back_from_reading = None;
            }
            // Just back, `Ctrl+C`, `Ctrl+L` and the suspension act as they
            // do anywhere, and hold nothing (the review of the second pull
            // request).
            if back
                && let TermEvent::Key(key) | TermEvent::Repeat(key) = event
                && self.reading_global(*key, at)
            {
                return true;
            }
            match event {
                TermEvent::Key(key) if reading => return self.reading_key(*key, at, false),
                TermEvent::Repeat(key) if reading => return self.reading_key(*key, at, true),
                // Into reading: a press, not a repeat, once the dialog takes
                // keys; before, the key goes where it was going. Just back
                // at a dialog the window has grown too small for, which
                // takes no key until it grows, reading again is the way
                // on (the review of the second pull request).
                TermEvent::Key(key)
                    if (ready || (back && unreadable))
                        && !note
                        && self
                            .input
                            .keymap()
                            .lookup(&[crate::keys::Context::Dialog], *key)
                            == Some(crate::keys::Action::DialogTranscript) =>
                {
                    self.back_from_reading = None;
                    self.reading_transcript = Some(id);
                    return true;
                }
                TermEvent::Key(_) | TermEvent::Repeat(_) | TermEvent::Paste(_)
                    if back || (reading && matches!(event, TermEvent::Paste(_))) =>
                {
                    self.not_taken(at);
                    return true;
                }
                _ => {}
            }
        }
        // The full-screen presenter's own keys (step 25): scrolling and its
        // search, where neither the input nor a dialog has a use for them.
        // Moving the view is not typing at anything.
        if let TermEvent::Key(key) | TermEvent::Repeat(key) = event {
            let searching = self.screen_searching();
            if self.screen_key(*key) {
                // Typing into the search is typing, as into the editor: a
                // dialog that comes next waits for it to pause (the review
                // of step 25).
                if searching {
                    self.not_taken(at);
                    self.last_key = Some(LastKey { typing: true });
                }
                return true;
            }
        }
        match event {
            TermEvent::Key(key) | TermEvent::Repeat(key) => {
                // A key held down where the terminal says so; where it does
                // not, the quiet keeps its repeats from answering.
                let repeat = matches!(event, TermEvent::Repeat(_));
                let before = self.last_key.take();
                // The key before this one was one a dialog took.
                let after_taken = before.is_some_and(|before| !before.typing);
                let Some(dialog) = self.dialogs.front_mut() else {
                    // A key held down after a panel took it goes where its
                    // press went, nowhere, though the panel it acted on has
                    // gone (`x`) or given the input the focus (`e`).
                    if self.panel_holds(*key, at, repeat) {
                        return true;
                    }
                    // A panel that has the focus takes its keys (step 24).
                    if let Target::Panel(id) = self.input.focus().focused() {
                        return self.panel_key(id, *key, at, repeat);
                    }
                    self.last_key = Some(LastKey { typing: true });
                    self.untaken = Some(at);
                    // Moving the focus to a panel is not typing at it.
                    if self.input.keymap().lookup(&self.input.contexts(), *key)
                        != Some(crate::keys::Action::FocusNext)
                    {
                        self.panel_untaken = Some(at);
                    }
                    self.panel_held = None;
                    return self.edit_with(*key, at);
                };
                let ready = dialog.takes_keys(at);
                let keyed = dialog.key(*key, at, repeat, after_taken, self.input.keymap());
                // What the key was: typing, which holds the dialog a quiet
                // second; a key the dialog took, which holds the next one
                // (and a key held down after it, still the dialog's); or
                // neither (a global key, one it had no use for).
                let (holds, typing, took) = match &keyed {
                    Keyed::Pass => (true, true, false),
                    Keyed::Answer(_) | Keyed::Rewound(_) | Keyed::Changed | Keyed::ShowWhole => {
                        (false, false, true)
                    }
                    Keyed::Nothing if !ready => (true, !after_taken, false),
                    Keyed::Nothing if repeat => (false, false, true),
                    Keyed::Nothing | Keyed::Global(_) => {
                        (false, before.is_none_or(|before| before.typing), false)
                    }
                };
                self.last_key = Some(LastKey { typing });
                if holds {
                    self.untaken = Some(at);
                    self.panel_untaken = Some(at);
                    dialog.typed(at);
                }
                if took {
                    self.taken = Some(at);
                }
                // A key the dialog took is not a second `Ctrl+C`'s first: the
                // quit it armed is disarmed.
                if matches!(
                    keyed,
                    Keyed::Answer(_) | Keyed::Rewound(_) | Keyed::Changed | Keyed::ShowWhole
                ) {
                    self.input.disarm_quit();
                }
                match keyed {
                    // To the full screen's search, where it was going while
                    // the search is open, hidden under the dialog: never the
                    // hidden editor, whose `Enter` would send (the second
                    // review of step 25).
                    Keyed::Pass if self.screen_searching() => {
                        self.search_key(*key);
                        true
                    }
                    // It goes on to the editor, where it was going.
                    Keyed::Pass => self.edit_with(*key, at),
                    Keyed::Global(action) => {
                        // `Ctrl+C` with no turn running (the trust question,
                        // the undo's) leaves the text the dialog hides.
                        let outcome = if action == crate::keys::Action::Interrupt {
                            self.input.interrupt_hidden()
                        } else {
                            self.input.act(action)
                        };
                        self.outcome(outcome, at);
                        true
                    }
                    Keyed::Answer(response) => {
                        self.answer(response);
                        true
                    }
                    Keyed::Rewound(choice) => {
                        self.rewind_answered(choice);
                        true
                    }
                    Keyed::ShowWhole => {
                        self.open_whole();
                        true
                    }
                    Keyed::Changed | Keyed::Nothing => true,
                }
            }
            // A paste never answers a dialog: it is a note being written, or
            // it goes to the editor, as input the dialog waits a pause after.
            TermEvent::Paste(text)
                if self
                    .dialogs
                    .front_mut()
                    .is_some_and(|dialog| dialog.paste(text)) =>
            {
                true
            }
            // The full-screen search's, while it is open (step 25): typing,
            // as a paste into the editor is.
            TermEvent::Paste(text) if self.screen_paste(text) => {
                self.not_taken(at);
                self.last_key = Some(LastKey { typing: true });
                true
            }
            TermEvent::Paste(_) => {
                self.not_taken(at);
                self.last_key = Some(LastKey { typing: true });
                if let Some(cut) = &mut self.note_cut {
                    *cut = at;
                }
                let outcome = self.input.handle(event);
                self.outcome(outcome, at);
                true
            }
            // Continued after a stop (`fg`): what was typed meanwhile went to
            // the shell, and the dialog starts its grace again.
            TermEvent::Resumed => {
                if let Some(dialog) = self.dialogs.front_mut() {
                    dialog.hide();
                }
                self.panels.restart();
                false
            }
            TermEvent::Resize { .. } | TermEvent::Reply(_) => false,
            // The mouse in the full-screen view (step 25). It answers no
            // dialog and types into no panel, so it is not typing: a dialog
            // that comes after the wheel is ready as one after a pause is.
            TermEvent::Mouse(mouse) if self.screen.is_some() => self.screen_mouse(*mouse),
            TermEvent::Focus(_) | TermEvent::Mouse(_) => false,
        }
    }

    /// A key that goes to the editor, read at `at`.
    fn edit_with(&mut self, key: Key, at: Instant) -> bool {
        if self.holds_submit(key, at) {
            return true;
        }
        let outcome = self.input.handle(&TermEvent::Key(key));
        self.outcome(outcome, at);
        true
    }

    /// Input no dialog took was read at `at`: the dialog shown, if any,
    /// waits a quiet second after it, and hides answers it showed.
    fn not_taken(&mut self, at: Instant) {
        self.untaken = Some(at);
        self.panel_untaken = Some(at);
        self.panels.typed(at);
        if let Some(dialog) = self.dialogs.front_mut() {
            dialog.typed(at);
        }
    }

    /// Whether `key`, read at `at` and going to the editor, is an `Enter`
    /// that must not send: the editor holds a note cut off by its dialog's
    /// end, and the typing has not paused for [`QUIET`] since. Any other key
    /// carries the typing on.
    fn holds_submit(&mut self, key: Key, at: Instant) -> bool {
        let Some(since) = self.note_cut else {
            return false;
        };
        if at.saturating_duration_since(since) >= QUIET {
            self.note_cut = None;
            return false;
        }
        self.note_cut = Some(at);
        self.input
            .keymap()
            .lookup(&[crate::keys::Context::Editor], key)
            == Some(crate::keys::Action::Submit)
    }

    /// Dialogs closed from outside: their request ended (the turn's end, a
    /// time limit) or another client answered it. A note being written in
    /// one goes into the editor as a draft, said, and the `Enter` that was
    /// to send it does not send it as a prompt (the review of step 23).
    fn closed_from_outside(&mut self, closed: Vec<Dialog>, now: Instant) {
        for mut dialog in closed {
            self.dialog_gone(&dialog.id);
            if let Asks::Rewind { .. } = dialog.asks {
                self.rewind_question_closed();
                continue;
            }
            let Some(note) = dialog.take_note().filter(|note| !note.trim().is_empty()) else {
                continue;
            };
            self.say(
                "The request was settled before your note was sent: the note is in the input \
                 box, and was not sent.",
            );
            self.give_back(&note);
            self.note_cut = Some(now);
        }
    }

    /// A dialog left the queue, however it went: reading the transcript
    /// from under it, and being just back from that, end with it, so that
    /// the next dialog follows the end as any does (the review of step 25's
    /// second pull request).
    fn dialog_gone(&mut self, id: &DialogId) {
        if self.reading_transcript.as_ref() == Some(id) {
            self.reading_transcript = None;
        }
        if self.back_from_reading.as_ref() == Some(id) {
            self.back_from_reading = None;
        }
    }

    fn outcome(&mut self, outcome: Outcome, at: Instant) {
        match outcome {
            Outcome::Submit { text, delivery } => {
                self.warmed = 0;
                self.submit(text, delivery, at);
            }
            Outcome::FileChosen(path) => self.warm(path),
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
            Outcome::Forked(name, index) => self.forked(&name, index),
            Outcome::Removed(name, index) => self.removed(&name, index),
            Outcome::PickerClosed(_) => self.picker = None,
            Outcome::OpenViewer => self.open_viewer(None),
            Outcome::ExternalEditor => self.editor_wanted = true,
            outcome @ (Outcome::Focused(_)
            | Outcome::Move(..)
            | Outcome::Activate(Target::Panel(_))
            | Outcome::ItemEdited(_)
            | Outcome::ItemEditCancelled) => self.panel_outcome(outcome),
            _ => {}
        }
    }

    /// A prompt was sent from the editor.
    fn submit(&mut self, text: String, delivery: Delivery, at: Instant) {
        if self.slash_command(&text, at) {
            return;
        }
        self.last_cost = None;
        // The full-screen view follows the end again: what the prompt brings
        // is what the person wants to see.
        self.screen_follow();
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
        // The prompts held while the session opened are still on their way:
        // this one follows them, never ahead of one of them. While the
        // transcript is read again after a rewind, it waits for the read, so
        // that its turn is not read twice (since step 25).
        if self.held_sent.is_some() || self.rereading.is_some() {
            self.held_prompts.push_back(text);
            self.start_running(at);
            return;
        }
        let started = delivery == Delivery::Prompt;
        if started {
            self.start_running(at);
            // The next prompt: the last turn's review goes.
            self.panels_see(&Seen::Prompt);
        }
        let sent = Sent {
            session,
            text,
            started,
            stopped: false,
            again: false,
        };
        self.send_prompt(sent, wire(delivery));
    }

    /// Sends a prompt with `delivery`, its answer awaited.
    fn send_prompt(&mut self, sent: Sent, delivery: Option<ItemDelivery>) -> Ticket {
        let ticket = self.link.send(Op::TurnStart(TurnStartParams {
            session_id: sent.session.clone(),
            input: vec![Part::text(sent.text.clone())],
            delivery,
        }));
        self.pending.insert(ticket, Pending::Prompt(sent));
        ticket
    }

    /// A slash command this build has is done, and one it does not have
    /// yet is said to be missing and given back. Says whether `text` was
    /// either; anything else is a prompt.
    fn slash_command(&mut self, text: &str, at: Instant) -> bool {
        let Some(command) = text.strip_prefix('/') else {
            return false;
        };
        let (name, rest) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        if COMMANDS.iter().any(|(known, _)| *known == name) {
            self.command(name, rest.trim(), at);
            return true;
        }
        if NOT_YET.contains(&name) {
            self.say(&format!(
                "/{name} is not in this build yet. /help lists the commands there are."
            ));
            self.give_back(text);
            return true;
        }
        false
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
        // The held prompts not sent yet go now, queued behind the one on
        // its way and ahead of the interrupt: the core holds them with it,
        // in order, as it holds any queued prompt at an interrupt, and they
        // follow the next prompt (`docs/architecture/03-agent-loop.md`,
        // "Only a turn that completed starts the next prompt by itself").
        // Given back, one sent again would run ahead of one the core holds.
        // The one on its way is answered from here on as any prompt is.
        if self.held_sent.take().is_some() {
            while let Some(text) = self.held_prompts.pop_front() {
                let sent = Sent {
                    session: session.clone(),
                    text,
                    started: false,
                    stopped: false,
                    again: false,
                };
                self.send_prompt(sent, Some(ItemDelivery::Queue));
            }
        }
        // A prompt on its way that is refused comes back: queued behind the
        // turn the person stopped, it would run after the stop.
        for pending in self.pending.values_mut() {
            if let Pending::Prompt(sent) = pending {
                sent.stopped = true;
            }
        }
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
        self.dialog_gone(&dialog.id);
        if let Some(call) = dialog.call() {
            self.mapper.answered(&mut self.transcript, call);
        }
        if let DialogId::Request(id) = dialog.id {
            self.link.respond(id, response);
        }
    }

    // ------------------------------------------------------------- core

    fn send_open(&mut self, open: Open) {
        self.opening = true;
        let (op, pending) = match open {
            Open::New {
                workspace,
                settings,
                ephemeral,
            } => (
                new_session(workspace, self.cwd.clone(), settings, ephemeral),
                Pending::Open,
            ),
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
            .send(Op::ModelsList(cineris_protocol::methods::Empty {}));
        self.pending.insert(ticket, Pending::Models);
    }

    /// Takes what has arrived, at most `most` events of it. Says whether
    /// anything changed, and whether more is waiting.
    pub fn drain(&mut self, now: Instant, most: usize) -> Drained {
        let mut drained = Drained::default();
        if self.take_file_rows() {
            drained.changed = true;
        }
        // The full screen's search, and code coloured late, wake the loop
        // when their workers answer: what they bring is a change to draw
        // (the review of step 25).
        if self.take_search_answer() {
            drained.changed = true;
        }
        if self.take_colours() {
            drained.changed = true;
        }
        let mut taken = 0;
        while taken < most {
            let Some(inbound) = self.link.next() else {
                return drained;
            };
            taken += 1;
            drained.changed = true;
            match inbound {
                Inbound::Event(event) => {
                    if self.opening || self.reading {
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
        self.show_dialog(dialog);
    }

    /// A dialog is to be answered, after those before it.
    fn show_dialog(&mut self, dialog: Dialog) {
        // A dialog asks about what just happened, at the end; not while the
        // transcript is read from under one, whose place is kept.
        if self.reading_transcript.is_none() {
            self.screen_follow();
        }
        self.dialogs.push_back(dialog);
        // No panel is drawn while a dialog is up, and none keeps the focus;
        // after it, a panel's readiness starts again.
        if matches!(self.input.focus().focused(), Target::Panel(_)) {
            self.input.focus_mut().to_input();
        }
        self.panels.restart();
    }

    fn done(&mut self, ticket: Ticket, result: Result<OpResult, ErrorData>, now: Instant) {
        let Some(pending) = self.pending.remove(&ticket) else {
            return;
        };
        match (pending, result) {
            (Pending::Open, Ok(OpResult::SessionNew(result))) => {
                self.opened(result.session, false, now);
            }
            (Pending::Open, Ok(OpResult::SessionOpen(result) | OpResult::SessionFork(result))) => {
                self.opened(result.session, true, now);
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
                        new_session(workspace, self.cwd.clone(), settings, ephemeral)
                    }
                };
                let ticket = self.link.send(op);
                self.pending.insert(ticket, Pending::Open);
            }
            (Pending::Models, Ok(OpResult::ModelsList(ModelsListResult { models }))) => {
                self.models = models;
            }
            (Pending::Prompt(sent), result) => self.prompt_answered(ticket, sent, result, now),
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
            // A result for a session left meanwhile is not this one's.
            (Pending::Undo(id) | Pending::Set(id) | Pending::Config(_, id), _)
                if !self.is_open(&id) => {}
            // The next session's first `@` asks for its own.
            (Pending::Files(id), _) if !self.is_open(&id) => self.files_listing = false,
            (Pending::History(id), _) if !(self.reading && self.is_open(&id)) => {}
            // A panel's, for a session left meanwhile, is dropped.
            (Pending::Panel { session, .. } | Pending::Opened { session, .. }, _)
                if !self.is_open(&session) => {}
            (Pending::Panel { panel, .. } | Pending::Opened { panel, .. }, Ok(result)) => {
                self.panel_result(panel, &result);
            }
            // What a panel asked as the session opened, which the core has
            // nothing of here: nothing the person asked for.
            (Pending::Opened { panel, .. }, Err(error)) if error.code == "not_available" => {
                // The panel hears of it, and its rows stop `reading…`.
                if self.panels.failed(panel, &error) {
                    self.panel_closed(panel);
                }
            }
            // The panel hears of it (its rows stop `reading…`), and the
            // error is said.
            (Pending::Panel { panel, .. } | Pending::Opened { panel, .. }, Err(error)) => {
                if self.panels.failed(panel, &error) {
                    self.panel_closed(panel);
                }
                self.error(&error);
            }
            (Pending::Undo(_), Ok(OpResult::SessionUndo(result))) => {
                let session = result.session.session_id.clone();
                self.session = Some(result.session);
                self.say(
                    "The last turn is undone: it is out of the conversation, and the files it \
                     changed are as they were before it.",
                );
                // Drawn as a rewind's is: the viewer's and the full screen's
                // transcript read again (since step 25).
                self.read_again(session);
            }
            (Pending::Points(id), Ok(OpResult::SessionRead(page))) => self.points_read(&id, page),
            // An answer of another kind ends what waited for it as an error
            // does, so that nothing is left waiting (the review of this
            // part).
            (Pending::Points(id), result) => {
                self.points_unread(&id, &result.err().unwrap_or_else(unexpected));
            }
            (Pending::RewindPrompt(id), Ok(OpResult::SessionRead(page))) => {
                self.rewind_prompt_read(&id, &page);
            }
            // Said, if it matters, when the rewind is done.
            (Pending::RewindPrompt(_), _) => {}
            (Pending::RewindPlan(id), Ok(OpResult::SessionRewindPlan(plan))) => {
                self.rewind_planned(&id, plan);
            }
            (Pending::RewindPlan(id) | Pending::Rewind(id), result)
                if !matches!(
                    result,
                    Ok(OpResult::SessionRewindPlan(_) | OpResult::SessionRewind(_))
                ) =>
            {
                self.rewind_failed(&id, &result.err().unwrap_or_else(unexpected));
            }
            (Pending::Rewind(id), Ok(OpResult::SessionRewind(result))) => {
                self.rewound(&id, result.session);
            }
            (Pending::Reread(id, number), Ok(OpResult::SessionRead(page))) => {
                self.reread_page(&id, number, page, now);
            }
            (Pending::Reread(id, number), result) => {
                self.reread_failed(&id, number, &result.err().unwrap_or_else(unexpected), now);
            }
            (Pending::Fork(forking), Ok(OpResult::SessionFork(result))) => {
                self.fork_opened(*forking, result.session, now);
            }
            (Pending::Fork(forking), result) => {
                self.fork_failed(*forking, &result.err().unwrap_or_else(unexpected), now);
            }
            (Pending::Permissions, Ok(OpResult::PermissionsList(list))) => {
                self.permissions_listed(list.approvals);
            }
            (Pending::Profiles(id), _) if !self.is_open(&id) => {}
            (Pending::Artifact { session, .. }, _) if !self.is_open(&session) => {}
            (Pending::Artifact { label, .. }, Ok(OpResult::SessionArtifact(artifact))) => {
                self.open_text(&label, &artifact);
            }
            (Pending::Config(shown, _), Ok(OpResult::ConfigGet(result))) => {
                self.config_shown(shown, &result.config, &result.origins);
            }
            (Pending::Profiles(_), Ok(OpResult::ConfigGet(result))) => {
                self.profiles_listed(&result.profiles, result.profile.as_deref());
            }
            (Pending::Reload, Ok(OpResult::ConfigReload(result))) => {
                self.reloaded(&result);
            }
            (Pending::Look, Ok(OpResult::ConfigReload(result))) => {
                self.restyle_if(&result.applied);
            }
            // A look's failure is the core's to say, when there is one.
            (Pending::Look, _) => {}
            (Pending::Restyle { keys, theme }, Ok(OpResult::ConfigGet(result))) => {
                self.restyled(&result.config, keys, theme);
            }
            (Pending::Set(_), Ok(OpResult::SessionSet(result))) => {
                self.session = Some(result.session);
            }
            (Pending::History(id), Ok(OpResult::SessionRead(page))) => {
                self.history_read(id, page, now);
            }
            (Pending::Summary(id), Ok(OpResult::SessionRead(page))) if self.is_open(&id) => {
                self.summary_read(ticket, &page);
            }
            // Said where it was asked for, and as any failure is.
            (Pending::Summary(_), Err(error)) => {
                self.summary_unread(ticket);
                self.error(&error);
            }
            (Pending::Export(id), Ok(OpResult::SessionRead(page))) => {
                self.export_read(&id, page);
            }
            (Pending::Export(id), result) => {
                self.export_unread(&id);
                if let Err(error) = result {
                    self.error(&error);
                }
            }
            (Pending::Files(_), Ok(OpResult::WorkspaceFiles(files))) => {
                // Listed and matched on a thread of their own: at the
                // core's cap either takes longer than a key's budget. With no
                // thread, on the key's path. The paths are moved into the
                // one compact list, never copied.
                match super::files::FileSearch::start(files.paths, self.waker.clone()) {
                    Ok(search) => {
                        self.file_search = Some(search);
                        self.input.filter_files_elsewhere();
                        self.ask_files();
                    }
                    Err(paths) => self.input.set_files(paths),
                }
            }
            // The `@` menu says why, and the next one asks again; this one
            // asks nothing more.
            (Pending::Files(_), Err(error)) => {
                self.files_listing = false;
                self.input.files_unlisted();
                self.error(&error);
            }
            // What the pages before the one that failed drew is dealt with
            // as a read's, but nothing is taken for cut off.
            (Pending::History(id), Err(error)) => {
                let seq = self.history_seq;
                self.read_ended(id, seq, false, now);
                self.error(&error);
            }
            (Pending::Warm, _) => {}
            (_, Err(error)) => self.error(&error),
            _ => {}
        }
    }

    /// The session could not be opened: what was held while it was being
    /// opened is shown or given back, and why is said. A fork's original,
    /// opened for it, is closed.
    fn open_failed(&mut self, error: Option<&ErrorData>, now: Instant) {
        self.opening = false;
        self.open_said = None;
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

    /// A prompt's answer. One refused `turn_running` is sent again once,
    /// queued behind the turn the core started meanwhile (step 27). Any
    /// other refusal, that one's second, or one after an interrupt brings it
    /// back into the editor with why, and the held prompts that wait for it
    /// after it, in order; an accepted held one lets the next go.
    fn prompt_answered(
        &mut self,
        ticket: Ticket,
        sent: Sent,
        result: Result<OpResult, ErrorData>,
        now: Instant,
    ) {
        let held = self.held_sent == Some(ticket);
        if held {
            self.held_sent = None;
        }
        let refused = match result {
            // Sent with no delivery as one turn ended and the next the core
            // took from its queue began, or onto a turn another client
            // began: it runs after that turn.
            Err(error)
                if error.code == code::TURN_RUNNING
                    && !sent.again
                    && !sent.stopped
                    && self.is_open(&sent.session) =>
            {
                let again = Sent {
                    started: false,
                    again: true,
                    ..sent
                };
                let ticket = self.send_prompt(again, Some(ItemDelivery::Queue));
                if held {
                    self.held_sent = Some(ticket);
                }
                return;
            }
            Err(error) => {
                // What began the spinner ends it: this prompt, or the held
                // prompts given back with it when no turn runs.
                if (sent.started || (held && !self.turn_runs)) && self.running.is_some() {
                    self.stop_running();
                }
                self.error(&error);
                self.give_back(&sent.text);
                true
            }
            Ok(OpResult::TurnStart(result)) => {
                if result.disposition == Disposition::Started {
                    self.had_turn = true;
                }
                false
            }
            Ok(_) => false,
        };
        if !held {
            return;
        }
        if refused || !self.is_open(&sent.session) {
            self.give_back_held();
        } else {
            self.send_held(false, now);
        }
    }

    /// `/new`: the session open now is closed, and a new one opened in the
    /// workspace, with its own header.
    fn new_session(&mut self) {
        if self.running.is_some() {
            self.say("A turn is running: wait for it to end, or stop it with esc.");
            return;
        }
        // What follows a held prompt on its way is this session's.
        if self.held_sent.is_some() {
            self.say(
                "Your prompts are still being sent: /new can start another session once they are.",
            );
            return;
        }
        if self.opening || self.reading {
            self.say("A session is being opened: /new can start another once it is.");
            return;
        }
        // What a rewind holds (a prompt sent while the transcript is read
        // again) is this session's (since step 25).
        if self.rewind_under_way() {
            self.say("A rewind is under way: /new can start another session once it is done.");
            return;
        }
        if let Some(session) = self.session.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: session.session_id,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        self.left_session();
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
        // Into the draft, not into an item being edited.
        self.input.cancel_item_edit();
        self.item_edit = None;
        // What is there keeps its chips.
        let mut now = self.input.editor().fragment();
        if !now.is_empty() {
            now.push(Fragment::text("\n"));
        }
        now.push(Fragment::text(text));
        self.input.external_result(Some(now));
    }

    /// A session is open; `stored` when it was opened from the store (a
    /// resume, a switch, a fork), whose history is then drawn.
    fn opened(&mut self, state: SessionState, stored: bool, now: Instant) {
        self.opening = false;
        if let Some(original) = self.forked_from.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: original,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        if let Some(said) = self.open_said.take() {
            self.say(&said);
        }
        let header = self.header_of(&state);
        self.transcript.push(Block::Header(header));
        // A stored session's history is drawn under its header before
        // anything that happens next (`docs/architecture/09-tui.md`,
        // "Resume"); its events wait for it.
        let session_id = state.session_id.clone();
        self.left_session();
        self.turn_runs = state.running_turn.is_some();
        self.session = Some(state);
        // What the panels show of a session it already holds (its plan),
        // and of its project (the entries that wait).
        self.panels_ask(stored);
        // The workspace's files for `@` are listed on the first `@` typed
        // (`ask_files`), not here.
        if stored {
            self.reading = true;
            self.read_mark = None;
            self.read_history(session_id, None);
        } else {
            let held = std::mem::take(&mut self.held);
            for event in held {
                self.event(event, now);
            }
        }
        let held = !self.held_prompts.is_empty();
        self.send_held(true, now);
        // Every one held was a command: no turn is on its way.
        if held && self.held_sent.is_none() {
            self.stop_running();
        }
    }

    /// Sends the next prompt held while the session opened, the one before
    /// it answered (step 27): the `first` as a new turn, or steering the
    /// turn the open says is running, as `Enter` would; each after it
    /// queued behind it. The command line's prompt is a command when it is
    /// one, as a typed one is: it was never through `submit`.
    fn send_held(&mut self, first: bool, now: Instant) {
        while let Some(text) = self.held_prompts.pop_front() {
            if self.slash_command(&text, now) {
                continue;
            }
            let Some(session) = self.session.as_ref() else {
                self.held_prompts.push_front(text);
                self.give_back_held();
                return;
            };
            let (delivery, started) = if !first {
                (Some(ItemDelivery::Queue), false)
            } else if session.running_turn.is_some() {
                (Some(ItemDelivery::Steer), false)
            } else {
                (None, true)
            };
            let sent = Sent {
                session: session.session_id.clone(),
                text,
                started,
                stopped: false,
                again: false,
            };
            self.held_sent = Some(self.send_prompt(sent, delivery));
            return;
        }
    }

    /// Asks for a page of the session's history.
    fn read_history(&mut self, session_id: SessionId, cursor: Option<String>) {
        let ticket = self.link.send(Op::SessionRead(
            cineris_protocol::methods::SessionReadParams {
                session_id: session_id.clone(),
                item_id: None,
                limit: Some(HISTORY_PAGE),
                cursor,
                redacted: false,
            },
        ));
        self.pending.insert(ticket, Pending::History(session_id));
    }

    /// A page of the history of `session`: its finished items drawn as they
    /// ended, and those in progress drawn as they are, to go on live
    /// (`docs/architecture/11-rpc-protocol.md`, `session/read`). At its last
    /// page, the events held meanwhile follow, but for those the read
    /// already holds.
    fn history_read(
        &mut self,
        session: SessionId,
        page: cineris_protocol::methods::SessionReadResult,
        now: Instant,
    ) {
        for item in page.items {
            let incomplete = item
                .id()
                .is_some_and(|id| page.incomplete.iter().any(|open| open.as_str() == id));
            if let Some(id) = item.id() {
                self.history_items
                    .push((id.to_owned(), incomplete.then_some(page.seq)));
            }
            // Each prompt began a turn, a queued one too, and a steering
            // message did not: the history's turns are counted from one.
            if let cineris_protocol::Item::UserMessage(message) = &item
                && message.delivery.as_ref() != Some(&ItemDelivery::Steer)
            {
                let number = self.turns.map_or(1, |(_, last)| last + 1);
                self.note_turn(number);
            }
            if incomplete {
                self.mapper.in_progress(&mut self.transcript, &item, now);
            } else {
                self.mapper.event(
                    &mut self.transcript,
                    &Event::ItemCompleted(cineris_protocol::event::ItemCompleted { item }),
                    now,
                );
            }
        }
        // An item that still gets events is among the last the read drew:
        // only those are kept, not every id of a long history.
        let older = self
            .history_items
            .len()
            .saturating_sub(HISTORY_PAGE as usize);
        self.history_items.drain(..older);
        self.history_seq = Some(page.seq);
        if let Some(cursor) = page.next_cursor {
            self.read_history(session, Some(cursor));
            return;
        }
        self.read_ended(session, Some(page.seq), true, now);
    }

    /// How many items the read in progress keeps the ids of.
    #[cfg(test)]
    pub(super) fn history_kept(&self) -> usize {
        self.history_items.len()
    }

    /// The read of `session`'s history has ended, its last page read at
    /// `seq` (none when its first page failed), `whole` when no page failed.
    /// Whether a turn ran at the read is decided, and the events held
    /// meanwhile follow, but for those the read already holds; so do those
    /// taken after it, which a link may give after the read's result though
    /// they were queued before it.
    fn read_ended(&mut self, session: SessionId, seq: Option<u64>, whole: bool, now: Instant) {
        self.reading = false;
        self.history_seq = None;
        let drawn: HashMap<String, Option<u64>> = std::mem::take(&mut self.history_items)
            .into_iter()
            .collect();
        let held = std::mem::take(&mut self.held);
        // Whether a turn was running when the read was taken: an item in
        // progress says so, and else this session's turn events held up to
        // it.
        let mut running = self
            .session
            .as_ref()
            .is_some_and(|session| session.running_turn.is_some());
        for event in held
            .iter()
            .filter(|event| event.session_id == session && seq.is_none_or(|seq| event.seq <= seq))
        {
            match &event.event {
                Event::TurnStarted(_) => running = true,
                Event::TurnCompleted(_) => running = false,
                Event::SessionUpdated(updated) => {
                    running = updated.session.running_turn.is_some();
                }
                _ => {}
            }
        }
        if drawn.values().any(Option::is_some) {
            running = true;
        }
        self.turn_runs = running;
        if running {
            // A turn this client did not start (another's, or one begun
            // before the open) is running: `Enter` steers it.
            self.start_running(now);
        } else if whole {
            // A call the history holds with no result was cut off. With a
            // page missing, its result may be on that page.
            self.mapper.turn_done(&mut self.transcript, now);
        }
        self.read_mark = seq.map(|seq| ReadMark {
            session,
            seq,
            drawn,
            whole,
        });
        for event in held {
            self.event(event, now);
        }
    }

    /// Whether `session` is the one open here.
    fn is_open(&self, session: &SessionId) -> bool {
        self.session
            .as_ref()
            .is_some_and(|open| open.session_id == *session)
    }

    fn event(&mut self, event: SessionEvent, now: Instant) {
        match &self.session {
            Some(session) if session.session_id != event.session_id => return,
            // With none open, an event is of one closed or that failed to
            // open: only what it said is shown.
            None if !matches!(event.event, Event::Notice(_)) => return,
            _ => {}
        }
        if let Some(mark) = &self.read_mark
            && mark.session == event.session_id
        {
            if mark.holds(&event) {
                return;
            }
            // A turn begun or ended after the read: what the read held has
            // all come.
            if event.seq > mark.seq
                && matches!(event.event, Event::TurnStarted(_) | Event::TurnCompleted(_))
            {
                self.read_mark = None;
            }
        }
        // While the transcript is read again, what comes live is drawn
        // from its events, and not again from the read (since step 25).
        if self.rereading.is_some()
            && let Some(item) = item_of(&event.event)
        {
            self.seen_while_read_again(&event.session_id, item);
        }
        match &event.event {
            Event::TurnStarted(_) => {
                if let Some(number) = event
                    .turn_id
                    .as_ref()
                    .and_then(|turn| turn.as_str().strip_prefix("t_")?.parse().ok())
                {
                    self.note_turn(number);
                }
                self.had_turn = true;
                self.turn_runs = true;
                self.start_running(now);
            }
            Event::TurnCompleted(completed) => {
                self.turn_runs = false;
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
                        cost: panel::with_verdict(
                            cost(&completed.usage),
                            completed.verification.as_ref(),
                            &self.look,
                        ),
                    });
                }
                let session = self.usage.as_ref().and_then(|usage| cost(&usage.totals));
                if let Some(turn) = cost(&completed.usage) {
                    self.last_cost = Some((turn, session.unwrap_or_default()));
                }
                let closed: Vec<Dialog> = self.dialogs.drain(..).collect();
                self.closed_from_outside(closed, now);
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
            // What a reload changed, found by another client's reload or by
            // the core's look before a turn. This application's own
            // `/reload` says it from its result, once.
            Event::ConfigChanged(changed) => {
                let asked = self
                    .pending
                    .values()
                    .any(|pending| matches!(pending, Pending::Reload));
                if !asked {
                    for line in reload_lines(&changed.applied, &changed.errors, &changed.notes) {
                        self.say(&line);
                    }
                }
                // Read again whoever's reload it was: one asked for already
                // takes it in.
                self.restyle_if(&changed.applied);
            }
            Event::Notice(notice) => {
                if let Some(message) = notice_text(notice) {
                    self.transcript.push(Block::Notice(message));
                }
            }
            Event::RequestResolved(resolved) => {
                if let Some(at) = self
                    .dialogs
                    .iter()
                    .position(|dialog| dialog.is_for(&resolved.request_id))
                    && let Some(dialog) = self.dialogs.remove(at)
                {
                    if let Some(call) = dialog.call() {
                        self.mapper.answered(&mut self.transcript, call);
                    }
                    self.closed_from_outside(vec![dialog], now);
                }
            }
            _ => self.mapper.event(&mut self.transcript, &event.event, now),
        }
        // The notices of a session not open (one that failed to open, one
        // closed) are said, and are nothing of a panel's.
        if self.is_open(&event.session_id) {
            self.panels_event(&event.event);
        }
    }

    /// A file named with `@` is read ahead by the core, so that the model's
    /// first read of it finds it in the system's cache (the features
    /// table's prefetch; `workspace/warm`). A folder is not a file to read.
    fn warm(&mut self, path: String) {
        if path.ends_with('/') || self.warmed >= WARM_FILES {
            return;
        }
        let Some(session) = self.session.as_ref().map(|s| s.session_id.clone()) else {
            return;
        };
        self.warmed += 1;
        let ticket = self.link.send(Op::WorkspaceWarm(
            cineris_protocol::methods::WorkspaceWarmParams {
                session_id: session,
                paths: vec![path],
            },
        ));
        self.pending.insert(ticket, Pending::Warm);
    }

    pub(super) fn say(&mut self, text: &str) {
        self.transcript.push(Block::Notice(text.to_owned()));
    }

    /// What a reload did, said: the keys in force now, the core's notes
    /// (what waits, and for what), and what was wrong.
    fn reloaded(&mut self, result: &cineris_protocol::methods::ConfigReloadResult) {
        for line in reload_lines(&result.applied, &result.errors, &result.notes) {
            self.say(&line);
        }
        self.restyle_if(&result.applied);
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
        let (commit, boxes) = self.prepare(now, width);
        let mut live = self.transcript.live(width, &self.look, self.tick);
        let Bottom {
            block,
            lines: bottom,
            cursor,
            ..
        } = self.bottom(now, width, &boxes, live.len(), None);
        let half = (self.rows / 2).max(1);

        // The live region is at most half the window, and at least the
        // bottom: what streams is cut from its top, with a mark.
        let room = half
            .max(bottom.len())
            .saturating_sub(bottom.len() + block.len());
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
        let offset = live.len() + block.len();
        live.extend(block);
        live.extend(bottom);
        let cursor = match cursor {
            Some((row, col)) => Cursor {
                row: offset + row,
                col,
            },
            // A dialog has no text cursor: it rests at the region's end.
            None => Cursor {
                row: live.len().saturating_sub(1),
                col: 0,
            },
        };
        (commit, live, cursor)
    }

    /// What every frame does first, in both presenters (since step 25):
    /// the clock, the `@` worker's query, the live rounds, and the lines
    /// that became final, the blocks they finish kept. Gives the lines and
    /// the live boxes' ids.
    fn prepare(&mut self, now: Instant, width: usize) -> (Vec<Line>, Vec<u64>) {
        // What the text asks of `@` goes to the worker before the frame:
        // the frame shows the rows it has, and the worker's come after.
        self.ask_files();
        // An export's writer that has answered.
        self.take_export();
        // The spinner's frame follows the clock, one every `SPIN`.
        let since = now.saturating_duration_since(self.began).as_millis();
        self.tick = usize::try_from(since / super::SPIN.as_millis()).unwrap_or(0);
        self.mapper.tick(&mut self.transcript, now);
        self.transcript
            .set_now(now.saturating_duration_since(self.began));
        let boxes = self.mapper.live_rounds(&mut self.transcript);
        let parts = self.transcript.commit_parts(width, &self.look);
        let commit: Vec<Line> = parts.into_iter().flat_map(Committed::into_lines).collect();
        let before = self.kept.len();
        let finished = self.transcript.take_finished();
        let finished = self.kept_words(finished);
        self.kept.extend(finished);
        // Code whose colours were late is asked for again, and what came of
        // it drawn (step 25).
        self.ask_colours(before);
        self.take_colours();
        (commit, boxes)
    }

    /// The panels' block and the bottom under the transcript's `live` rows,
    /// in both presenters (since step 25): a dialog in the input box's place,
    /// or the input box with the menu or the hint row under it, and the
    /// panels above it.
    /// `column`, in the full screen on a wide window: the panels go in a
    /// column of this many columns beside the transcript instead.
    fn bottom(
        &mut self,
        now: Instant,
        width: usize,
        boxes: &[u64],
        live: usize,
        column: Option<usize>,
    ) -> Bottom {
        let mut bottom = Vec::new();
        let mut side = Vec::new();
        let cursor_row;
        let mut cursor_col = 0;
        let half = (self.rows / 2).max(1);
        // The panels' block (step 24), between the live rows and the bottom.
        let mut block = Vec::new();
        self.panel_drawn = None;
        if !self.dialogs.is_empty() {
            // No panel is drawn while a dialog is up.
            self.input.focus_mut().update(boxes, &[]);
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
            // The presenter, not the view, which a full-screen frame has
            // taken out while it draws.
            let full_screen = self.settings.presenter == super::Presenter::Fullscreen;
            let reading = full_screen
                && self.reading_transcript.is_some()
                && self.dialogs.front().map(|dialog| &dialog.id)
                    == self.reading_transcript.as_ref();
            let (lines, keys) = match self.dialogs.front_mut() {
                // Reading the transcript (since step 25): the dialog stays up
                // as its first line, and its row says how to read and how to
                // go back.
                Some(dialog) if reading => {
                    // The question's first row, cut to leave room for the way
                    // back, which a window too small for the keys row still
                    // shows.
                    let back = format!("  {}", reading_back(keymap));
                    let room = width.saturating_sub(crate::editor::cluster::width(&back));
                    let first = dialog
                        .view(keymap, rows, width, &self.look, now)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let mut line = crate::layout::cut(&first, room, self.look.glyphs.ellipsis);
                    line.push(cineris_term::Span::new(
                        back,
                        self.look.theme.style(crate::theme::Token::Hint),
                    ));
                    (vec![line], reading_keys(keymap))
                }
                Some(dialog) => {
                    // Drawn first: the frame that draws the answers offers
                    // what they make pressable, and a dialog with nothing
                    // moving gets no frame after it (the review of the
                    // second pull request).
                    let lines = dialog.view(keymap, rows, width, &self.look, now);
                    let mut keys = dialog.hint_keys(keymap);
                    // Offered once it can be pressed, after the key that
                    // refuses, so that a narrow row still shows it.
                    if full_screen
                        && !dialog.writing_note()
                        && dialog.takes_keys(now)
                        && let Some(key) = keymap
                            .keys_of(crate::keys::Action::DialogTranscript)
                            .first()
                    {
                        let at = keys.len().min(1);
                        keys.insert(at, format!("{} reads above", crate::keys::name(*key)));
                    }
                    (lines, keys)
                }
                None => (Vec::new(), Vec::new()),
            };
            bottom.extend(lines);
            if chrome {
                bottom.push(Line::new());
                // A first `Ctrl+C` with no turn running says what a second
                // does, over the dialog's keys.
                let hint = if self.input.quit_armed() {
                    Hint::QuitArmed
                } else {
                    Hint::Keys(keys)
                };
                bottom.push(self.hint_row(hint, now));
            }
            cursor_row = None;
        } else {
            let on_input = self.input.focus().on_input();
            let mut drawn = view::input::input(&self.input, &self.look, width);
            let menu = view::menu::menu(&self.input, &self.look, width);
            // The bottom's height does not hang on the focus: the panels
            // are laid out in what it leaves, and the focus ring is given
            // the panels that can be drawn focused.
            let height = drawn.lines.len() + menu.as_ref().map_or(1, Vec::len);
            let room = half.max(height) - height;
            // What the panels would have with the hint row, not a menu:
            // what `/plan`, typed with its menu open, is measured against.
            let plain = drawn.lines.len() + 1;
            if let Some(column) = column {
                // Beside the transcript: the column's rows are all those
                // above the bottom.
                let rows = self.rows.saturating_sub(height);
                self.panel_room = rows;
                side = self.panel_column(rows, column, menu.is_some(), boxes);
            } else {
                self.panel_room = self.panel_rows(half.max(plain) - plain, live);
                block = self.panel_block(room, live, menu.is_some(), width, boxes);
            }
            if self.input.focus().on_input() != on_input {
                drawn = view::input::input(&self.input, &self.look, width);
            }
            let top = bottom.len();
            bottom.extend(drawn.lines);
            cursor_row = Some(top + drawn.cursor.row);
            cursor_col = drawn.cursor.col;
            match menu {
                Some(menu) => bottom.extend(menu),
                None => {
                    let left = self.hint_left(now);
                    bottom.push(self.hint_row(left, now));
                }
            }
        }
        Bottom {
            block,
            lines: bottom,
            cursor: cursor_row.map(|row| (row, cursor_col)),
            column: side,
        }
    }

    fn hint_left(&self, now: Instant) -> Hint {
        if self.input.quit_armed() {
            return Hint::QuitArmed;
        }
        if let Some(keys) = self.panel_hint() {
            return Hint::Keys(keys);
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

/// What a finished read of a session's history holds: the events of that
/// session it already drew are not drawn again, whether they were held
/// while it ran or taken after its result, which the link may give before
/// the events queued ahead of it.
#[derive(Debug)]
struct ReadMark {
    session: SessionId,
    /// The `seq` of its last page.
    seq: u64,
    /// The last page's worth of the items it drew: `None` for one drawn
    /// whole, the `seq` of its page for one drawn in progress.
    drawn: HashMap<String, Option<u64>>,
    /// No page failed: an item it does not hold, of before its `seq`, came
    /// to nothing. With a page missing, it may be on that page.
    whole: bool,
}

impl ReadMark {
    /// Whether the read already drew `event`: of an item it drew whole, any;
    /// of one it drew in progress, those up to its page's `seq`; of any
    /// other item, those up to the read's (an item that came to nothing),
    /// when no page failed.
    fn holds(&self, event: &SessionEvent) -> bool {
        if event.session_id != self.session {
            return false;
        }
        let Some(id) = item_of(&event.event) else {
            return false;
        };
        match self.drawn.get(id) {
            Some(None) => true,
            Some(Some(seq)) => event.seq <= *seq,
            None => self.whole && event.seq <= self.seq,
        }
    }
}

/// The error an answer of another kind than the one asked for stands for.
fn unexpected() -> ErrorData {
    ErrorData::new(
        cineris_protocol::ErrorKind::Internal,
        "unexpected_result",
        "The core answered with something other than what was asked for.",
    )
}

/// `text` on one line, its runs of white space one space, cut after
/// `most` characters with `ellipsis` (since step 25: a fork's note in
/// `/sessions`). Sanitised where it is drawn.
pub(super) fn one_line_cut(text: &str, most: usize, ellipsis: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= most {
        return line;
    }
    let mut cut: String = line.chars().take(most).collect();
    // At the end of a word, where there is one to end at.
    if line.chars().nth(most).is_some_and(|next| next != ' ')
        && let Some(space) = cut.rfind(' ')
    {
        cut.truncate(space);
    }
    cut.push_str(ellipsis);
    cut
}

/// The item an event is of, if it is of one.
fn item_of(event: &Event) -> Option<&str> {
    match event {
        Event::ItemStarted(started) => Some(started.item_id.as_str()),
        Event::ItemDelta(delta) => Some(delta.item_id.as_str()),
        Event::ItemCompleted(completed) => completed.item.id(),
        Event::ItemResync(item) | Event::ItemReset(item) => Some(item.item_id.as_str()),
        _ => None,
    }
}

/// A prompt's delivery as the protocol says it: none for a new turn.
fn wire(delivery: Delivery) -> Option<ItemDelivery> {
    match delivery {
        Delivery::Prompt => None,
        Delivery::Steer => Some(ItemDelivery::Steer),
        Delivery::Queue => Some(ItemDelivery::Queue),
    }
}

fn new_session(
    workspace: String,
    cwd: Option<String>,
    settings: SessionSettings,
    ephemeral: bool,
) -> Op {
    Op::SessionNew(SessionNewParams {
        cwd,
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
