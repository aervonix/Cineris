//! The first run in the application (since step 25;
//! `docs/architecture/08-configuration.md`, "First run";
//! `docs/architecture/09-tui.md`, "The first run").
//!
//! **The order:** the provider and its key, what other tools left,
//! whether the user's own skill folders are read without asking (since step
//! 28's third pull request, the owner's answer: no is the default, and a
//! skip is no), the mode; then the configuration file is written ([`Setup::write_config`]),
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
//! **Since the full review of the third pull request:** a step is shown,
//! and takes keys, only when nothing else has the input box's place: never
//! under a dialog or the viewer, and the last step, which comes once the
//! session is open, only while the session is idle (no turn, no prompt on
//! its way, the input box empty; [`App::first_run_idle`]); until then the
//! input box is the input box. On its first frame a step takes what was
//! typed just before it and the key that answered what came before it, as
//! a dialog does; hidden and shown again, it starts as if new; its rows
//! changed under it by the worker, its grace starts again. The imports'
//! selection is the row it is, not its place. A step fits the window: the
//! card goes first, then its rows are a window around the selected one,
//! which is drawn whole, and a row is accepted only once it has been.
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

use super::{App, LastKey, Pending};
use crate::app::Open;
use crate::app::ready::{REPEAT, Readiness, scaled};
use crate::app::setup::{
    Answer, Chosen, Found, Imported, Job, KeyStore, McpServer, NotWritten, Providers, Setup, Worker,
};
use crate::keys::{Action, Context};
use crate::secret::{KEYS as KEY_KEYS, KeyEdit, KeyInput, key_box};
use crate::view::setup::{Fitted, Mark, Row, Step as StepView, card, fitted};
use crate::view::{Hint, one_line};

