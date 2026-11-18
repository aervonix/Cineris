//! The first run in the application (since step 25;
//! `docs/architecture/08-configuration.md`, "First run";
//! `docs/architecture/09-tui.md`, "The first run").
//!
//! **The order:** the provider and its key, what other tools left, the
//! mode; then the configuration file is written ([`Setup::write_config`]),
//! the core reads it again (`config/reload`), and the session opens, made
//! with what was chosen; its trust question is its own (screen 2c); then
//! the commands the project's files name (`project/detect`), which join
//! the profile as entries the person wrote (`project/update`'s `add`).
//!
//! **Each step takes the input box's place** under the set-up card, which
//! stays in the live region until the session opens and is never
//! committed; a step answered commits one dim line of what it did. Every
//! step holds the dialog's rule of keys ([`Readiness`]): no key until
//! 300 ms after its first frame, and only a quiet second after input it did
//! not take; a key held from the step before, a repeat and a paste never
//! answer one. `s` skips a step, `x` the whole set-up: what was done stays
//! done, the rest take their defaults. `Ctrl+C` twice leaves the program,
//! nothing written but what a finished step wrote.
//!
//! **Nothing found is started, trusted or put in the prompt without a
//! yes:** the imports' rows start unpicked, a skip picks nothing, and the
//! commands' rows start unpicked too.

use std::sync::Arc;
use std::time::Instant;

use cineris_protocol::methods::{
    ConfigReloadParams, Op, OpResult, ProjectDetectParams, ProjectUpdateParams,
};
use cineris_protocol::project::{DetectedCommand, EntryKind, NewCommand, NewEntry};
use cineris_protocol::{ErrorData, SessionId};
use cineris_term::{Event as TermEvent, Key, KeyCode, Line, Mods};

use super::{App, Pending};
use crate::app::ready::{REPEAT, Readiness};
use crate::app::setup::{
    Answer, Chosen, Found, Imported, Job, KeyStore, McpServer, Providers, Setup, Worker,
};
use crate::keys::{Action, Context};
use crate::secret::{KEYS as KEY_KEYS, KeyEdit, KeyInput, key_box};
use crate::view::setup::{Mark, Row, Step as StepView, card, step};
use crate::view::{Hint, one_line};

/// What an operation the first run sent is for.
#[derive(Debug, Clone)]
pub(super) enum Sent {
    /// The configuration read again after the file was written.
    Reload,
    /// The project's commands, for the last step.
    Detect(SessionId),
    /// The commands picked, added to the profile.
    Add,
}

/// The first run, while it runs.
#[derive(Debug)]
pub(super) struct FirstRun {
    worker: Worker,
    step: Step,
    ready: Readiness,
    /// When a step last took a key that answered it: the next is ready no
    /// sooner than [`REPEAT`] after.
    taken: Option<Instant>,
    chosen: Chosen,
    providers: Option<Providers>,
    /// The configuration file, as the person reads its path, once written.
    written: Option<String>,
    /// `x` was pressed: the steps left take their defaults.
    skipped: bool,
    /// The configuration was read again: the session may open.
    released: bool,
    /// A frame has drawn the step's keys, which it shows once it is ready,
    /// as a dialog shows its answers: it takes keys only then.
    keys_drawn: bool,
}

#[derive(Debug)]
enum Step {
    /// The providers, and where each one's key comes from.
    Provider { selected: usize },
    /// A key typed for a provider; `storing` once it is with the worker.
    Key {
        provider: String,
        input: KeyInput,
        storing: bool,
        error: Option<String>,
    },
    /// What other tools left.
    Imports(Imports),
    /// The mode.
    Mode { selected: usize },
    /// The file being written, the configuration read again, the session
    /// opening, and its trust question: nothing of the first run is shown.
    Waiting,
    /// The session is open: its commands are asked for once no dialog is
    /// up.
    Opened,
    /// The commands found.
    Commands(Commands),
}

#[derive(Debug, Default)]
struct Imports {
    found: Option<Found>,
    selected: usize,
    instructions: bool,
    skills: bool,
    files: Vec<File>,
}

#[derive(Debug)]
struct File {
    path: String,
    from: String,
    open: bool,
    reading: bool,
    servers: Option<Result<Vec<(McpServer, bool)>, String>>,
}

/// A row of the imports one can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Instructions,
    Skills,
    File(usize),
    Server(usize, usize),
}

#[derive(Debug, Default)]
struct Commands {
    rows: Vec<(DetectedCommand, bool)>,
    selected: usize,
    editing: Option<String>,
    trusted: bool,
}

/// The levels the first run offers, in the ladder's order, each with what
/// it means as 06's table has it. `unrestricted` is not one: 06 has it
/// chosen explicitly, and `/mode` and the configuration still reach it.
const LEVELS: [(&str, &str); 3] = [
    (
        "read-only",
        "reads, and runs only commands known to read; asks before any other, and never writes",
    ),
    (
        "supervised",
        "reads and edits files in this project without asking; asks before a command that is \
         not on a safe list, and before anything outside the project",
    ),
    (
        "autonomous",
        "runs commands in the sandbox without asking; asks before leaving it, using the \
         network or writing outside the project",
    ),
];