/// What an operation the first run sent is for.
#[derive(Debug, Clone)]
pub(super) enum Sent {
    /// The configuration read again after the file was written.
    Reload,
    /// The configuration read again after a file was found at the name,
    /// made meanwhile: what was chosen applies over it for the run (since
    /// the short review of the fixes of the third pull request).
    FoundRead,
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
    /// The configuration file, as the person reads its path, once written;
    /// why it was not, when it was not.
    written: Option<Result<String, NotWritten>>,
    /// `x` was pressed: the steps left take their defaults.
    skipped: bool,
    /// The configuration was read again: the session may open.
    released: bool,
    /// The first run's own session is being opened: an open that fails
    /// ends the first run only then, not a later `/new`'s, `/resume`'s or
    /// fork's (since the review of the fixes of the third pull request).
    opening: bool,
    /// A file found at the configuration's name could not be read when it
    /// was read at once: it is not in force now (since the final review).
    found_unread: bool,
    /// A frame has drawn the step's keys, which it shows once it is ready,
    /// as a dialog shows its answers: it takes keys only then.
    keys_drawn: bool,
    /// The step's readiness has been seeded since it was last shown: with
    /// the input nothing took and the key something took, as a dialog's is
    /// on its first frame (the full review of the third pull request).
    seeded: bool,
    /// The last frame drew the selected row whole: a key that accepts it
    /// (`enter` on a provider or a mode, `space` on a server or a command)
    /// is taken only then (the full review of the third pull request).
    whole: bool,
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
    /// Whether the user's own skill folders are read without asking
    /// (`permissions.read_skill_folders`): `0` no, the default, `1` yes.
    SkillFolders { selected: usize },
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
    /// The row selected, by what it is: a file's answer inserts its
    /// servers above the rows after it, and the selection stays where it
    /// was (the full review of the third pull request).
    selected: Pick,
    instructions: bool,
    skills: bool,
    files: Vec<File>,
    /// Why the last `space` picked nothing, until the next key.
    said: Option<String>,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Pick {
    #[default]
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
    /// Why a paste into the edit, or the edit kept, was refused.
    refusal: Option<String>,
    trusted: bool,
}

/// The most characters a command edited in the last step may have.
const EDIT_MAX: usize = 1_000;

/// A key typed into a command's edit field: `Backspace`, `Ctrl+U` and
/// characters that can be shown; nothing else does anything there.
fn edit_text(text: &mut String, key: Key) {
    let plain = key.mods.is_empty() || key.mods == Mods::SHIFT;
    match key.code {
        KeyCode::Backspace => {
            text.pop();
        }
        KeyCode::Char('u') if key.mods == Mods::CTRL => text.clear(),
        KeyCode::Char(character)
            if plain
                && !cineris_base::acts_unseen(character)
                && text.chars().count() < EDIT_MAX =>
        {
            text.push(character);
        }
        _ => {}
    }
}

/// A paste into a command's edit field: kept whole, a line break at its
/// end left off, or refused whole, with why.
fn paste_text(text: &mut String, pasted: &str) -> Result<(), &'static str> {
    let pasted = pasted
        .strip_suffix("\r\n")
        .or_else(|| pasted.strip_suffix('\n'))
        .or_else(|| pasted.strip_suffix('\r'))
        .unwrap_or(pasted);
    if pasted.chars().any(cineris_base::acts_unseen) {
        return Err(
            "That paste holds a line break or a character that cannot be shown: nothing of it \
             was kept.",
        );
    }
    if text.chars().count() + pasted.chars().count() > EDIT_MAX {
        return Err("That is longer than a command can be here: nothing of it was kept.");
    }
    text.push_str(pasted);
    Ok(())
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
    /// The first run on `setup`.
    ///
    /// # Errors
    ///
    /// Why its worker could not start ([`App::first_run_unstarted`] says
    /// it).
    pub(super) fn start(setup: Arc<dyn Setup>) -> std::io::Result<Self> {
        let worker = Worker::start(setup)?;
        worker.ask(Job::Providers);
        Ok(Self {
            worker,
            step: Step::Provider { selected: 0 },
            ready: Readiness::default(),
            taken: None,
            chosen: Chosen::default(),
            providers: None,
            written: None,
            skipped: false,
            released: false,
            opening: false,
            found_unread: false,
            keys_drawn: false,
            seeded: false,
            whole: false,
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

    /// Not drawn in the last frame (under a dialog, the viewer or a turn):
    /// shown again, it starts as if new, its grace from its next frame and
    /// its readiness seeded again.
    fn hidden(&mut self) {
        self.ready.hide();
        self.keys_drawn = false;
        self.seeded = false;
        self.whole = false;
    }

    /// Its rows changed under it, from the worker (the providers came, a
    /// file was read): its grace starts again with its next frame, keeping
    /// the quiet it owes input it did not take, so that a key meant for
    /// what was there does not land on what came.
    fn rows_changed(&mut self) {
        self.ready.again();
        self.keys_drawn = false;
    }

    /// Whether the set-up card is shown: until the session opens.
    fn card(&self) -> bool {
        matches!(
            self.step,
            Step::Provider { .. }
                | Step::Key { .. }
                | Step::Imports(_)
                | Step::SkillFolders { .. }
                | Step::Mode { .. }
        )
    }

    /// A new step, ready only after its grace and a key's repeat window.
    fn to(&mut self, step: Step) {
        self.step = step;
        self.keys_drawn = false;
        self.seeded = false;
        self.whole = false;
        self.ready = Readiness::default();
        if let Some(taken) = self.taken {
            self.ready.not_before(taken + scaled(REPEAT));
        }
    }

    /// `provider` is chosen, and with it its model: the one the catalog
    /// marks as its default, written as `model.main` (since the full review
    /// of the third pull request), and whether another provider has a
    /// model of its id, which names the provider beside it (since the
    /// review of the fixes).
    fn choose(&mut self, provider: &str) {
        let known = self
            .providers
            .as_ref()
            .and_then(|found| found.providers.iter().find(|known| known.name == provider));
        self.chosen.provider = Some(provider.to_owned());
        self.chosen.model = known.and_then(|known| known.model.clone());
        self.chosen.model_shared = known.is_some_and(|known| known.model_shared);
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

    /// Whether a step of the first run is what the input box's place
    /// shows, and so what takes the keys: never under a dialog or the
    /// viewer, and the last step, which comes once the session is open,
    /// only while it is idle ([`App::first_run_idle`]). A step not shown
    /// takes no key: the keys go where they would without it (the full
    /// review of the third pull request).
    pub(super) fn first_run_shown(&self) -> bool {
        let Some(run) = &self.first_run else {
            return false;
        };
        match run.step {
            Step::Waiting | Step::Opened => false,
            Step::Commands(_) => self.first_run_idle(),
            _ => self.dialogs.is_empty() && !self.viewing(),
        }
    }

    /// Whether the session is idle, for the last step, which is asked for
    /// and shown only then: open, no turn running and no prompt on its
    /// way, no dialog, not under the viewer, the full screen's search or a
    /// rewind, no item of a panel being edited, and the input box empty
    /// with no menu open. Until then the input box is the input box, and
    /// a prompt sent from it is a prompt (the full review of the third
    /// pull request: the step took the input box's place under a turn).
    fn first_run_idle(&self) -> bool {
        self.session.is_some()
            && !self.opening
            && self.running.is_none()
            && !self.turn_runs
            && self.held_prompts.is_empty()
            && self.held_sent.is_none()
            && self.dialogs.is_empty()
            && !self.viewing()
            && !self.screen_searching()
            && !self.holds_prompts_for_rewind()
            && self.item_edit.is_none()
            && self.input.editor().is_empty()
            && self.input.menu().is_none()
            && self.input.search().is_none()
    }

    /// A frame was drawn at `at`. A step shown starts its grace, and on
    /// its first frame takes what was typed just before it and the key
    /// that answered what came before it, as a dialog does; a step not
    /// shown starts again as if new (the full review of the third pull
    /// request).
    pub(super) fn first_run_drawn(&mut self, at: Instant) {
        let shown = self.first_run_shown();
        if !shown {
            self.first_run_edit_cut(at);
        }
        let (untaken, taken) = (self.untaken, self.taken);
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        if !shown {
            run.hidden();
            return;
        }
        run.ready.drawn(at);
        if !run.seeded {
            run.seeded = true;
            if let Some(untaken) = untaken {
                run.ready.typed(untaken);
            }
            if let Some(taken) = taken {
                run.ready.not_before(taken + scaled(REPEAT));
            }
        }
    }

    /// The program was continued after a stop (`fg`): what was typed
    /// meanwhile went to the shell, and the step starts again as if just
    /// shown, as a dialog does (the early review of the third pull
    /// request's fixes: the `Enter` that ended `fg` chose a mode).
    pub(super) fn first_run_resumed(&mut self) {
        if let Some(run) = self.first_run.as_mut() {
            run.hidden();
        }
    }

    /// Input the first run did not take: its step waits a quiet second.
    pub(super) fn first_run_typed(&mut self, at: Instant) {
        if let Some(run) = self.first_run.as_mut() {
            run.typed(at);
        }
    }

    /// Input the first run's step did not take, read at `at`: what is
    /// shown next, a dialog or a step, waits a quiet second after it, as
    /// after typing into the editor.
    fn first_run_untaken(&mut self, at: Instant) {
        self.untaken = Some(at);
        self.panel_untaken = Some(at);
        self.last_key = Some(LastKey { typing: true });
    }

    /// When the step shown is to be drawn with its keys: the end of its
    /// grace, until a frame has drawn them.
    pub(super) fn first_run_deadline(&self) -> Option<Instant> {
        if !self.first_run_shown() {
            return None;
        }
        let run = self.first_run.as_ref()?;
        if run.keys_drawn {
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
                    run.rows_changed();
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
                run.choose(&provider);
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
                    run.rows_changed();
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
                    run.rows_changed();
                }
            }
            Answer::Written(written) => {
                let mode = run
                    .chosen
                    .mode
                    .clone()
                    .unwrap_or_else(|| "supervised".to_owned());
                run.written = Some(written.clone());
                match written {
                    Ok(path) => {
                        self.say(&format!("Mode: {mode}. Written to {}.", one_line(&path)));
                        // The core reads the file again, so that the session
                        // is made with what was chosen; it opens once that is
                        // done.
                        let ticket = self
                            .link
                            .send(Op::ConfigReload(ConfigReloadParams::default()));
                        self.pending.insert(ticket, Pending::FirstRun(Sent::Reload));
                    }
                    // Nothing was saved: this run's sessions carry what was
                    // chosen as their own settings, and the set-up runs again
                    // next time (the full review of the third pull request:
                    // the line said "for this session" of a mode the session
                    // never had).
                    Err(NotWritten::Failed(why)) => {
                        self.say(&format!(
                            "Mode: {mode}, for this run. Nothing was saved: {}. This run's \
                             sessions are opened with what was chosen, and the set-up runs \
                             again at the next start.",
                            one_line(&why)
                        ));
                        self.first_run_open(true);
                    }
                    // A file made meanwhile was found at the name, and left
                    // as it is, and the set-up does not run again (the
                    // review of the fixes: this was said to be nothing
                    // saved). It is read now, as the core's watch would read
                    // it at the next key, and what was chosen applies over it
                    // for the run (the short review of the fixes: it was
                    // said to be in force only from the next start).
                    Err(NotWritten::Found(path)) => {
                        self.say(&format!(
                            "Mode: {mode}, for this run. {} was made meanwhile, not by the \
                             set-up, and was left as it is: it is read now, and what was \
                             chosen applies over it for this run.",
                            one_line(&path)
                        ));
                        let ticket = self
                            .link
                            .send(Op::ConfigReload(ConfigReloadParams::default()));
                        self.pending
                            .insert(ticket, Pending::FirstRun(Sent::FoundRead));
                    }
                }
            }
        }
    }

    /// The session opens: made with what was chosen, from the file the core
    /// read again, or, `own`, with the choices as the session's own settings
    /// because the file was not written or not read again. Every session
    /// this run opens carries them then (`/new`'s too).
    fn first_run_open(&mut self, own: bool) {
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
        run.step = Step::Opened;
        run.released = true;
        if own {
            // Over a file found at the name, what was chosen names its
            // provider beside its model: the file's own role provider
            // would hold the chosen model to another's (the final review).
            let found = matches!(run.written, Some(Err(NotWritten::Found(_))));
            self.run_config = chosen_config(&run.chosen, found);
        }
        // The session made with what was chosen: its header is the one
        // card from now on.
        if let Some(open) = self.open.take() {
            run.opening = true;
            let open = if own { self.own_open(open) } else { open };
            self.send_open(open);
        }
    }

    /// The session the run opens, when what was chosen is this run's own
    /// configuration (`App::run_config`) and not in force from a file: one
    /// resumed, forked or continued is opened with the configuration in
    /// force, which `session/open` cannot change, so a new session, which
    /// carries it, is opened in its place, and that is said (the review of
    /// the fixes of the third pull request: the defaults applied, and an
    /// `AGENTS.md` left unpicked could reach the prompt).
    fn own_open(&mut self, open: Open) -> Open {
        if matches!(open, Open::New { .. }) {
            return open;
        }
        self.say(
            "This is a new session, not the one asked for: a session resumed, forked or continued \
             opens with the configuration in force, without what this run's sessions carry. \
             `/resume` opens the other as it is.",
        );
        let (settings, ephemeral) = self.fresh.clone();
        Open::New {
            workspace: self.workspace.clone(),
            settings,
            ephemeral,
        }
    }

    /// The first run's own session is open (since the review of the fixes
    /// of the third pull request): an open that fails after it is not the
    /// first run's.
    pub(super) fn first_run_session_opened(&mut self) {
        if let Some(run) = self.first_run.as_mut() {
            run.opening = false;
        }
    }

    /// The first run could not start, its worker refused: said, and this
    /// run's sessions read nothing of other tools', as a skipped set-up
    /// leaves them (the full review of the third pull request: it was
    /// dropped without a word, and the defaults read `AGENTS.md`); a
    /// session to be resumed or continued is a new one, which can carry
    /// that (the review of its fixes).
    pub(super) fn first_run_unstarted(&mut self, error: &std::io::Error) {
        self.run_config = chosen_config(&Chosen::default(), false);
        self.say(&format!(
            "The set-up could not start ({}): this run's sessions read nothing of other tools', \
             and the set-up runs again at the next start. `cineris auth login <provider>` stores \
             a key meanwhile.",
            one_line(&error.to_string())
        ));
        if let Some(open) = self.open.take() {
            self.open = Some(self.own_open(open));
        }
    }

    /// An operation the first run sent was answered.
    pub(super) fn first_run_done(&mut self, sent: Sent, result: Result<OpResult, ErrorData>) {
        match (sent, result) {
            (sent @ (Sent::Reload | Sent::FoundRead), result) => {
                let found = matches!(sent, Sent::FoundRead);
                // A reload that failed, or found the files wrong, leaves the
                // configuration in force as it was: the file written is not
                // read, and the session carries what was chosen itself.
                let failed = match &result {
                    Err(error) => Some(error.message.clone()),
                    Ok(OpResult::ConfigReload(reload)) if !reload.errors.is_empty() => {
                        Some(reload.errors.join("; "))
                    }
                    Ok(_) => None,
                };
                if let Some(why) = &failed {
                    let file = match self.first_run.as_ref().and_then(|run| run.written.clone()) {
                        Some(Ok(path) | Err(NotWritten::Found(path))) => one_line(&path),
                        _ => "The settings".to_owned(),
                    };
                    self.say(&if found {
                        format!(
                            "{file} could not be read: {}. This run's sessions are opened with \
                             what was chosen.",
                            one_line(why)
                        )
                    } else {
                        format!(
                            "{file} was written, but the configuration could not be read \
                             again: {}. This run's sessions are opened with what was chosen.",
                            one_line(why)
                        )
                    });
                }
                if found
                    && failed.is_some()
                    && let Some(run) = self.first_run.as_mut()
                {
                    run.found_unread = true;
                }
                self.first_run_open(failed.is_some() || found);
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
                        refusal: None,
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
    /// open and idle ([`App::first_run_idle`]), or, after `x`, the first
    /// run's end once no dialog is up.
    fn first_run_next(&mut self) {
        let Some(run) = self.first_run.as_ref() else {
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
        // Not under a turn, a dialog, the viewer or what is being typed:
        // the step that follows takes the input box's place (the full
        // review of the third pull request).
        if !self.first_run_idle() {
            return;
        }
        let Some(run) = self.first_run.as_mut() else {
            return;
        };
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

    /// The first run ends: its last line says the provider and the model
    /// the session runs with, how to change them and do each step again,
    /// by commands this version has, and whether the settings were saved
    /// (the full review of the third pull request: no model was written,
    /// and the line did not say so).
    fn first_run_end(&mut self) {
        let Some(run) = self.first_run.take() else {
            return;
        };
        let model = match (&run.chosen.provider, &run.chosen.model) {
            (Some(provider), Some(model)) => format!(
                "The model is {}'s {} (model.main).",
                one_line(provider),
                one_line(model)
            ),
            (Some(provider), None) => format!(
                "No model of {} is known: model.main names one.",
                one_line(provider)
            ),
            (None, _) => "No provider was chosen, so there is no model yet: model.main names one \
                          once a key is stored."
                .to_owned(),
        };
        self.say(&format!(
            "{} {model} `/model` chooses another model for this session, `/mode` sets the mode, \
             `/project` shows this project's commands, and `cineris auth login <provider>` \
             stores a key. {}",
            if run.skipped {
                "Set-up skipped."
            } else {
                "Set-up done."
            },
            saved(&run)
        ));
    }

    /// The session could not be opened: when it was the first run's own,
    /// the first run ends there, saying what was saved (the full review of
    /// the third pull request: it waited for good, and its last line never
    /// came). A later open's failure, `/new`'s at the last step say, is not
    /// the first run's: it waits on for a session (the review of the
    /// fixes).
    pub(super) fn first_run_open_failed(&mut self) {
        if !self.first_run.as_ref().is_some_and(|run| run.opening) {
            return;
        }
        let Some(run) = self.first_run.take() else {
            return;
        };
        self.say(&format!(
            "Set-up ended: the session could not be opened. {} /new tries again.",
            saved(&run)
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
        // What the steps before answered stays answered: only the steps
        // not reached take their defaults (the full review of the third
        // pull request: `x` at the mode threw the imports away).
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
    /// does not ([`App::first_run_shown`]), and the event goes where it
    /// would otherwise. What the step takes, and what it does not, count
    /// for what comes after it as a dialog's keys do (the full review of
    /// the third pull request).
    pub(super) fn first_run_event(&mut self, event: &TermEvent, at: Instant) -> Option<bool> {
        // Whether the step was ready for a key is judged when it was
        // pressed (`App::terminal_pressed`); what it holds counts from `at`.
        let pressed = self.judged(at);
        if !self.first_run_shown() {
            self.first_run_edit_cut(at);
            return None;
        }
        let run = self.first_run.as_mut()?;
        // Typing into a command's edit field is typing, as into the key's
        // input: it goes in whenever the step is shown, and only `Enter` and
        // `Esc` wait for the step's keys (the full review of the third pull
        // request: a repeat or a paste was dropped, and hid the keys).
        if let Step::Commands(commands) = &mut run.step
            && let Some(text) = &mut commands.editing
        {
            // Taken by the field, and why a paste was refused.
            let taken: Option<Option<&'static str>> = match event {
                TermEvent::Key(key) | TermEvent::Repeat(key)
                    if !matches!(key.code, KeyCode::Enter | KeyCode::Esc)
                        && self
                            .input
                            .keymap()
                            .lookup(&[Context::Global], *key)
                            .is_none() =>
                {
                    edit_text(text, *key);
                    Some(None)
                }
                TermEvent::Paste(pasted) => Some(paste_text(text, pasted).err()),
                _ => None,
            };
            if let Some(refused) = taken {
                commands.refusal = refused.map(str::to_owned);
                self.first_run_untaken(at);
                return Some(true);
            }
        }
        let run = self.first_run.as_mut()?;
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
                    self.first_run_untaken(at);
                    return Some(true);
                }
                if !run.takes_keys(pressed) {
                    run.typed(at);
                    // The last step's, before it takes keys, goes to the
                    // input box, as a dialog's typing does: what it types
                    // there hides the step until it is sent or emptied (the
                    // early review of the third pull request's fixes: it
                    // was dropped, as people began their next prompt).
                    if matches!(run.step, Step::Commands(_)) {
                        return None;
                    }
                    self.first_run_untaken(at);
                    return Some(true);
                }
                // A key the step took: what comes next waits a key's
                // repeat window after it. Held down, it goes where its press
                // went, nowhere, also once the step it answered has gone
                // (the short review of the fixes: a held `s` that ended the
                // last step typed `sss` into the input box), as a panel's.
                self.taken = Some(at);
                self.last_key = Some(LastKey { typing: false });
                self.panel_held = Some((*key, at, false));
                self.first_run_key(*key, at);
                Some(true)
            }
            // A key held down never answers a step. The last step's goes
            // where its press went: to the input box when the step did not
            // take it, and nowhere when it did, the press being marked held
            // as a panel's is (`App::panel_holds`; the review of the fixes
            // of the third pull request: held, `Down` recalled the history
            // into the input box, and `space` typed spaces there).
            TermEvent::Repeat(_) => {
                run.typed(at);
                if matches!(run.step, Step::Commands(_)) {
                    return None;
                }
                self.first_run_untaken(at);
                Some(true)
            }
            // A paste is the key's, or nobody's: it never answers a step.
            // At the last step it is the input box's, as at a dialog.
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
                    Step::Commands(_) => {
                        run.typed(at);
                        return None;
                    }
                    _ => run.typed(at),
                }
                self.first_run_untaken(at);
                Some(true)
            }
            _ => None,
        }
    }

    /// The last step is not shown while a command is being edited in it
    /// (a turn or a dialog came): the edit is dropped, said, and an `Enter`
    /// that was to keep it does not send what the input box holds as a
    /// prompt until the typing pauses (the early review of the third pull
    /// request's fixes).
    fn first_run_edit_cut(&mut self, at: Instant) {
        if let Some(run) = self.first_run.as_mut()
            && let Step::Commands(commands) = &mut run.step
            && commands.editing.take().is_some()
        {
            self.note_cut = Some(at);
            self.say(
                "The commands' step gave way while a command was being edited: the edit was left, \
                 and the step comes back once nothing else is going on.",
            );
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
        // A row is accepted only once it has been drawn whole: in a window
        // too small for it, `enter` on it and `space` on it do nothing (the
        // full review of the third pull request).
        let whole = run.whole;
        // The selection moved: what the last frame drew whole was another
        // row, and a key before the next frame accepts nothing (the early
        // review of the third pull request's fixes).
        if up || down {
            run.whole = false;
        }
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
                } else if enter
                    && whole
                    && let Some((provider, how)) = rows.get(*selected).cloned()
                {
                    run.taken = Some(at);
                    match how {
                        How::Type => run.to(Step::Key {
                            provider,
                            input: KeyInput::new(),
                            storing: false,
                            error: None,
                        }),
                        How::Variable(variable) => {
                            run.choose(&provider);
                            run.worker.ask(Job::Discover);
                            run.to(Step::Imports(Imports::default()));
                            self.say(&format!(
                                "Provider: {}, its key from {}.",
                                one_line(&provider),
                                one_line(&variable)
                            ));
                        }
                        How::Stored => {
                            run.choose(&provider);
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
                imports.said = None;
                let picks = imports.picks();
                // Where the selection is now: rows a file's answer inserted
                // above it move it down, not onto them.
                let place = picks
                    .iter()
                    .position(|pick| *pick == imports.selected)
                    .unwrap_or(0);
                if up {
                    if let Some(pick) = picks.get(place.saturating_sub(1)) {
                        imports.selected = *pick;
                    }
                } else if down {
                    if let Some(pick) = picks.get((place + 1).min(picks.len().saturating_sub(1))) {
                        imports.selected = *pick;
                    }
                } else if space && whole {
                    match picks.get(place) {
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
                            // One name is one server in the user's file: a
                            // second of a name picked already is not picked
                            // (the full review of the third pull request:
                            // the writer kept one and dropped the other).
                            let taken_by = imports.files.get(*file).and_then(|this| {
                                let Some(Ok(servers)) = &this.servers else {
                                    return None;
                                };
                                let (wanted, picked) = servers.get(*server)?;
                                if *picked {
                                    return None;
                                }
                                imports.files.iter().enumerate().find_map(|(index, other)| {
                                    let Some(Ok(servers)) = &other.servers else {
                                        return None;
                                    };
                                    servers
                                        .iter()
                                        .enumerate()
                                        .any(|(number, (other, also))| {
                                            *also
                                                && other.name == wanted.name
                                                && (index, number) != (*file, *server)
                                        })
                                        .then(|| (wanted.name.clone(), other.path.clone()))
                                })
                            });
                            if let Some((name, path)) = taken_by {
                                imports.said = Some(format!(
                                    "A server named {} is picked already, from {}: one name is \
                                     one server.",
                                    one_line(&name),
                                    one_line(&path)
                                ));
                            } else if let Some(Some(Ok(servers))) =
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
                    run.to(Step::SkillFolders { selected: 0 });
                    self.say(&said);
                }
            }
            // No is the default, and a skip is no (the owner's answer).
            Step::SkillFolders { selected } => {
                if up {
                    *selected = 0;
                } else if down {
                    *selected = 1;
                } else if (enter && whole) || letter == Some('s') {
                    run.taken = Some(at);
                    let yes = enter && *selected == 1;
                    run.chosen.read_skill_folders = yes;
                    run.to(Step::Mode { selected: 1 });
                    self.say(if yes {
                        "Skill folders: your own are read without asking, and what is read is \
                         sent to the model's provider."
                    } else {
                        "Skill folders: a read of your own is asked about. `cineris setup \
                         skill-folders` changes it."
                    });
                }
            }
            Step::Mode { selected } => {
                if up {
                    *selected = selected.saturating_sub(1);
                } else if down {
                    *selected = (*selected + 1).min(levels.len().saturating_sub(1));
                } else if (enter && whole) || letter == Some('s') {
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
                // Typing into the edit is taken when the step is shown
                // (`first_run_event`); its `Enter` and `Esc` wait for the
                // step's keys.
                if let Some(text) = &mut commands.editing {
                    match key.code {
                        KeyCode::Enter if key.mods.is_empty() && whole => {
                            let edited = text.trim().to_owned();
                            // An edit kept is a pick; one that changed
                            // nothing is not (the early review of the third
                            // pull request's fixes).
                            if let Some((row, picked)) = commands.rows.get_mut(commands.selected)
                                && !edited.is_empty()
                                && edited != row.command
                            {
                                row.command = edited;
                                *picked = true;
                            }
                            commands.editing = None;
                            commands.refusal = None;
                            // A held `Enter`, or a second one, does not keep
                            // the commands at once: the step waits a key's
                            // repeat window, as a new step does.
                            run.ready.not_before(at + scaled(REPEAT));
                            run.keys_drawn = false;
                        }
                        KeyCode::Esc => {
                            commands.editing = None;
                            commands.refusal = None;
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
                } else if space && whole {
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
    /// above them until the session opens, and its keys under them, in at
    /// most the window's rows: the card goes first when they do not fit,
    /// then the step is fitted ([`fitted`]). `None` when it shows no step.
    pub(super) fn first_run_bottom(&mut self, now: Instant, width: usize) -> Option<Vec<Line>> {
        if !self.first_run_shown() {
            return None;
        }
        let quit_armed = self.input.quit_armed();
        let run = self.first_run.as_mut()?;
        // Its keys once it is ready, as a dialog's answers; not while the
        // hint row says what a second `Ctrl+C` does in their place (the
        // early review of the third pull request's fixes).
        let ready = run.ready.ready(now);
        run.keys_drawn = ready && !quit_armed;
        let run = self.first_run.as_ref()?;
        let look = &self.look;
        let (view, mut keys) = self.first_run_view(run);
        // Under the step: the key's box and what it says, or a blank; then
        // the hint row.
        let mut under = Vec::new();
        under.push(Line::new());
        if let Step::Key {
            input,
            error,
            storing,
            ..
        } = &run.step
        {
            under.extend(key_box(input, width, look));
            let said = if *storing {
                Some(format!("storing{}", look.glyphs.ellipsis))
            } else {
                error.clone().or_else(|| input.refusal().map(str::to_owned))
            };
            if let Some(said) = said {
                under.extend(crate::secret::sentence(&said, width, look));
            }
        }
        // The window's rows (the full review of the third pull request): a
        // step taller than them lost its title and its sentence off the
        // top, and its selected row could be off the screen.
        let room = self.rows.max(1).saturating_sub(under.len() + 1);
        let lead = usize::from(!self.kept.is_empty());
        let set_up = if run.card() {
            let mut lines = card(&self.version, width, look);
            lines.push(Line::new());
            lines
        } else {
            Vec::new()
        };
        // Drawn whole by height is not drawn whole by width: a row cut to
        // the window's width, or whose label its column cuts, is not (the
        // review of the fixes of the third pull request: a step that fitted
        // by height was said whole).
        let Fitted { lines: all, whole } = fitted(&view, width, look, usize::MAX);
        let mut lines = Vec::new();
        let whole = if lead + set_up.len() + all.len() <= room {
            lines.extend((lead == 1).then(Line::new));
            lines.extend(set_up);
            lines.extend(all);
            whole
        } else if lead + all.len() <= room {
            lines.extend((lead == 1).then(Line::new));
            lines.extend(all);
            whole
        } else if all.len() <= room {
            lines.extend(all);
            whole
        } else {
            let fitted = fitted(&view, width, look, room);
            lines.extend(fitted.lines);
            fitted.whole
        };
        lines.extend(under);
        if !whole {
            keys.insert(0, "a larger window shows this row whole".to_owned());
        }
        let hint = if quit_armed {
            Hint::QuitArmed
        } else if ready {
            Hint::Keys(keys)
        } else {
            Hint::Empty
        };
        lines.push(self.hint_row(hint, now));
        if let Some(run) = self.first_run.as_mut() {
            run.whole = whole;
        }
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
                // As the keys find it: the first row when it is gone.
                let selected = picks
                    .contains(&imports.selected)
                    .then_some(imports.selected)
                    .or_else(|| picks.first().copied());
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
                                // What the person accepts is not all on the
                                // screen: said (the full review of the third
                                // pull request).
                                if server.hidden && server.refused.is_none() {
                                    detail.push(format!(
                                        "a value shown {} is imported as it is in that file, \
                                         unseen here",
                                        cineris_base::redact::MASK
                                    ));
                                }
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
                view.note = imports.said.clone();
                let keys = match selected {
                    Some(Pick::File(_)) => "space reads or folds it",
                    _ => "space picks",
                };
                let mut all = vec![keys.to_owned(), "enter keeps what is picked".to_owned()];
                all.extend(skip);
                (view, all)
            }
            Step::SkillFolders { selected } => (
                StepView {
                    title: "Skill folders".to_owned(),
                    number: 3,
                    sentence: crate::app::setup::SKILL_FOLDERS_QUESTION.to_owned(),
                    rows: crate::app::setup::SKILL_FOLDERS_ANSWERS
                        .iter()
                        .map(|(answer, means)| {
                            Row::new(0, Mark::None, *answer, vec![(*means).to_owned()])
                        })
                        .collect(),
                    selected: Some(*selected),
                    note: None,
                },
                vec![
                    "enter chooses".to_owned(),
                    "s keeps it off".to_owned(),
                    "x skips the set-up".to_owned(),
                ],
            ),
            Step::Mode { selected } => {
                let levels = run.levels();
                (
                    StepView {
                        title: "Mode".to_owned(),
                        number: 4,
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
                        number: 5,
                        sentence,
                        rows,
                        selected: Some(commands.selected),
                        note: commands.refusal.clone(),
                    },
                    keys,
                )
            }
            Step::Waiting | Step::Opened => (StepView::default(), Vec::new()),
        }
    }
}

/// Whether the settings chosen were saved, as a sentence.
fn saved(run: &FirstRun) -> String {
    match &run.written {
        Some(Ok(path)) if !path.is_empty() => format!(
            "{} holds the settings, and a change there is kept.",
            one_line(path)
        ),
        // Found at the name, and not read when it was read at once (the
        // final review: the line said it was in force all the same).
        Some(Err(NotWritten::Found(path))) if run.found_unread => format!(
            "Nothing of the set-up's was saved: {}, made meanwhile, could not be read, so this \
             run's sessions carry what was chosen; the next start reads it again, and the set-up \
             does not run again.",
            one_line(path)
        ),
        // Found at the name, not written (the review of the fixes of the
        // third pull request).
        Some(Err(NotWritten::Found(path))) => format!(
            "Nothing of the set-up's was saved: {}, made meanwhile, is the configuration now, \
             what was chosen applying over it for this run, and the set-up does not run again.",
            one_line(path)
        ),
        _ => "Nothing was saved, so the set-up runs again at the next start.".to_owned(),
    }
}

/// What was chosen as a session's own configuration (`session/new`'s
/// `config`, which ranks as the command line does), for when the file
/// written is not in force: the instructions and the skills as picked, off
/// unless picked; the mode; the model, and its provider beside it only when
/// another provider has a model of its id, as the file has them (the review
/// of the fixes of the third pull request), or `over_found`, over a file
/// found at the name, whose own role provider would otherwise hold the
/// model (the final review). The servers picked are not among them: this
/// version runs no MCP server.
fn chosen_config(chosen: &Chosen, over_found: bool) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::{Value, json};
    let imports = chosen.imports.clone().unwrap_or_default();
    let mut config = serde_json::Map::new();
    config.insert(
        "compat".to_owned(),
        json!({ "instructions": imports.instructions, "skills": imports.skills }),
    );
    let mut permissions = serde_json::Map::new();
    if let Some(mode) = &chosen.mode {
        permissions.insert("mode".to_owned(), Value::String(mode.clone()));
    }
    permissions.insert(
        "read_skill_folders".to_owned(),
        Value::Bool(chosen.read_skill_folders),
    );
    config.insert("permissions".to_owned(), Value::Object(permissions));
    if let Some(main) = &chosen.model {
        let mut model = serde_json::Map::new();
        model.insert("main".to_owned(), Value::String(main.clone()));
        if (chosen.model_shared || over_found)
            && let Some(provider) = &chosen.provider
        {
            model.insert(
                "roles".to_owned(),
                json!({ "main": { "provider": provider } }),
            );
        }
        config.insert("model".to_owned(), Value::Object(model));
    }
    config
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