impl FirstRun {
    /// The first run on `setup`; `None` when its worker could not start.
    pub(super) fn start(setup: Arc<dyn Setup>) -> Option<Self> {
        let worker = Worker::start(setup)?;
        worker.ask(Job::Providers);
        Some(Self {
            worker,
            step: Step::Provider { selected: 0 },
            ready: Readiness::default(),
            taken: None,
            chosen: Chosen::default(),
            providers: None,
            written: None,
            skipped: false,
            released: false,
            keys_drawn: false,
        })
    }

    /// Whether the session waits for the steps before it.
    pub(super) fn holds_open(&self) -> bool {
        !self.released
    }

    /// Input the step did not take: it is ready a quiet second after, and
    /// its keys are hidden until then.
    fn typed(&mut self, at: Instant) {
        self.ready.typed(at);
        self.keys_drawn = false;
    }

    /// Whether the step takes a key at `at`: it is ready, and a frame has
    /// drawn its keys.
    fn takes_keys(&self, at: Instant) -> bool {
        self.keys_drawn && self.ready.ready(at)
    }

    /// Whether a step is shown, in the input box's place.
    fn shows(&self) -> bool {
        !matches!(self.step, Step::Waiting | Step::Opened)
    }

    /// Whether the set-up card is shown: until the session opens.
    fn card(&self) -> bool {
        matches!(
            self.step,
            Step::Provider { .. } | Step::Key { .. } | Step::Imports(_) | Step::Mode { .. }
        )
    }

    /// A new step, ready only after its grace and a key's repeat window.
    fn to(&mut self, step: Step) {
        self.step = step;
        self.keys_drawn = false;
        self.ready = Readiness::default();
        if let Some(taken) = self.taken {
            self.ready.not_before(taken + REPEAT);
        }
    }

    fn levels(&self) -> Vec<(&'static str, &'static str)> {
        let sandbox = self.providers.as_ref().is_some_and(|found| found.sandbox);
        LEVELS
            .iter()
            .copied()
            .filter(|(level, _)| *level != "autonomous" || sandbox)
            .collect()
    }
}

/// How a provider's row gets its key.
#[derive(Debug, Clone, PartialEq, Eq)]
enum How {
    Variable(String),
    Stored,
    Type,
}

fn provider_rows(providers: &Providers) -> Vec<(String, How)> {
    let mut rows = Vec::new();
    for provider in &providers.providers {
        if let Some(variable) = &provider.variable {
            rows.push((provider.name.clone(), How::Variable(variable.clone())));
        }
    }
    for provider in &providers.providers {
        if provider.stored {
            rows.push((provider.name.clone(), How::Stored));
        }
    }
    for provider in &providers.providers {
        rows.push((provider.name.clone(), How::Type));
    }
    rows
}

impl Imports {
    fn picks(&self) -> Vec<Pick> {
        let mut picks = vec![Pick::Instructions, Pick::Skills];
        for (index, file) in self.files.iter().enumerate() {
            picks.push(Pick::File(index));
            if file.open
                && let Some(Ok(servers)) = &file.servers
            {
                picks.extend((0..servers.len()).map(|server| Pick::Server(index, server)));
            }
        }
        picks
    }
}

impl App {
    /// Whether the first run holds the session back: it opens once the
    /// configuration file is written and read again.
    pub(super) fn first_run_holds_open(&self) -> bool {
        self.first_run.as_ref().is_some_and(FirstRun::holds_open)
    }

    /// A frame was drawn at `at`: a step shown starts its grace.
    pub(super) fn first_run_drawn(&mut self, at: Instant) {
        if let Some(run) = self.first_run.as_mut()
            && run.shows()
        {
            run.ready.drawn(at);
        }
    }

    /// Input the first run did not take: its step waits a quiet second.
    pub(super) fn first_run_typed(&mut self, at: Instant) {
        if let Some(run) = self.first_run.as_mut() {
            run.typed(at);
        }
    }

    /// When the step shown is to be drawn with its keys: the end of its
    /// grace, until a frame has drawn them.
    pub(super) fn first_run_deadline(&self) -> Option<Instant> {
        let run = self.first_run.as_ref()?;
        if !run.shows() || run.keys_drawn {
            return None;
        }
        run.ready.ready_at()
    }

    /// The terminal's waker, for the worker.
    pub(super) fn first_run_waker(&mut self, waker: &cineris_term::Waker) {
        if let Some(run) = &self.first_run {
            run.worker.set_waker(waker.clone());
        }
    }

    /// What the worker answered, taken: when the loop is woken, and before
    /// a frame.
    /// Says whether one came, which is a change to draw.
    pub(super) fn first_run_answers(&mut self) -> bool {
        let mut any = false;
        while let Some(answer) = self.first_run.as_ref().and_then(|run| run.worker.answer()) {
            self.first_run_answer(answer);
            any = true;
        }
        self.first_run_next();
        any
    }

    /// Waits at most `limit` for each answer the worker owes, taking each
    /// as the loop would, for tests, which have no loop to wake. Says
    /// whether it owes none now: an answer a frame took already is owed no
    /// more.
    #[cfg(any(test, feature = "test-kit"))]
    pub fn wait_for_setup(&mut self, limit: std::time::Duration) -> bool {
        while let Some(answer) = self
            .first_run
            .as_ref()
            .and_then(|run| run.worker.wait(limit))
        {
            self.first_run_answer(answer);
            self.first_run_next();
        }
        self.first_run
            .as_ref()
            .is_none_or(|run| run.worker.settled())
    }

    /// Whether the first run is still running, for tests.
    #[must_use]
    pub fn in_first_run(&self) -> bool {
        self.first_run.is_some()
    }

    fn first_run_answer(&mut self, answer: Answer) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        match answer {
            Answer::Providers(Ok(providers)) => {
                let selected = provider_rows(&providers)
                    .iter()
                    .position(|(_, how)| *how != How::Type)
                    .unwrap_or(0);
                run.providers = Some(providers);
                if let Step::Provider { selected: at } = &mut run.step {
                    *at = selected;
                }
            }
            Answer::Providers(Err(why)) => {
                run.providers = Some(Providers::default());
                self.say(&format!(
                    "The providers could not be listed ({}): the key is skipped. \
                     `cineris auth login <provider>` stores one later.",
                    one_line(&why)
                ));
                if let Some(run) = self.first_run.as_mut()
                    && matches!(run.step, Step::Provider { .. })
                {
                    run.worker.ask(Job::Discover);
                    run.to(Step::Imports(Imports::default()));
                }
            }
            Answer::Stored(provider, Ok(())) => {
                let kept = run
                    .providers
                    .as_ref()
                    .map_or_else(String::new, |found| found.store.kept());
                run.chosen.provider = Some(provider.clone());
                run.worker.ask(Job::Discover);
                run.to(Step::Imports(Imports::default()));
                self.say(&format!(
                    "Provider: {}, its key stored {kept}.",
                    one_line(&provider)
                ));
            }
            Answer::Stored(_, Err(why)) => {
                if let Step::Key { storing, error, .. } = &mut run.step {
                    *storing = false;
                    *error = Some(format!(
                        "The key could not be stored: {}. Try again, or esc to choose another \
                         way.",
                        one_line(&why)
                    ));
                }
            }
            Answer::Found(found) => {
                if let Step::Imports(imports) = &mut run.step {
                    imports.files = found
                        .user_mcp
                        .iter()
                        .map(|file| File {
                            path: file.path.clone(),
                            from: file.from.clone(),
                            open: false,
                            reading: false,
                            servers: None,
                        })
                        .collect();
                    imports.found = Some(found);
                }
            }
            Answer::Servers(path, read) => {
                if let Step::Imports(imports) = &mut run.step
                    && let Some(file) = imports.files.iter_mut().find(|file| file.path == path)
                {
                    file.reading = false;
                    file.open = true;
                    file.servers = Some(read.map(|servers| {
                        servers.into_iter().map(|server| (server, false)).collect()
                    }));
                }
            }
            Answer::Written(written) => {
                let mode = run
                    .chosen
                    .mode
                    .clone()
                    .unwrap_or_else(|| "supervised".to_owned());
                let said = match &written {
                    Ok(path) => format!("Mode: {mode}. Written to {}.", one_line(path)),
                    Err(why) => format!(
                        "Mode: {mode}, for this session. The settings were not written: {}",
                        one_line(why)
                    ),
                };
                run.written = Some(written.clone().unwrap_or_default());
                self.say(&said);
                // The core reads the file again, so that the session is made
                // with what was chosen; it opens once that is done.
                let ticket = self
                    .link
                    .send(Op::ConfigReload(ConfigReloadParams::default()));
                self.pending.insert(ticket, Pending::FirstRun(Sent::Reload));
            }
        }
    }

    /// An operation the first run sent was answered.
    pub(super) fn first_run_done(&mut self, sent: Sent, result: Result<OpResult, ErrorData>) {
        match (sent, result) {
            (Sent::Reload, result) => {
                if let Err(error) = result {
                    self.error(&error);
                }
                if let Some(run) = self.first_run.as_mut() {
                    run.step = Step::Opened;
                    run.released = true;
                    // The session made with what was chosen: its header is
                    // the one card from now on.
                    if let Some(open) = self.open.take() {
                        self.send_open(open);
                    }
                }
            }
            (Sent::Detect(session), Ok(OpResult::ProjectDetect(found))) => {
                if self.session.as_ref().map(|state| &state.session_id) != Some(&session) {
                    self.first_run_end();
                    return;
                }
                if found.commands.is_empty() {
                    self.say("Commands: none found in this project's files.");
                    self.first_run_end();
                    return;
                }
                let trusted = self.session.as_ref().is_some_and(|state| {
                    state
                        .trust
                        .as_ref()
                        .is_some_and(|trust| matches!(trust.as_str(), "trusted" | "for_the_run"))
                });
                if let Some(run) = self.first_run.as_mut() {
                    run.to(Step::Commands(Commands {
                        rows: found
                            .commands
                            .into_iter()
                            .map(|command| (command, false))
                            .collect(),
                        selected: 0,
                        editing: None,
                        trusted,
                    }));
                }
            }
            (Sent::Detect(_), result) => {
                if let Err(error) = result {
                    self.error(&error);
                }
                self.first_run_end();
            }
            (Sent::Add, Ok(_)) => self.first_run_end(),
            (Sent::Add, Err(error)) => {
                self.error(&error);
                self.first_run_end();
            }
        }
    }

    /// What follows by itself: the commands asked for once the session is
    /// open and no dialog is up.
    fn first_run_next(&mut self) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        if !matches!(run.step, Step::Opened) || !self.dialogs.is_empty() || self.opening {
            return;
        }
        let Some(session) = self.session.as_ref().map(|state| state.session_id.clone()) else {
            return;
        };
        if run.skipped {
            self.first_run_end();
            return;
        }
        run.step = Step::Waiting;
        let trusted = self.session.as_ref().is_some_and(|state| {
            state
                .trust
                .as_ref()
                .is_some_and(|trust| matches!(trust.as_str(), "trusted" | "for_the_run"))
        });
        self.say(&format!(
            "Project: {} {}.",
            one_line(&self.folder),
            if trusted { "trusted" } else { "not trusted" }
        ));
        let ticket = self.link.send(Op::ProjectDetect(ProjectDetectParams {
            session_id: session.clone(),
        }));
        self.pending
            .insert(ticket, Pending::FirstRun(Sent::Detect(session)));
    }

    /// The first run ends: its last line names what does each step again.
    fn first_run_end(&mut self) {
        let Some(run) = self.first_run.take() else {
            return;
        };
        let file = run.written.filter(|path| !path.is_empty()).map_or_else(
            || "the configuration file".to_owned(),
            |path| one_line(&path),
        );
        self.say(&format!(
            "{} Each step can be done again: `cineris auth login <provider>` stores a key, \
             `/mode` sets the mode, `/project` shows this project's commands, and {file} holds \
             the settings.",
            if run.skipped {
                "Set-up skipped."
            } else {
                "Set-up done."
            }
        ));
    }

    /// `x`: the steps left take their defaults. Before the session, the
    /// file is written with what was done; after it, the first run ends.
    fn first_run_skip_all(&mut self, at: Instant) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        run.taken = Some(at);
        run.skipped = true;
        if matches!(run.step, Step::Commands(_)) {
            self.first_run_end();
            return;
        }
        let unsaid = run.chosen.provider.is_none()
            && matches!(run.step, Step::Provider { .. } | Step::Key { .. });
        run.chosen.imports = None;
        run.chosen.mode = None;
        if unsaid {
            self.say("Provider: skipped.");
        }
        self.first_run_write();
    }

    fn first_run_write(&mut self) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        run.worker.ask(Job::Write(run.chosen.clone()));
        run.step = Step::Waiting;
    }

    /// A terminal event while the first run shows a step: `None` when it
    /// does not, and the event goes where it would otherwise.
    pub(super) fn first_run_event(&mut self, event: &TermEvent, at: Instant) -> Option<bool> {
        let run = self.first_run.as_mut()?;
        if !run.shows() {
            return None;
        }
        match event {
            TermEvent::Key(key) => {
                // The global keys: `Ctrl+C` twice leaves, `Ctrl+L` draws
                // again, `Ctrl+Z` stops.
                let global = self.input.keymap().lookup(&[Context::Global], *key);
                match global {
                    Some(Action::Interrupt) => {
                        let outcome = self.input.interrupt_hidden();
                        self.outcome(outcome, at);
                        return Some(true);
                    }
                    Some(action @ (Action::Redraw | Action::Suspend)) => {
                        let outcome = self.input.act(action);
                        self.outcome(outcome, at);
                        return Some(true);
                    }
                    _ => {}
                }
                self.input.disarm_quit();
                let Some(run) = self.first_run.as_mut() else {
                    return Some(true);
                };
                // Typing into the key's input is typing, not an answer: it
                // goes in whenever the step is shown. `Enter` and `Esc`
                // answer it, and wait for its keys.
                if let Step::Key {
                    input,
                    storing,
                    error,
                    ..
                } = &mut run.step
                    && !matches!(key.code, KeyCode::Enter | KeyCode::Esc)
                {
                    if !*storing && input.key(*key) == KeyEdit::Changed {
                        *error = None;
                    }
                    return Some(true);
                }
                if !run.takes_keys(at) {
                    run.typed(at);
                    return Some(true);
                }
                self.first_run_key(*key, at);
                Some(true)
            }
            // A key held down never answers a step.
            TermEvent::Repeat(_) => {
                run.typed(at);
                Some(true)
            }
            // A paste is the key's, or nobody's: it never answers a step.
            TermEvent::Paste(text) => {
                match &mut run.step {
                    Step::Key {
                        input,
                        storing,
                        error,
                        ..
                    } if !*storing => {
                        if input.paste(text) == KeyEdit::Changed {
                            *error = None;
                        }
                    }
                    _ => run.typed(at),
                }
                Some(true)
            }
            _ => None,
        }
    }

    fn first_run_key(&mut self, key: Key, at: Instant) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        let plain = key.mods.is_empty() || key.mods == Mods::SHIFT;
        let letter = match key.code {
            KeyCode::Char(character) if plain => Some(character),
            _ => None,
        };
        let up = key.code == KeyCode::Up && key.mods.is_empty();
        let down = key.code == KeyCode::Down && key.mods.is_empty();
        let enter = key.code == KeyCode::Enter && key.mods.is_empty();
        let space = letter == Some(' ');
        let editing = matches!(&run.step, Step::Commands(commands) if commands.editing.is_some());
        let typing_key = matches!(run.step, Step::Key { .. });
        let levels = run.levels();
        if letter == Some('x') && !typing_key && !editing {
            self.first_run_skip_all(at);
            return;
        }
        match &mut run.step {
            Step::Provider { selected } => {
                let rows = run
                    .providers
                    .as_ref()
                    .map(provider_rows)
                    .unwrap_or_default();
                if up {
                    *selected = selected.saturating_sub(1);
                } else if down {
                    *selected = (*selected + 1).min(rows.len().saturating_sub(1));
                } else if letter == Some('s') {
                    run.taken = Some(at);
                    run.worker.ask(Job::Discover);
                    run.to(Step::Imports(Imports::default()));
                    self.say("Provider: skipped. `cineris auth login <provider>` stores a key.");
                } else if enter && let Some((provider, how)) = rows.get(*selected).cloned() {
                    run.taken = Some(at);
                    match how {
                        How::Type => run.to(Step::Key {
                            provider,
                            input: KeyInput::new(),
                            storing: false,
                            error: None,
                        }),
                        How::Variable(variable) => {
                            run.chosen.provider = Some(provider.clone());
                            run.worker.ask(Job::Discover);
                            run.to(Step::Imports(Imports::default()));
                            self.say(&format!(
                                "Provider: {}, its key from {}.",
                                one_line(&provider),
                                one_line(&variable)
                            ));
                        }
                        How::Stored => {
                            run.chosen.provider = Some(provider.clone());
                            run.worker.ask(Job::Discover);
                            run.to(Step::Imports(Imports::default()));
                            self.say(&format!(
                                "Provider: {}, its stored key.",
                                one_line(&provider)
                            ));
                        }
                    }
                }
            }
            Step::Key {
                provider,
                input,
                storing,
                error,
            } => {
                if *storing {
                    return;
                }
                match input.key(key) {
                    KeyEdit::Submit => {
                        let ascii = run
                            .providers
                            .as_ref()
                            .is_some_and(|found| found.store.ascii_only());
                        match input.take(ascii) {
                            Ok(secret) => {
                                *storing = true;
                                *error = None;
                                run.taken = Some(at);
                                run.worker.ask(Job::StoreKey(provider.clone(), secret));
                            }
                            Err(why) => *error = Some(why.to_owned()),
                        }
                    }
                    KeyEdit::Cancel => {
                        input.clear();
                        run.taken = Some(at);
                        run.to(Step::Provider { selected: 0 });
                    }
                    KeyEdit::Changed => *error = None,
                    KeyEdit::Refused | KeyEdit::Nothing => {}
                }
            }
            Step::Imports(imports) => {
                let picks = imports.picks();
                if up {
                    imports.selected = imports.selected.saturating_sub(1);
                } else if down {
                    imports.selected = (imports.selected + 1).min(picks.len().saturating_sub(1));
                } else if space {
                    match picks.get(imports.selected) {
                        Some(Pick::Instructions) => imports.instructions = !imports.instructions,
                        Some(Pick::Skills) => imports.skills = !imports.skills,
                        Some(Pick::File(index)) => {
                            if let Some(file) = imports.files.get_mut(*index) {
                                if file.servers.is_some() {
                                    file.open = !file.open;
                                } else if !file.reading {
                                    // Reading it is the import the person
                                    // asked for: on the worker.
                                    file.reading = true;
                                    run.worker.ask(Job::Read(file.path.clone()));
                                }
                            }
                        }
                        Some(Pick::Server(file, server)) => {
                            if let Some(Some(Ok(servers))) =
                                imports.files.get_mut(*file).map(|file| &mut file.servers)
                                && let Some((server, picked)) = servers.get_mut(*server)
                                && server.refused.is_none()
                            {
                                *picked = !*picked;
                            }
                        }
                        None => {}
                    }
                } else if enter || letter == Some('s') {
                    run.taken = Some(at);
                    let imported = enter.then(|| Imported {
                        instructions: imports.instructions,
                        skills: imports.skills,
                        servers: imports
                            .files
                            .iter()
                            .flat_map(|file| {
                                let picked: Vec<(String, String)> = match &file.servers {
                                    Some(Ok(servers)) => servers
                                        .iter()
                                        // A server refused is never picked
                                        // (`space` passes it over).
                                        .filter(|(_, picked)| *picked)
                                        .map(|(server, _)| (file.path.clone(), server.name.clone()))
                                        .collect(),
                                    _ => Vec::new(),
                                };
                                picked
                            })
                            .collect(),
                    });
                    let said = imports_said(imported.as_ref());
                    run.chosen.imports = imported;
                    run.to(Step::Mode { selected: 1 });
                    self.say(&said);
                }
            }
            Step::Mode { selected } => {
                if up {
                    *selected = selected.saturating_sub(1);
                } else if down {
                    *selected = (*selected + 1).min(levels.len().saturating_sub(1));
                } else if enter || letter == Some('s') {
                    run.taken = Some(at);
                    run.chosen.mode = if enter {
                        levels.get(*selected).map(|(level, _)| (*level).to_owned())
                    } else {
                        None
                    };
                    self.first_run_write();
                }
            }
            Step::Commands(commands) => {
                if let Some(text) = &mut commands.editing {
                    match key.code {
                        KeyCode::Enter if key.mods.is_empty() => {
                            let edited = text.trim().to_owned();
                            if let Some((row, picked)) = commands.rows.get_mut(commands.selected)
                                && !edited.is_empty()
                            {
                                row.command = edited;
                                *picked = true;
                            }
                            commands.editing = None;
                        }
                        KeyCode::Esc => commands.editing = None,
                        KeyCode::Backspace => {
                            text.pop();
                        }
                        KeyCode::Char('u') if key.mods == Mods::CTRL => text.clear(),
                        KeyCode::Char(character) if plain && !character.is_control() => {
                            text.push(character);
                        }
                        _ => {}
                    }
                    return;
                }
                if up {
                    commands.selected = commands.selected.saturating_sub(1);
                } else if down {
                    commands.selected =
                        (commands.selected + 1).min(commands.rows.len().saturating_sub(1));
                } else if space {
                    if let Some((_, picked)) = commands.rows.get_mut(commands.selected) {
                        *picked = !*picked;
                    }
                } else if letter == Some('e') {
                    commands.editing = commands
                        .rows
                        .get(commands.selected)
                        .map(|(row, _)| row.command.clone());
                } else if enter || letter == Some('s') {
                    run.taken = Some(at);
                    let picked: Vec<DetectedCommand> = if enter {
                        commands
                            .rows
                            .iter()
                            .filter(|(_, picked)| *picked)
                            .map(|(command, _)| command.clone())
                            .collect()
                    } else {
                        Vec::new()
                    };
                    self.first_run_add(picked);
                }
            }
            Step::Waiting | Step::Opened => {}
        }
    }

    /// The commands picked join the profile as entries the person wrote.
    fn first_run_add(&mut self, picked: Vec<DetectedCommand>) {
        let Some(session) = self.session.as_ref().map(|state| state.session_id.clone()) else {
            self.first_run_end();
            return;
        };
        if picked.is_empty() {
            self.say("Commands: none kept.");
            self.first_run_end();
            return;
        }
        let names: Vec<String> = picked
            .iter()
            .map(|command| one_line(&command.name))
            .collect();
        self.say(&format!(
            "Commands: {} kept in this project's profile.",
            and_list(&names)
        ));
        let add = picked
            .into_iter()
            .map(|command| NewEntry {
                kind: EntryKind::Command,
                text: format!(
                    "{}: {}, found in {} by the set-up",
                    command.name, command.command, command.source
                ),
                command: Some(NewCommand {
                    purpose: command.purpose,
                    command: command.command,
                    cwd: None,
                    read_only: false,
                }),
                check: None,
            })
            .collect();
        let ticket = self.link.send(Op::ProjectUpdate(ProjectUpdateParams {
            session_id: Some(session),
            workspace: None,
            changes: Vec::new(),
            add,
        }));
        self.pending.insert(ticket, Pending::FirstRun(Sent::Add));
        if let Some(run) = self.first_run.as_mut() {
            run.step = Step::Waiting;
        }
    }

    /// The first run's rows in the input box's place, with the set-up card
    /// above them until the session opens, and its keys under them. `None`
    /// when it shows no step.
    pub(super) fn first_run_bottom(&mut self, now: Instant, width: usize) -> Option<Vec<Line>> {
        let run = self.first_run.as_mut()?;
        if !run.shows() {
            return None;
        }
        // Its keys once it is ready, as a dialog's answers.
        let ready = run.ready.ready(now);
        run.keys_drawn = ready;
        let run = self.first_run.as_ref()?;
        let look = &self.look;
        let mut lines = Vec::new();
        if !self.kept.is_empty() {
            lines.push(Line::new());
        }
        if run.card() {
            lines.extend(card(&self.version, width, look));
            lines.push(Line::new());
        }
        let (view, keys) = self.first_run_view(run);
        lines.extend(step(&view, width, look));
        if let Step::Key {
            input,
            error,
            storing,
            ..
        } = &run.step
        {
            lines.push(Line::new());
            lines.extend(key_box(input, width, look));
            let said = if *storing {
                Some(format!("storing{}", look.glyphs.ellipsis))
            } else {
                error.clone().or_else(|| input.refusal().map(str::to_owned))
            };
            if let Some(said) = said {
                lines.extend(crate::secret::sentence(&said, width, look));
            }
        } else {
            lines.push(Line::new());
        }
        let hint = if self.input.quit_armed() {
            Hint::QuitArmed
        } else if ready {
            Hint::Keys(keys)
        } else {
            Hint::Empty
        };
        lines.push(self.hint_row(hint, now));
        Some(lines)
    }

    /// The step shown, and its keys.
    fn first_run_view(&self, run: &FirstRun) -> (StepView, Vec<String>) {
        let reading = format!("reading{}", self.look.glyphs.ellipsis);
        let skip = vec![
            "s skips this step".to_owned(),
            "x skips the set-up".to_owned(),
        ];
        match &run.step {
            Step::Provider { selected } => {
                let Some(providers) = &run.providers else {
                    return (
                        StepView {
                            title: "Provider".to_owned(),
                            number: 1,
                            sentence: reading,
                            ..StepView::default()
                        },
                        skip,
                    );
                };
                let rows = provider_rows(providers);
                let found = providers.providers.iter().find_map(|provider| {
                    provider
                        .variable
                        .as_ref()
                        .map(|variable| (provider.name.clone(), variable.clone()))
                });
                let kept = providers.store.kept();
                let sentence = match found {
                    Some((name, variable)) => format!(
                        "A key for {name} is in the environment, in {variable}. Use it from \
                         there, or store a key: it is kept {kept}, and never in a file of \
                         settings."
                    ),
                    None => format!(
                        "Choose the provider whose key to store. It is kept {kept}, and never \
                         in a file of settings."
                    ),
                };
                let view = StepView {
                    title: "Provider".to_owned(),
                    number: 1,
                    sentence,
                    rows: rows
                        .iter()
                        .map(|(name, how)| {
                            let detail = match how {
                                How::Variable(variable) => {
                                    format!("its key from {variable}, as it is set")
                                }
                                How::Stored => "its stored key".to_owned(),
                                How::Type => "store a key".to_owned(),
                            };
                            Row::new(0, Mark::None, name.clone(), vec![detail])
                        })
                        .collect(),
                    selected: Some(*selected),
                    note: match &providers.store {
                        KeyStore::Unavailable(why) => Some(format!(
                            "No key can be stored here: {why}. Set the provider's variable \
                             instead, or skip this step."
                        )),
                        _ => None,
                    },
                };
                let mut keys = vec!["enter chooses".to_owned()];
                keys.extend(skip);
                (view, keys)
            }
            Step::Key { provider, .. } => {
                let kept = run
                    .providers
                    .as_ref()
                    .map_or_else(String::new, |found| found.store.kept());
                (
                    StepView {
                        title: "Provider".to_owned(),
                        number: 1,
                        sentence: format!(
                            "Paste or type the key for {provider}. Nothing of it is shown. It is \
                             stored {kept}, and is never written to a file of settings, the log \
                             or the prompt history."
                        ),
                        ..StepView::default()
                    },
                    KEY_KEYS.iter().map(|key| (*key).to_owned()).collect(),
                )
            }
            Step::Imports(imports) => {
                let mut view = StepView {
                    title: "Imports".to_owned(),
                    number: 2,
                    sentence: "Other tools left these. None is read, started or put in the \
                               prompt unless you pick it, and what is left unpicked stays off. \
                               What you pick holds in every project."
                        .to_owned(),
                    ..StepView::default()
                };
                let picks = imports.picks();
                let selected = picks.get(imports.selected).copied();
                let mut rows = vec![Row::heading("Instructions")];
                let mark = |at: Pick, rows: &Vec<Row>| (selected == Some(at)).then_some(rows.len());
                view.selected = mark(Pick::Instructions, &rows);
                rows.push(Row::new(
                    0,
                    Mark::Picked(imports.instructions),
                    "AGENTS.md, CLAUDE.md",
                    vec![
                        "read as each project's instructions; off unless picked \
                         (compat.instructions)"
                            .to_owned(),
                    ],
                ));
                rows.push(Row::heading("Skills"));
                if let Some(at) = mark(Pick::Skills, &rows) {
                    view.selected = Some(at);
                }
                rows.push(Row::new(
                    0,
                    Mark::Picked(imports.skills),
                    ".claude/skills, .agents/skills",
                    vec![
                        "yours and each project's, once this version reads skills; off unless \
                         picked (compat.skills)"
                            .to_owned(),
                    ],
                ));
                rows.push(Row::heading("MCP servers"));
                let Some(found) = &imports.found else {
                    view.rows = rows;
                    view.note = Some(reading);
                    return (view, skip);
                };
                for (index, file) in imports.files.iter().enumerate() {
                    if let Some(at) = mark(Pick::File(index), &rows) {
                        view.selected = Some(at);
                    }
                    let detail = if file.reading {
                        format!("{}'s: {reading}", file.from)
                    } else if file.servers.is_none() {
                        format!("{}'s: space reads it", file.from)
                    } else {
                        format!("{}'s", file.from)
                    };
                    rows.push(Row::new(
                        0,
                        Mark::Folded(file.open),
                        file.path.clone(),
                        vec![detail],
                    ));
                    if !file.open {
                        continue;
                    }
                    match &file.servers {
                        Some(Ok(servers)) if servers.is_empty() => rows.push(Row::new(
                            1,
                            Mark::Blank,
                            "",
                            vec!["no MCP server is defined in it".to_owned()],
                        )),
                        Some(Ok(servers)) => {
                            for (number, (server, picked)) in servers.iter().enumerate() {
                                if let Some(at) = mark(Pick::Server(index, number), &rows) {
                                    view.selected = Some(at);
                                }
                                let mut detail = vec![server.shown.clone()];
                                for (header, variable) in &server.headers {
                                    detail.push(format!(
                                        "its {header} header comes from {variable}, which you set"
                                    ));
                                }
                                if !server.env.is_empty() {
                                    detail.push(format!(
                                        "takes {} from your environment",
                                        server.env.join(", ")
                                    ));
                                }
                                let mark = match &server.refused {
                                    Some(why) => {
                                        detail.push(format!("{why}: not imported"));
                                        Mark::Blank
                                    }
                                    None => Mark::Picked(*picked),
                                };
                                rows.push(Row::new(1, mark, server.name.clone(), detail));
                            }
                        }
                        Some(Err(why)) => rows.push(Row::new(
                            1,
                            Mark::Blank,
                            "",
                            vec![format!("it could not be read: {why}")],
                        )),
                        None => {}
                    }
                }
                if !found.project_mcp.is_empty() {
                    rows.push(Row::new(
                        0,
                        Mark::Blank,
                        found.project_mcp.join(", "),
                        vec![
                            "this project's: offered in its sessions, after trust, never \
                             imported"
                                .to_owned(),
                        ],
                    ));
                }
                if imports.files.is_empty() && found.project_mcp.is_empty() {
                    rows.push(Row::new(
                        0,
                        Mark::Blank,
                        "",
                        vec!["no other client's file was found".to_owned()],
                    ));
                }
                view.rows = rows;
                let keys = match selected {
                    Some(Pick::File(_)) => "space reads or folds it",
                    _ => "space picks",
                };
                let mut all = vec![keys.to_owned(), "enter keeps what is picked".to_owned()];
                all.extend(skip);
                (view, all)
            }
            Step::Mode { selected } => {
                let levels = run.levels();
                (
                    StepView {
                        title: "Mode".to_owned(),
                        number: 3,
                        sentence: "How much the agent does before it asks you. /mode changes it \
                                   at any time."
                            .to_owned(),
                        rows: levels
                            .iter()
                            .map(|(level, means)| {
                                Row::new(0, Mark::None, *level, vec![(*means).to_owned()])
                            })
                            .collect(),
                        selected: Some(*selected),
                        note: None,
                    },
                    vec![
                        "enter chooses".to_owned(),
                        "s keeps supervised".to_owned(),
                        "x skips the set-up".to_owned(),
                    ],
                )
            }
            Step::Commands(commands) => {
                let mut sources: Vec<String> = Vec::new();
                for (command, _) in &commands.rows {
                    if !sources.contains(&command.source) {
                        sources.push(command.source.clone());
                    }
                }
                let sources: Vec<String> = sources.iter().map(|source| one_line(source)).collect();
                let sentence = if commands.trusted {
                    format!(
                        "Found in {}. Each one picked joins the project's profile as an entry \
                         you wrote: it runs without asking, and the checks at a turn's end use \
                         it.",
                        and_list(&sources)
                    )
                } else {
                    format!(
                        "Found in {}. Each one picked joins the project's profile as an entry \
                         you wrote. The project is not trusted, so a command still asks until \
                         it is.",
                        and_list(&sources)
                    )
                };
                let rows = commands
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(index, (command, picked))| {
                        let shown = match (&commands.editing, index == commands.selected) {
                            (Some(text), true) => format!("{text}_"),
                            _ => command.command.clone(),
                        };
                        let mut detail = vec![shown];
                        if let Some(runs) = &command.runs {
                            detail.push(format!("runs {runs}"));
                        }
                        Row::new(0, Mark::Picked(*picked), command.name.clone(), detail)
                    })
                    .collect();
                let keys = if commands.editing.is_some() {
                    vec![
                        "enter keeps the edit".to_owned(),
                        "esc leaves it as it was".to_owned(),
                    ]
                } else {
                    // The session is open: `x` would do what `s` does.
                    vec![
                        "space picks".to_owned(),
                        "e edits".to_owned(),
                        "enter keeps what is picked".to_owned(),
                        "s skips this step".to_owned(),
                    ]
                };
                (
                    StepView {
                        title: "Commands".to_owned(),
                        number: 4,
                        sentence,
                        rows,
                        selected: Some(commands.selected),
                        note: None,
                    },
                    keys,
                )
            }
            Step::Waiting | Step::Opened => (StepView::default(), Vec::new()),
        }
    }
}

/// What the imports' line says was brought in.
fn imports_said(imported: Option<&Imported>) -> String {
    let Some(imported) = imported else {
        return "Imports: skipped; nothing of other tools' is read.".to_owned();
    };
    let mut parts = Vec::new();
    if imported.instructions {
        parts.push("AGENTS.md and CLAUDE.md read".to_owned());
    }
    if imported.skills {
        parts.push("skills folders read, once this version reads skills".to_owned());
    }
    let names: Vec<String> = imported
        .servers
        .iter()
        .map(|(_, name)| one_line(name))
        .collect();
    match names.len() {
        0 => {}
        1 => parts.push(format!("the MCP server {}", names[0])),
        _ => parts.push(format!("the MCP servers {}", and_list(&names))),
    }
    if parts.is_empty() {
        "Imports: nothing brought in.".to_owned()
    } else {
        format!("Imports: {}.", parts.join("; "))
    }
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
