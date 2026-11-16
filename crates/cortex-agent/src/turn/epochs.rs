//! The loop's part in context management
//! (`docs/architecture/07-sessions-and-context.md`): when an epoch begins,
//! the instruction files, the date, the budget, and compaction.
//!
//! The [`Context`](crate::context) holds what is sent. What needs the loop
//! is here: reading files (on the blocking pool), records in the log (the
//! loop's store, and always before the context is told), the models of the
//! roles, and the usage and the ledger.
//!
//! **Where each thing happens.**
//!
//! | When | What |
//! |---|---|
//! | A turn starts, before its prompt | The epoch: begun, or held against the prompt the harness would build now. Instruction files that changed, appeared or went. |
//! | Before each request (a safe boundary: no round is open) | A change of the main role's model starts an epoch. Instruction files under directories a tool first touched. The budget: a summary in the background from `prepare_at`, a compaction from `compact_at`. |
//! | Compacting | The summary prepared in the background if it is still good (waited for if it is not ready), or one asked for now. Then the `summary` entry, the `epoch` entry, and the new epoch. |
//! | After each request | The context record, and in a debug build the check that the request began with the one before it. |

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use cortex_base::{AbsPath, CancelToken, Classified, ContentHash};
use cortex_llm::{ModelClient, ModelEvent, ModelRequest, Usage};
use cortex_protocol::event::{
    CompactionNotice, CompactionState, Event, InfoNotice, InstructionsNotice, ItemCompleted, Notice,
};
use cortex_protocol::item::{NoticeSource, SummaryItem};
use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::RequestUsage;
use cortex_protocol::{
    ErrorData, ErrorKind, Item, ItemId, Message, Part, Role, Speed, StopReason, TurnId,
};
use cortex_tools::run_blocking;

use super::{End, Phase, Prep, Session, Turn};
use crate::config::{RoleModel, UtilityCall};
use crate::context::{
    Budget, Built, Carried, Change, Discovered, Found, History, InFlight, Place, PrefixDifference,
    Pressure, SUMMARY_CUT, Sections, Shape, build, check_prefix, difference, discover, excerpt,
    fraction_of, hash_of, instructions_message, limit_of, nested_dirs, notice_text, places,
    pressure, project, read_one, request_bytes, summary_room,
};
use crate::state::TurnState;
use crate::stats::{ContextStat, RequestPurpose, RequestStat, StatEvent};
use crate::store::{EpochReason, EpochStart, Record, StoreError, Summary};
use crate::timing::Spend;
use crate::usage::add_request;

/// A summary taken in the background, while it runs and once it is done.
pub(super) enum Background {
    Running(Job),
    Ready(Ready),
}

/// A summary request running beside the turn.
pub(super) struct Job {
    covered: Cover,
    cancel: CancelToken,
    handle: tokio::task::JoinHandle<Summarized>,
}

/// A summary that is done, its usage counted.
pub(super) struct Ready {
    covered: Cover,
    outcome: Result<Written, ErrorData>,
}

/// What a summary request answered with.
#[derive(Debug, Clone)]
pub(super) struct Written {
    text: String,
    /// The response stopped at the output limit: the summary's end is
    /// missing.
    cut: bool,
}

/// What a summary was taken over: the first `messages` of the epoch's
/// history, which end with the item `through`. A summary is used only over
/// a history that still begins so.
#[derive(Debug, Clone)]
struct Cover {
    messages: usize,
    through: Option<ItemId>,
}

impl Cover {
    fn holds_for(&self, history: &History) -> bool {
        self.messages <= history.len() && history.end_of(self.messages) == self.through
    }
}

/// What a summary request came to.
pub(super) struct Summarized {
    model: RoleModel,
    role: ModelRole,
    outcome: Result<Written, ErrorData>,
    usage: Option<Usage>,
    /// The request's size in bytes, and the window of the model it went
    /// to: what a refusal as too long teaches the budget.
    bytes: u64,
    window: u64,
    /// The turn that asked for it.
    started_in: TurnId,
    /// How many attempts it took.
    attempts: u32,
}

/// Why a compaction could not finish.
enum Unfinished {
    Store(StoreError),
    /// The summary, with what it carries and what it replays, does not
    /// fit the new epoch.
    TooLarge(String),
}

/// The least a summary is asked to write, however little room the trigger
/// leaves: a summary shorter than that is of little use, and one that does
/// not fit fails as too large for the new epoch, where it is seen.
const SUMMARY_FLOOR: u64 = 512;

/// The date of a time, as the model is told it.
fn date_of(time: cortex_base::UtcTime) -> String {
    time.to_string().chars().take(10).collect()
}

/// Sends a summary request and reads its text.
///
/// A response that ends because the request did not fit is a refusal as
/// too long, as an error of that kind is: the main request's are read the
/// same way (`turn::decide`).
async fn summarize(
    client: Arc<dyn ModelClient>,
    request: ModelRequest,
    cancel: CancelToken,
) -> (Result<Written, ErrorData>, Option<Usage>, u32) {
    let mut stream = client.stream(request, cancel.child());
    let mut text = String::new();
    let mut usage = None;
    let mut attempts = 1;
    loop {
        let Some(event) = cancel.race(stream.next()).await else {
            let error = ErrorData::new(
                ErrorKind::Interrupted,
                "interrupted",
                "The summary was stopped.",
            );
            // What the provider said it used before the stop was used.
            return (Err(error), usage, attempts);
        };
        let Some(event) = event else {
            let error = ErrorData::new(
                ErrorKind::Transient,
                "incomplete_stream",
                "The summary's response ended before it was complete.",
            );
            return (Err(error), usage, attempts);
        };
        match event {
            ModelEvent::TextDelta { text: piece, .. } => text.push_str(&piece),
            // A new attempt begins: what came before it does not count.
            ModelEvent::Retry(_) => {
                text.clear();
                attempts += 1;
            }
            ModelEvent::Start { .. } => text.clear(),
            ModelEvent::Usage(running) => usage = Some(running),
            ModelEvent::End {
                usage: last, stop, ..
            } => {
                let outcome = if stop == StopReason::ContextOverflow {
                    Err(ErrorData::new(
                        ErrorKind::ContextOverflow,
                        "context_overflow",
                        "The request for a summary did not fit the model's context window.",
                    ))
                } else if text.trim().is_empty() {
                    Err(ErrorData::new(
                        ErrorKind::ModelStop,
                        "empty_summary",
                        "The model answered the request for a summary with no text.",
                    ))
                } else {
                    Ok(Written {
                        text,
                        cut: stop == StopReason::MaxOutput,
                    })
                };
                return (outcome, Some(last), attempts);
            }
            ModelEvent::Error(error) => return (Err(error.to_error_data()), usage, attempts),
            _ => {}
        }
    }
}

/// The stop reason a summary's outcome stands for, for its record.
fn stop_of(outcome: &Result<Written, ErrorData>) -> StopReason {
    match outcome {
        Ok(written) if written.cut => StopReason::MaxOutput,
        Ok(_) => StopReason::End,
        Err(error) if error.kind == ErrorKind::ContextOverflow => StopReason::ContextOverflow,
        Err(error) if error.kind == ErrorKind::Interrupted => StopReason::Interrupted,
        Err(_) => StopReason::Error,
    }
}

impl Session {
    /// The main role's model as the session is set now, for this request.
    pub(super) fn refresh_main(&mut self) {
        let inbox = self.shared.lock();
        self.model = inbox.models.main().clone();
        inbox
            .models
            .catalog_version()
            .clone_into(&mut self.catalog_version);
    }

    /// A new context epoch of the table of seen files: the model no longer
    /// has what it was shown, so the next read of any file is in full. The
    /// one place every new epoch reaches the table, a reload's included
    /// (`docs/architecture/07-sessions-and-context.md`, "What each part of
    /// a session's state follows").
    pub(super) fn new_file_epoch(&self) {
        self.env.workspace.files.begin_epoch();
    }

    fn root(&self) -> AbsPath {
        self.env.workspace.paths.root().clone()
    }

    /// Reads the instruction files and builds the system prompt.
    async fn build_prompt(&mut self) -> (Built, Discovered) {
        let (found, io, cap) = {
            let parts = &self.context.parts;
            (
                places(
                    &self.root(),
                    parts.user_dir.as_ref(),
                    &parts.settings.instruction_names,
                ),
                Arc::clone(&parts.instructions),
                parts.settings.instructions_cap,
            )
        };
        let discovered = run_blocking(&CancelToken::new(), move |_| discover(&*io, &found, cap))
            .await
            .unwrap_or_default();
        let parts = &self.context.parts;
        let built = build(&Sections {
            environment: Some(&parts.environment),
            instructions: &discovered.texts,
            // The project profile's source is wired in step 17's second
            // part.
            profile: None,
            append: parts.append.as_deref(),
            guided: self.model.variants.prompt_style == "guided",
        });
        (built, discovered)
    }

    /// Tells the user what the cap left out, or what could not be read.
    fn warn_instructions(&self, turn: &Turn, discovered: &Discovered) {
        for message in &discovered.warnings {
            self.notify(
                turn,
                Notice::Instructions(InstructionsNotice {
                    message: message.clone(),
                    files: data_names(&discovered.warned),
                }),
            );
        }
    }

    /// The start of an epoch with this prompt, for the main model now.
    fn epoch_start(
        &self,
        built: &Built,
        discovered: &Discovered,
        reason: EpochReason,
    ) -> EpochStart {
        let info = &self.model.entry.info;
        EpochStart {
            provider: info.provider.clone(),
            protocol: info.protocol.clone(),
            model: info.id.clone(),
            effort: self.model.effort.clone(),
            system: built.system.clone(),
            tools: self.context.tools.clone(),
            reason: Some(reason),
            base: Some(built.base),
            instructions: discovered.marks.clone(),
            profile: built.profile,
            altered: false,
            variants: Some(self.model.variants.clone()),
        }
    }

    /// The variants of the epoch in force: fixed for it, and recorded in
    /// its `epoch` entry. An epoch a build before step 16 wrote has none
    /// recorded, and takes the main model's.
    pub(super) fn epoch_variants(&self) -> cortex_llm::Variants {
        self.context
            .epoch
            .as_ref()
            .and_then(|epoch| epoch.variants.clone())
            .unwrap_or_else(|| self.model.variants.clone())
    }

    /// Records an epoch, then begins it. A summary taken in the background
    /// is of the old epoch's history, which only a change of model keeps.
    async fn begin_epoch(
        &mut self,
        turn: &Turn,
        epoch: EpochStart,
        history: Option<History>,
    ) -> Result<(), StoreError> {
        self.store
            .append(&turn.id, Record::Epoch(epoch.clone()))
            .await?;
        if epoch.reason != Some(EpochReason::Model) {
            self.drop_background();
            // A new history: the `fast` role may hold it.
            self.fast_refused = None;
        }
        self.context.begin(epoch, history);
        self.new_file_epoch();
        Ok(())
    }

    /// At the start of a turn, before its prompt is recorded: the epoch,
    /// and changed instruction files.
    pub(super) async fn context_turn_start(&mut self, turn: &Turn) -> Result<(), StoreError> {
        self.refresh_main();
        let mut begun = false;
        if self.context.epoch.is_none() || self.context.unchecked {
            let (built, discovered) = self.build_prompt().await;
            self.warn_instructions(turn, &discovered);
            // An epoch whose prompt the log does not hold as it was sent
            // (the filter masked something in it) cannot go on: its next
            // request would not begin as the earlier ones did.
            let goes_on = self.context.epoch.as_ref().is_some_and(|epoch| {
                !epoch.altered
                    && epoch.tools == self.context.tools
                    // Variants the user changed since apply from a new
                    // epoch: they are fixed for one.
                    && epoch
                        .variants
                        .as_ref()
                        .is_none_or(|variants| *variants == self.model.variants)
                    && match epoch.base {
                        Some(base) => base == built.base,
                        None => epoch.system == built.system,
                    }
            });
            if goes_on {
                // The prompt is the one the harness would build, but for
                // the instruction files: what changed in them is told as a
                // notice, below. A session read again on a later day is
                // told the date.
                self.context.unchecked = false;
                let today = date_of(self.clock.now());
                if self
                    .context
                    .date
                    .as_deref()
                    .is_some_and(|date| date != today)
                {
                    self.record_notice(
                        turn,
                        NoticeSource::Harness,
                        crate::context::date_text(&today),
                    )
                    .await?;
                    self.context.date = Some(today);
                }
            } else {
                let reason = if self.context.epoch.is_none() && self.context.history.len() == 0 {
                    EpochReason::Start
                } else {
                    EpochReason::Resume
                };
                let epoch = self.epoch_start(&built, &discovered, reason);
                self.begin_epoch(turn, epoch, None).await?;
                begun = true;
            }
        }
        if !begun {
            self.instruction_changes(turn).await?;
        }
        Ok(())
    }

    /// Tells the model of each instruction file that changed, appeared or
    /// went since it was given it.
    async fn instruction_changes(&mut self, turn: &Turn) -> Result<(), StoreError> {
        let root = self.root();
        let parts = &self.context.parts;
        let mut checked = places(
            &root,
            parts.user_dir.as_ref(),
            &parts.settings.instruction_names,
        );
        // For each place, the name as written when it differs from the
        // place's: tried first.
        let mut as_written: Vec<Option<AbsPath>> = vec![None; checked.len()];
        // The nested files the model was given.
        for shown in self.context.given.files.keys() {
            if checked.iter().any(|place| &place.shown == shown) {
                continue;
            }
            // The key is the name the model was shown, escaped: a name that
            // held a character that acts unseen leads to its file read back
            // (the independent review of step 18's fixes). A name that held
            // `\n` as two characters is the key as it is, which is tried
            // first, on the blocking pool, since the loop looks at no file.
            let unescaped = cortex_base::unescape(shown);
            let read_back = root.join(&unescaped);
            let written = match unescaped {
                std::borrow::Cow::Owned(_) => root.join(shown).ok(),
                std::borrow::Cow::Borrowed(_) => None,
            };
            // Read back it may not join (`\u{0}` as six characters): then
            // only as written.
            let (path, written) = match (read_back, written) {
                (Ok(path), written) => (path, written),
                (Err(_), Some(written)) => (written, None),
                (Err(_), None) => continue,
            };
            as_written.push(written);
            checked.push(Place {
                dir: path.parent().unwrap_or_else(|| root.clone()),
                path,
                shown: shown.clone(),
                user: false,
            });
        }
        let io = Arc::clone(&parts.instructions);
        let cap = parts.settings.instructions_cap;
        let read = run_blocking(&CancelToken::new(), move |_| {
            checked
                .into_iter()
                .zip(as_written)
                .map(|(place, as_written)| {
                    if let Some(path) = as_written {
                        let written = Place {
                            dir: path.parent().unwrap_or_else(|| place.dir.clone()),
                            path,
                            ..place.clone()
                        };
                        if let Ok(Some(found)) = read_one(&*io, &written, cap) {
                            return (written, Ok(Some(found)));
                        }
                    }
                    let found = read_one(&*io, &place, cap);
                    (place, found)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();

        let mut changes = Vec::new();
        let mut in_dir: BTreeMap<String, BTreeSet<ContentHash>> = BTreeMap::new();
        for (place, outcome) in read {
            let given = self.context.given.files.get(&place.shown).copied();
            match outcome {
                // Said nothing about: the next turn looks again.
                Err(_) => {}
                Ok(Some(found)) => {
                    // A second name of a file of the same directory.
                    if !in_dir
                        .entry(place.dir.key().to_owned())
                        .or_default()
                        .insert(found.hash())
                    {
                        continue;
                    }
                    match given {
                        Some(Some(had)) if had == found.hash() => {}
                        Some(Some(_)) => changes.push(Change::Changed(found)),
                        _ => changes.push(Change::New(found)),
                    }
                }
                Ok(None) => {
                    if let Some(Some(_)) = given {
                        changes.push(Change::Removed(place.shown.clone()));
                    }
                }
            }
        }
        self.tell_changes(turn, changes).await
    }

    /// Appends a notice for each change, and tells front ends.
    async fn tell_changes(&mut self, turn: &Turn, changes: Vec<Change>) -> Result<(), StoreError> {
        let cap = self.context.parts.settings.instructions_cap;
        for change in changes {
            let text = notice_text(&change, cap);
            let (shown, hash, said) = match &change {
                Change::Changed(found) => (
                    found.place.shown.clone(),
                    Some(found.hash()),
                    "changed; the model was given the new version",
                ),
                Change::New(found) => (
                    found.place.shown.clone(),
                    Some(found.hash()),
                    "is new; the model was given it",
                ),
                Change::Removed(shown) => (shown.clone(), None, "was removed; the model was told"),
                Change::Nested { found, .. } => (
                    found.place.shown.clone(),
                    Some(found.hash()),
                    "applies to files a tool touched; the model was given it",
                ),
            };
            self.record_notice(turn, NoticeSource::Instructions, text)
                .await?;
            self.context.given.files.insert(shown.clone(), hash);
            self.notify(
                turn,
                Notice::Instructions(InstructionsNotice {
                    message: format!("The instruction file {shown} {said}."),
                    files: data_names(std::slice::from_ref(&shown)),
                }),
            );
        }
        Ok(())
    }

    /// Before each request, at a safe boundary: a change of the main
    /// role's model starts an epoch, and instruction files under
    /// directories a tool has first touched are given.
    pub(super) async fn context_before_request(&mut self, turn: &Turn) -> Result<(), StoreError> {
        if let Some(epoch) = &self.context.epoch {
            let info = &self.model.entry.info;
            if epoch.provider != info.provider
                || epoch.protocol != info.protocol
                || epoch.model != info.id
                || epoch.effort != self.model.effort
            {
                // The prompt and the tools stay exactly, so the prompt's
                // style stays; how tools are declared and edited is the new
                // model's, which may not take the old one's.
                let variants = epoch.variants.clone().map(|kept| cortex_llm::Variants {
                    prompt_style: kept.prompt_style,
                    ..self.model.variants.clone()
                });
                let next = EpochStart {
                    provider: info.provider.clone(),
                    protocol: info.protocol.clone(),
                    model: info.id.clone(),
                    effort: self.model.effort.clone(),
                    reason: Some(EpochReason::Model),
                    variants,
                    ..epoch.clone()
                };
                self.begin_epoch(turn, next, None).await?;
            }
        }
        self.nested_instructions(turn).await
    }

    /// Gives the model, once, each instruction file of a directory between
    /// the workspace root and a file a tool has read or written: not a file
    /// only a command changed, which may be any number of them in a
    /// vendored tree. What one boundary gives is held to the cap on
    /// instruction files, and the user is told of what is left out (since
    /// the review of step 15, its sixth note).
    async fn nested_instructions(&mut self, turn: &Turn) -> Result<(), StoreError> {
        let root = self.root();
        let mut dirs: Vec<AbsPath> = Vec::new();
        for file in self.env.workspace.files.touched() {
            for dir in nested_dirs(&root, &file) {
                let key = crate::context::shown_in(&root, &dir);
                if self.context.given.looked.insert(key) {
                    dirs.push(dir);
                }
            }
        }
        if dirs.is_empty() {
            return Ok(());
        }
        let names = self.context.parts.settings.instruction_names.clone();
        let mut candidates = Vec::new();
        for dir in &dirs {
            for name in &names {
                if let Ok(path) = dir.join(name) {
                    candidates.push(Place {
                        shown: crate::context::shown_in(&root, &path),
                        path,
                        user: false,
                        dir: dir.clone(),
                    });
                }
            }
        }
        let io = Arc::clone(&self.context.parts.instructions);
        let cap = self.context.parts.settings.instructions_cap;
        let (read, unread) = run_blocking(&CancelToken::new(), move |_| {
            let mut read: Vec<Found> = Vec::new();
            let mut unread: Vec<(String, String)> = Vec::new();
            for place in candidates {
                match read_one(&*io, &place, cap) {
                    Ok(Some(found)) => read.push(found),
                    Ok(None) => {}
                    Err(error) => unread.push((place.shown, error)),
                }
            }
            (read, unread)
        })
        .await
        .unwrap_or_default();
        // A directory is looked at once, so each is told once.
        for (shown, error) in unread {
            self.notify(
                turn,
                Notice::Instructions(InstructionsNotice {
                    message: format!("The instruction file {shown} could not be read: {error}"),
                    files: data_names(std::slice::from_ref(&shown)),
                }),
            );
        }
        let mut changes = Vec::new();
        let mut in_dir: BTreeMap<String, BTreeSet<ContentHash>> = BTreeMap::new();
        // Each file is already cut to the cap; together they are held to it.
        let mut room = cap;
        let mut left_out = Vec::new();
        for found in read {
            if self.context.given.files.contains_key(&found.place.shown) {
                continue;
            }
            if !in_dir
                .entry(found.place.dir.key().to_owned())
                .or_default()
                .insert(found.hash())
            {
                continue;
            }
            if found.text.len() > room {
                // Its directory is looked at again at the next boundary,
                // which has room of its own: marked as looked, it would not
                // be given in this epoch at all (found by the review of
                // these fixes).
                self.context
                    .given
                    .looked
                    .remove(&crate::context::shown_in(&root, &found.place.dir));
                left_out.push(found.place.shown.clone());
                continue;
            }
            room -= found.text.len();
            let dir = crate::context::shown_in(&root, &found.place.dir);
            changes.push(Change::Nested { found, dir });
        }
        if !left_out.is_empty() {
            self.notify(
                turn,
                Notice::Instructions(InstructionsNotice {
                    message: format!(
                        "The instruction files under the directories a tool touched hold more \
                         than the {} that are given at once, so {} was left out for now. \
                         `context.instructions_cap` sets the limit.",
                        crate::context::cap_text(cap),
                        left_out.join(", ")
                    ),
                    files: data_names(&left_out),
                }),
            );
        }
        self.tell_changes(turn, changes).await
    }

    /// The context window of a role's model: what its local server
    /// reported, or the catalog's.
    pub(super) fn window_of(&self, model: &RoleModel) -> u64 {
        let info = &model.entry.info;
        self.shared
            .lock()
            .windows
            .get(&(info.provider.clone(), info.id.clone()))
            .copied()
            .unwrap_or(info.context_window)
    }

    /// The window of a role's model, and the most a request of it may hold.
    pub(super) fn limit(&self, model: &RoleModel) -> (u64, u64) {
        let window = self.window_of(model);
        let max_output = model.max_output.unwrap_or(model.entry.info.max_output);
        (window, limit_of(window, max_output))
    }

    /// The estimate of the next request, in tokens.
    pub(super) fn estimate(&self) -> u64 {
        self.context.budget.estimate(self.context.content_bytes())
    }

    /// Where the next request stands against the thresholds.
    pub(super) fn pressure(&self) -> Pressure {
        let (_, limit) = self.limit(&self.model);
        self.pressure_of(self.estimate(), limit)
    }

    /// Where a request of `estimate` tokens stands, against `limit`.
    fn pressure_of(&self, estimate: u64, limit: u64) -> Pressure {
        let prepare = fraction_of(self.context.parts.settings.prepare_at, limit);
        let (trigger, _) = self.trigger(limit);
        pressure(estimate, prepare, trigger)
    }

    /// The epoch's prompt and tools, in tokens: what every request of it
    /// holds, and a new epoch's will too.
    fn fixed_tokens(&self) -> u64 {
        self.context.budget.tokens(
            self.context
                .content_bytes()
                .saturating_sub(self.context.history.bytes()),
        )
    }

    /// The trigger in tokens: `compact_at` of `limit`, and never below what
    /// a compaction needs, the epoch's prompt and tools and room for a
    /// summary, up to the limit itself; and whether the setting was raised
    /// to that. Below it every compaction is too large for the new epoch,
    /// and the session stops at the trigger (the review of step 15, its
    /// fifth note). A low setting on a large window, which leaves that
    /// room, is the user's to choose, and stands.
    fn trigger(&self, limit: u64) -> (u64, bool) {
        let asked = fraction_of(self.context.parts.settings.compact_at, limit);
        let needed = self
            .fixed_tokens()
            .saturating_add(summary_room(limit))
            .min(limit);
        if asked < needed {
            (needed, true)
        } else {
            (asked, false)
        }
    }

    /// The most of the recent conversation a compaction replays, in
    /// tokens: `keep_recent_tokens`, and no more than half of what the
    /// trigger leaves beyond the prompt and the tools, so that the summary
    /// and what follows have the other half. Past that the replay alone
    /// fills the new epoch (the review of step 15, its fifth note).
    fn keep_room(&self) -> (u64, bool) {
        let keep = self.context.parts.settings.keep_recent_tokens;
        let (_, limit) = self.limit(&self.model);
        let (trigger, _) = self.trigger(limit);
        let room = trigger.saturating_sub(self.fixed_tokens()) / 2;
        if keep > room {
            (room, true)
        } else {
            (keep, false)
        }
    }

    /// Tells the user, once in a session, of a setting the loop holds to
    /// what a compaction needs.
    pub(super) fn tell_clamped(&mut self, turn: &Turn) {
        let (_, limit) = self.limit(&self.model);
        let settings = &self.context.parts.settings;
        let (compact_at, keep) = (settings.compact_at, settings.keep_recent_tokens);
        let (trigger, raised) = self.trigger(limit);
        if raised && !self.told_trigger {
            self.told_trigger = true;
            let fixed = self.fixed_tokens();
            let message = format!(
                "`context.compact_at` ({compact_at}) puts the trigger at {} of the {limit} \
                 tokens a request may hold, below what a compaction needs: the prompt and the \
                 tools ({fixed}) and room for a summary ({}). The conversation is compacted \
                 past {trigger} tokens instead.",
                fraction_of(compact_at, limit),
                summary_room(limit)
            );
            self.notify(turn, Notice::Info(InfoNotice { message }));
        }
        let (kept, capped) = self.keep_room();
        if capped && !self.told_keep {
            self.told_keep = true;
            let message = format!(
                "`context.keep_recent_tokens` ({keep}) is more than a compaction can replay \
                 here: it keeps at most {kept} tokens of the most recent conversation, half of \
                 what the trigger leaves once the prompt and the tools are counted."
            );
            self.notify(turn, Notice::Info(InfoNotice { message }));
        }
    }

    /// The budget a role's requests are estimated with: one of each
    /// role's own (since step 16). The main model's is the context's. A
    /// utility role has one once a request of its has taught it something,
    /// a count or a refusal as too long; until then it is estimated at the
    /// main model's ratio, the best guess there is. So a refusal by the
    /// `fast` model marks the `fast` budget alone, and the main model's
    /// retry of the summary is sent as its own estimate allows.
    fn budget_for(&self, role: &ModelRole) -> &Budget {
        if *role == ModelRole::Main {
            return &self.context.budget;
        }
        self.role_budgets.get(role).unwrap_or(&self.context.budget)
    }

    /// See [`budget_for`](Self::budget_for). A utility role's own budget is
    /// made here, from the main model's ratio, the first time it learns.
    fn budget_for_mut(&mut self, role: &ModelRole) -> &mut Budget {
        if *role == ModelRole::Main {
            return &mut self.context.budget;
        }
        let seed = self.context.budget.ratio();
        self.role_budgets
            .entry(role.clone())
            .or_insert_with(|| Budget::seeded(seed))
    }

    /// The model a summary is asked of: the `fast` role's, when it has one
    /// whose window holds the epoch and it has not refused a summary of
    /// this epoch as too long, and otherwise the main model.
    fn summary_model(&self) -> (ModelRole, RoleModel) {
        let (role, model) = {
            let inbox = self.shared.lock();
            let (role, model) = inbox.models.for_call(UtilityCall::Summary);
            (role, model.clone())
        };
        let fast = (role != ModelRole::Main).then_some(model);
        if let Some(fast) = fast
            && self.fast_refused != Some(self.context.epoch_number)
        {
            let estimate = self
                .budget_for(&ModelRole::Fast)
                .estimate(self.context.content_bytes());
            let (_, limit, _) = self.summary_limit(&fast);
            if estimate <= limit {
                return (ModelRole::Fast, fast);
            }
        }
        (ModelRole::Main, self.model.clone())
    }

    /// For a summary asked of `model`: its window, the most the request may
    /// hold, and the output it is given.
    ///
    /// The output is `summary_max_output` **on top of the role's reasoning
    /// budget**, where the role has one, up to the model's maximum. The
    /// budget is part of the output on every protocol, and Anthropic's
    /// refuses a budget that is not below the request's `max_tokens` (the
    /// review of step 15 found every summary of a budget of 16,384 or more
    /// refused). The budget itself is sent as the epoch's requests send it:
    /// a change to the thinking parameters would lose the epoch's cache.
    /// Where the model's maximum cannot hold both, the summary has what the
    /// maximum leaves; a summary cut off there is marked as cut.
    ///
    /// **The summary's own share is at most half of what the trigger leaves
    /// beyond the prompt and the tools**, the other half being the turns a
    /// compaction replays ([`keep_room`](Self::keep_room)): a summary that
    /// wrote more could not fit the new epoch, and a trigger set low would
    /// make every compaction too large for it (found by the review of these
    /// fixes). A summary cut off there is used, and marked as cut.
    ///
    /// **A request with a reasoning budget keeps its whole output in
    /// reserve**, not the capped reserve of [`limit_of`]: Anthropic's models
    /// before Claude 4.5 refuse a request with thinking whose input and
    /// `max_tokens` together pass the window, with an error that is not an
    /// overflow (the review of these fixes; the main request's own case is
    /// `docs/outstanding/a-reasoning-budget-request-past-the-window-on-older-models.md`).
    fn summary_limit(&self, model: &RoleModel) -> (u64, u64, u64) {
        let window = self.window_of(model);
        let (_, main_limit) = self.limit(&self.model);
        let (trigger, _) = self.trigger(main_limit);
        let share = (trigger.saturating_sub(self.fixed_tokens()) / 2).max(SUMMARY_FLOOR);
        let cap = self.context.parts.settings.summary_max_output.min(share);
        let budget = model.reasoning.budget_tokens;
        let wanted = budget.map_or(cap, |budget| budget.saturating_add(cap));
        let max_output = wanted.min(model.entry.info.max_output);
        let limit = if budget.is_some() {
            window
                .saturating_sub(max_output)
                .saturating_sub(window / 50)
        } else {
            limit_of(window, max_output)
        };
        (window, limit, max_output)
    }

    /// A summary request over the first `covered` messages of the epoch:
    /// the epoch's own prefix, so that a summary on the same model reads
    /// the epoch's cache, then the instructions.
    ///
    /// After a provider refused a request as too long (the turn's own, or a
    /// summary's) and before it counted one again, the estimate is known to
    /// be low, so the request is always cut, to half the limit.
    fn summary_request(
        &self,
        model: &RoleModel,
        role: &ModelRole,
        covered: usize,
        focus: Option<&str>,
    ) -> ModelRequest {
        let assembled = self.context.assemble();
        let mut request = ModelRequest::new(Arc::clone(&model.entry), role.clone());
        request.system = assembled.system;
        request.tools = assembled.tools;
        request.cache = assembled.cache;
        request.reasoning = model.reasoning.clone();
        request.speed = self.shared.lock().speeds.of(role);
        // The main model's summary reads the epoch's cache, so it is sent
        // as the epoch's requests are; another model's as it is addressed.
        request.variants = if *role == ModelRole::Main {
            self.epoch_variants()
        } else {
            model.variants.clone()
        };
        let (_, limit, max_output) = self.summary_limit(model);
        request.max_output = Some(max_output);
        let messages = &self.context.history.messages()[..covered];
        let budget = self.budget_for(role);
        let whole = budget.tokens(self.context.content_bytes());
        // What the request holds besides the messages: the prompt, the
        // tools and the instructions. The messages get the rest.
        let fixed = budget.tokens(
            self.context
                .content_bytes()
                .saturating_sub(self.context.history.bytes())
                + crate::context::SUMMARY_INSTRUCTIONS.len() as u64,
        );
        request.messages = if budget.refused() {
            self.trimmed(messages, (limit / 2).saturating_sub(fixed), budget)
        } else if whole <= limit {
            messages.to_vec()
        } else {
            self.trimmed(
                messages,
                (limit.saturating_mul(9) / 10).saturating_sub(fixed),
                budget,
            )
        };
        request.messages.push(instructions_message(focus));
        request
    }

    /// The newest messages of `messages` that fit `room`, whole turns at a
    /// time and without their reasoning, after the epoch's summary if it
    /// begins with one: for a summary request the summarising model could
    /// not take whole (the provider said the epoch does not fit).
    fn trimmed(&self, messages: &[Message], room: u64, budget: &Budget) -> Vec<Message> {
        let starts_with_summary = self.context.history.starts_with_summary();
        let first = usize::from(starts_with_summary).min(messages.len());
        // A round begins at any message that is not a tool's results: the
        // cut is made only there, so that no call is left without them.
        let begins_round = |index: usize| messages[index].role != Role::Tool;
        let mut from = messages.len();
        for index in (first..messages.len()).rev() {
            let tokens = budget
                .tokens(self.context.history.bytes_from(index))
                .saturating_sub(budget.tokens(self.context.history.bytes_from(messages.len())));
            if tokens > room {
                break;
            }
            if begins_round(index) {
                from = index;
            }
        }
        // Nothing fitted: the last round is kept all the same, so that the
        // summary is of the work, and a request still too large fails
        // where it can be seen.
        if from == messages.len() {
            from = (first..messages.len())
                .rev()
                .find(|&index| begins_round(index))
                .unwrap_or(first);
        }
        let mut out = Vec::new();
        if starts_with_summary {
            out.push(messages[0].clone());
        }
        if from > first {
            out.push(Message {
                role: Role::User,
                parts: vec![Part::text(format!(
                    "{}The earlier part of this conversation is left out here: it did not fit.",
                    crate::context::NOTICE_LEAD
                ))],
                origin: None,
            });
        }
        out.extend(messages[from..].iter().map(project));
        out
    }

    /// Starts a summary in the background, when the settings allow one and
    /// none is under way or ready.
    pub(super) fn start_background(&mut self, turn: &Turn) {
        let settings = &self.context.parts.settings;
        if !settings.auto_compact
            || !settings.background
            || self.background.is_some()
            || self.background_failed == Some(self.context.epoch_number)
            || self.context.history.len() == 0
        {
            return;
        }
        // A summary is a request, and one prepared ahead of need is not
        // worth stopping the session for: with no request left, none.
        if self.ledger.take_request().is_err() {
            return;
        }
        let covered = self.context.history.len();
        let (role, model) = self.summary_model();
        let request = self.summary_request(&model, &role, covered, None);
        let bytes = request_bytes(&request);
        let (window, _, _) = self.summary_limit(&model);
        let cancel = self.shared.cancel.child();
        let token = cancel.clone();
        let client = Arc::clone(&model.client);
        let started_in = turn.id.clone();
        let span = tracing::info_span!(
            "request",
            role = %role,
            provider = %model.entry.info.provider,
            model = %model.entry.info.id,
            attempts = tracing::field::Empty,
        );
        let handle = tokio::spawn(tracing::Instrument::instrument(
            async move {
                let (outcome, usage, attempts) = summarize(client, request, token).await;
                tracing::Span::current().record("attempts", attempts);
                Summarized {
                    model,
                    role,
                    outcome,
                    usage,
                    bytes,
                    window,
                    started_in,
                    attempts,
                }
            },
            span,
        ));
        self.background = Some(Background::Running(Job {
            covered: Cover {
                messages: covered,
                through: self.context.history.end_of(covered),
            },
            cancel,
            handle,
        }));
        self.notify(
            turn,
            Notice::Compaction(CompactionNotice {
                message: "Preparing a summary of the conversation in the background.".to_owned(),
                state: CompactionState::Started,
                background: true,
                before_tokens: None,
                after_tokens: None,
            }),
        );
    }

    /// A summary that finished in the background has its usage counted
    /// now, and waits as ready.
    pub(super) async fn collect_background(&mut self, turn: &mut Turn) -> Result<(), StoreError> {
        // A summary given up was paid for, as far as it went: one that
        // finished, and one stopped part-way, whose task returns what the
        // provider had said it used.
        // One the log cannot take does not keep the others from being
        // counted: the first failure is returned once all are.
        let mut failed = None;
        for handle in std::mem::take(&mut self.given_up) {
            if let Ok(summarized) = handle.await
                && let Err(error) = self.out_of_band(turn, &summarized).await
            {
                failed.get_or_insert(error);
            }
        }
        if let Some(error) = failed {
            return Err(error);
        }
        let finished =
            matches!(&self.background, Some(Background::Running(job)) if job.handle.is_finished());
        if !finished {
            return Ok(());
        }
        let Some(Background::Running(job)) = self.background.take() else {
            return Ok(());
        };
        let covered = job.covered.clone();
        let summarized = job.handle.await;
        let outcome = self.counted(turn, summarized).await?;
        if outcome.is_err() {
            self.background_failed = Some(self.context.epoch_number);
        }
        self.background = Some(Background::Ready(Ready { covered, outcome }));
        Ok(())
    }

    /// Counts what a summary request used, and records it. A request the
    /// provider refused as too long teaches the budget, as the turn's own
    /// request's refusal does (`main_refused`), and a refusal by the `fast`
    /// role's model sends the epoch's summaries to the main model.
    async fn counted(
        &mut self,
        turn: &mut Turn,
        summarized: Result<Summarized, tokio::task::JoinError>,
    ) -> Result<Result<Written, ErrorData>, StoreError> {
        let summarized = match summarized {
            Ok(summarized) => summarized,
            Err(_) => {
                self.summary_role = ModelRole::Main;
                return Ok(Err(ErrorData::new(
                    ErrorKind::Internal,
                    "summary_failed",
                    "The summary's task ended before it finished.",
                )));
            }
        };
        self.summary_role = summarized.role.clone();
        let record = self.out_of_band(turn, &summarized).await?;
        match &summarized.outcome {
            Err(error) if error.kind == ErrorKind::ContextOverflow => {
                self.budget_for_mut(&summarized.role)
                    .overflowed(summarized.bytes, summarized.window);
            }
            // A count teaches the role's ratio, as the main request's does
            // (the context's `counted`): the bytes sent against the input
            // the provider counted. The main model's estimate of its next
            // request still goes on from its own last count.
            _ => {
                if let Some(record) = &record {
                    let tokens = &record.tokens;
                    let input = tokens.input + tokens.cache_read + tokens.cache_write;
                    self.budget_for_mut(&summarized.role)
                        .learned(summarized.bytes, input);
                }
            }
        }
        // A `fast` model that refused or failed is not asked again in the
        // epoch: its summaries go to the main model.
        if let Err(error) = &summarized.outcome
            && error.kind != ErrorKind::Interrupted
            && summarized.role != ModelRole::Main
        {
            self.fast_refused = Some(self.context.epoch_number);
        }
        Ok(summarized.outcome)
    }

    /// The provider refused the turn's request as too long: the budget
    /// learns what that shows of the ratio, whether or not a compaction
    /// follows.
    pub(super) fn main_refused(&mut self) {
        let (window, _) = self.limit(&self.model);
        let sent = self.context.sent_bytes;
        self.budget_for_mut(&ModelRole::Main)
            .overflowed(sent, window);
    }

    /// Counts what a summary request used, in the session's usage, the
    /// turn's, and every account of the ledger, records it as a `usage`
    /// entry, and sends its statistics event.
    ///
    /// **The turn that counts it owns it** (settled in step 16): a summary
    /// taken in the background is counted at the first boundary after its
    /// request ended, in the turn that is running then, which may not be
    /// the one that started it. That turn's usage, its `turn.completed`,
    /// its account of the ledger and the session log's `usage` entry all
    /// have it, and so does the statistics event, so that the four agree;
    /// the event names the turn that started it as `started_in`.
    async fn out_of_band(
        &mut self,
        turn: &mut Turn,
        summarized: &Summarized,
    ) -> Result<Option<RequestUsage>, StoreError> {
        let model = &summarized.model;
        let record = summarized
            .usage
            .clone()
            .map(|usage| self.usage_record_as(model, summarized.role.clone(), usage, None));
        if let Some(record) = &record {
            self.usage.add(record);
            self.ledger.charge(record);
            add_request(&mut turn.usage, record);
            self.shared.lock().usage = self.usage.snapshot();
            self.store
                .append(
                    &turn.id,
                    Record::Usage {
                        purpose: "compaction".to_owned(),
                        usage: record.clone(),
                    },
                )
                .await?;
            self.tell_usage(turn, model, None, true);
        }
        self.summary_stat(&turn.id, summarized, record.clone());
        Ok(record)
    }

    /// The statistics event of a summary request.
    fn summary_stat(&self, turn: &TurnId, summarized: &Summarized, usage: Option<RequestUsage>) {
        let info = &summarized.model.entry.info;
        let variants = if summarized.role == ModelRole::Main {
            self.epoch_variants()
        } else {
            summarized.model.variants.clone()
        };
        self.stats.record(StatEvent::Request(RequestStat {
            session: self.shared.info.id.clone(),
            turn: turn.clone(),
            at: self.clock.now(),
            provider: info.provider.clone(),
            model: info.id.clone(),
            role: summarized.role.clone(),
            purpose: RequestPurpose::Summary,
            started_in: (summarized.started_in != *turn).then(|| summarized.started_in.clone()),
            variants,
            effort: summarized.model.effort.clone(),
            usage,
            time: cortex_protocol::usage::ModelTime::default(),
            stop: stop_of(&summarized.outcome),
            error: summarized.outcome.as_ref().err().cloned(),
            retries: summarized.attempts.saturating_sub(1),
            charged_to: self.ledger.charged_to(),
        }));
    }

    /// When the session's loop ends: a summary that is still running is
    /// stopped, and every summary no boundary counted is counted now, in
    /// the last turn that ran. It was spent. Nothing is written to the log
    /// for it, which a turn that has ended takes no more of; the ledger and
    /// the statistics have it.
    pub(super) async fn count_left_summaries(&mut self) {
        if let Some(Background::Running(job)) = self.background.take() {
            job.cancel.cancel();
            self.given_up.push(job.handle);
        }
        let Some(turn) = self.last_turn.clone() else {
            return;
        };
        for handle in std::mem::take(&mut self.given_up) {
            if let Ok(summarized) = handle.await {
                let record = summarized.usage.clone().map(|usage| {
                    self.usage_record_as(&summarized.model, summarized.role.clone(), usage, None)
                });
                if let Some(record) = &record {
                    self.usage.add(record);
                    self.ledger.charge(record);
                    self.shared.lock().usage = self.usage.snapshot();
                }
                self.summary_stat(&turn, &summarized, record);
            }
        }
    }

    /// Stops a summary that runs in the background, and forgets one that
    /// is ready: the history it was taken over is not the session's any
    /// more.
    pub(super) fn drop_background(&mut self) {
        if let Some(Background::Running(job)) = self.background.take() {
            self.give_up(job);
        }
        self.background_failed = None;
    }

    /// Stops a summary's request, and keeps its task, to be counted at the
    /// next boundary: what it used was spent, whether it finished or was
    /// stopped part-way (since step 16; before, a stopped one was counted
    /// nowhere). A stopped task returns at once with what the provider had
    /// said it used.
    fn give_up(&mut self, job: Job) {
        if !job.handle.is_finished() {
            job.cancel.cancel();
        }
        self.given_up.push(job.handle);
    }

    /// Stops a summary that runs in the background: the turn was
    /// interrupted, and an interrupt means stop. One that is ready, or whose
    /// request has finished, stays.
    pub(super) fn stop_background(&mut self) {
        if matches!(&self.background, Some(Background::Running(job)) if !job.handle.is_finished()) {
            self.drop_background();
        }
    }

    /// How many messages a compaction made now summarises: all of them, or
    /// with `keep_recent_tokens`, all but the most recent turns, never
    /// leaving a round's results without their calls.
    fn covered_now(&self) -> usize {
        let history = &self.context.history;
        let (keep, _) = self.keep_room();
        let count = history.len();
        if keep == 0 {
            return count;
        }
        let messages = history.messages();
        let mut covered = count;
        for index in (1..count).rev() {
            if self.context.budget.tokens(history.bytes_from(index)) > keep {
                break;
            }
            if messages[index].role != Role::Tool {
                covered = index;
            }
        }
        covered
    }

    /// `Compacting`: the summary, then the new epoch.
    ///
    /// **A summary request the provider refuses as too long** has taught
    /// the budget by the time it is seen here ([`counted`](Self::counted)),
    /// and is answered by one more try: cut to half the limit at the ratio
    /// the refusal showed, and on the main model when the `fast` role's
    /// refused. A second refusal in one compaction, a background summary's
    /// included, ends the turn with `compaction_failed`, and what the
    /// budget learned stays for the next turn: so the requests of one
    /// compaction are bounded, and the next turn's summary is cut.
    #[expect(
        clippy::too_many_lines,
        reason = "one compaction from the summary to the new epoch, each way it can end in its place"
    )]
    pub(super) async fn compacting(&mut self, turn: &mut Turn, prep: Prep) -> Phase {
        self.enter(TurnState::Compacting);
        self.manual = false;
        let focus = self
            .manual_focus
            .take()
            .filter(|focus| !focus.trim().is_empty());
        let notice = |state, message: String, before: Option<u64>, after: Option<u64>| {
            Notice::Compaction(CompactionNotice {
                message,
                state,
                background: false,
                before_tokens: before,
                after_tokens: after,
            })
        };
        if self.context.epoch.is_none() || self.context.history.len() == 0 {
            let message = "There is nothing to compact yet.".to_owned();
            self.notify(
                turn,
                notice(CompactionState::Failed, message.clone(), None, None),
            );
            if prep.only {
                return Phase::Finishing(End::completed());
            }
            return Phase::Finishing(End::failed(ErrorData::new(
                ErrorKind::ContextOverflow,
                "context_overflow",
                message,
            )));
        }
        // An overflow of the turn's request has taught the budget already
        // (`main_refused`), so the summary request below is cut.
        let before = self.estimate();
        self.notify(
            turn,
            notice(
                CompactionState::Started,
                "Compacting the conversation.".to_owned(),
                None,
                None,
            ),
        );
        if let Err(error) = self.collect_background(turn).await {
            return Phase::Finishing(End::store(&error));
        }
        // A summary prepared in the background is used when it is of the
        // history the session has now, and the user asked for no focus.
        let usable = |cover: &Cover| cover.holds_for(&self.context.history);
        let background = match self.background.take() {
            Some(Background::Ready(ready)) if focus.is_none() && usable(&ready.covered) => {
                Some((ready.covered, ready.outcome))
            }
            Some(Background::Running(job)) if focus.is_none() && usable(&job.covered) => {
                // The one case in which compaction stops the session: the
                // trigger came before the summary was ready.
                let covered = job.covered.clone();
                let cancel = job.cancel.clone();
                self.times.enter(self.clock.monotonic(), Spend::Summary);
                let mut handle = job.handle;
                let joined = turn.cancel.race(&mut handle).await;
                self.times.enter(self.clock.monotonic(), Spend::Harness);
                match joined {
                    None => {
                        // Stopped, and kept: what it used is counted at the
                        // next boundary, or when the loop ends (found by
                        // the review of step 16).
                        cancel.cancel();
                        self.given_up.push(handle);
                        return Phase::Interrupted(None);
                    }
                    Some(joined) => match self.counted(turn, joined).await {
                        Ok(outcome) => Some((covered, outcome)),
                        Err(error) => return Phase::Finishing(End::store(&error)),
                    },
                }
            }
            Some(Background::Running(job)) => {
                self.give_up(job);
                None
            }
            _ => None,
        };
        let mut attempt = background.map(|(covered, outcome)| (covered, outcome, true));
        // The summary requests refused as too long in this compaction.
        let mut refused = 0u32;
        // Whether the main model has been asked after the `fast` role's
        // failed, which is done once.
        let mut fast_retried = false;
        loop {
            let (covered, outcome, from_background) = match attempt.take() {
                Some(taken) => taken,
                None => {
                    let messages = self.covered_now();
                    let cover = Cover {
                        messages,
                        through: self.context.history.end_of(messages),
                    };
                    let (role, model) = self.summary_model();
                    // A summary is a request: the ledger is asked first.
                    self.ledger.catch_up().await;
                    if let Err(reached) = self.ledger.take_request() {
                        self.notify(
                            turn,
                            notice(
                                CompactionState::Failed,
                                format!(
                                    "The conversation was not compacted: {}",
                                    reached.message()
                                ),
                                None,
                                None,
                            ),
                        );
                        return self.limit_reached(turn, &reached).await;
                    }
                    let request = self.summary_request(&model, &role, messages, focus.as_deref());
                    let bytes = request_bytes(&request);
                    let (window, _, _) = self.summary_limit(&model);
                    let client = Arc::clone(&model.client);
                    let span = tracing::info_span!(
                        "request",
                        role = %role,
                        provider = %model.entry.info.provider,
                        model = %model.entry.info.id,
                        attempts = tracing::field::Empty,
                    );
                    self.times.enter(self.clock.monotonic(), Spend::Summary);
                    // The summary's own token: an interrupt stops it, and
                    // what it used is still counted.
                    let summary = summarize(client, request, turn.cancel.child());
                    let (outcome, usage, attempts) =
                        tracing::Instrument::instrument(summary, span.clone()).await;
                    span.record("attempts", attempts);
                    self.times.enter(self.clock.monotonic(), Spend::Harness);
                    let summarized = Summarized {
                        model,
                        role,
                        outcome,
                        usage,
                        bytes,
                        window,
                        started_in: turn.id.clone(),
                        attempts,
                    };
                    if turn.cancel.is_cancelled() {
                        // Counted, then the turn stops.
                        if let Err(error) = self.counted(turn, Ok(summarized)).await {
                            return Phase::Finishing(End::store(&error));
                        }
                        return Phase::Interrupted(None);
                    }
                    match self.counted(turn, Ok(summarized)).await {
                        Ok(outcome) => (cover, outcome, false),
                        Err(error) => return Phase::Finishing(End::store(&error)),
                    }
                }
            };
            // Whether the provider refused the summary request as too long:
            // not a summary too large for the new epoch, which is the
            // harness's own finding.
            let mut refusal = false;
            let failure = match outcome {
                Ok(written) => match self.finish_compaction(turn, &written, &covered).await {
                    Ok(after) => {
                        let mut message = format!(
                            "The conversation was compacted: about {before} tokens became {after}."
                        );
                        if written.cut {
                            message.push_str(
                                " The summary was cut off at the model's output limit, so its \
                                 end is missing; the plan, the files and the last message were \
                                 carried in full.",
                            );
                        }
                        self.notify(
                            turn,
                            notice(
                                CompactionState::Finished,
                                message,
                                Some(before),
                                Some(after),
                            ),
                        );
                        return if prep.only {
                            Phase::Finishing(End::completed())
                        } else {
                            Phase::Preparing(Prep {
                                compacted: true,
                                only: false,
                                ..prep
                            })
                        };
                    }
                    Err(Unfinished::Store(error)) => return Phase::Finishing(End::store(&error)),
                    Err(Unfinished::TooLarge(message)) => {
                        // Too large from the `fast` role's model: the main
                        // model is asked, and not `fast` again (found by the
                        // review of step 16).
                        if self.summary_role != ModelRole::Main {
                            self.fast_refused = Some(self.context.epoch_number);
                        }
                        ErrorData::new(ErrorKind::ContextOverflow, "summary_too_large", message)
                    }
                },
                Err(error) => {
                    refusal = error.kind == ErrorKind::ContextOverflow;
                    error
                }
            };
            if failure.kind == ErrorKind::Interrupted || turn.cancel.is_cancelled() {
                return Phase::Interrupted(None);
            }
            if refusal {
                refused += 1;
            }
            // The summary from the background could not be used: one is
            // asked for now, over everything. A refusal is answered once,
            // and so is a failure of the `fast` role's model, by the main
            // model (`counted` turned the role away for the epoch).
            let fast_failed = !refusal && self.summary_role != ModelRole::Main && !fast_retried;
            let again = if refusal {
                refused < 2
            } else {
                from_background || fast_failed
            };
            if fast_failed {
                fast_retried = true;
            }
            if again {
                continue;
            }
            self.notify(
                turn,
                notice(CompactionState::Failed, failure.message.clone(), None, None),
            );
            if prep.overflowed {
                turn.stop = Some(StopReason::ContextOverflow);
            }
            // The turn could not go on because the conversation does not
            // fit: that is what it ends with, and why is in the message.
            return Phase::Finishing(End::failed(ErrorData::new(
                ErrorKind::ContextOverflow,
                "compaction_failed",
                format!(
                    "The conversation could not be compacted: {}",
                    failure.message
                ),
            )));
        }
    }

    /// Records the summary and the new epoch, and begins it. Returns the
    /// estimate of the new epoch's first request.
    async fn finish_compaction(
        &mut self,
        turn: &Turn,
        written: &Written,
        covered: &Cover,
    ) -> Result<u64, Unfinished> {
        let (built, discovered) = self.build_prompt().await;
        self.warn_instructions(turn, &discovered);
        let root = self.root();
        let today = date_of(self.clock.now());
        let carried = Carried {
            plan: self.context.parts.plan.as_ref().map(|board| board.get()),
            files: Carried::files_from(
                &root,
                &self.env.workspace.files.carried(),
                &self.context.touched,
                self.context.carried_before.as_ref(),
            ),
            last_user: self.context.last_user.clone(),
            artifacts: self.context.artifacts.iter().flatten().cloned().collect(),
        };
        // A summary cut off at the output limit says so where it stops.
        let text = if written.cut {
            format!("{}\n\n{SUMMARY_CUT}", written.text.trim_end())
        } else {
            written.text.clone()
        };
        let item = SummaryItem {
            id: self.item_ids.next_id(),
            text: carried.render(&text, &today),
            covers_through: covered.through.clone(),
            usage: None,
            created_at: Some(self.clock.now().to_timestamp()),
        };
        // The new epoch's history: the summary, then what it did not cover,
        // without the reasoning that belongs to the old prefix.
        let mut history = History::default();
        history.push(&Item::Summary(item.clone()));
        history.extend_projected(&self.context.history, covered.messages);
        let tools_bytes: u64 = self
            .context
            .tools
            .iter()
            .map(|tool| {
                (tool.name.len() + tool.description.len() + tool.schema.to_string().len()) as u64
            })
            .sum();
        let after = self
            .context
            .budget
            .tokens(built.system.len() as u64 + tools_bytes + history.bytes());
        let (_, limit) = self.limit(&self.model);
        if self.pressure_of(after, limit) == Pressure::Compact {
            return Err(Unfinished::TooLarge(format!(
                "The summary, with what it carries and the turns it replays, is about {after} \
                 tokens, more than a new context of the model allows."
            )));
        }
        let carried_value = carried.to_value();
        self.store
            .append(
                &turn.id,
                Record::Summary(Summary {
                    item: item.clone(),
                    carried: carried_value.clone(),
                }),
            )
            .await
            .map_err(Unfinished::Store)?;
        let epoch = self.epoch_start(&built, &discovered, EpochReason::Compaction);
        self.begin_epoch(turn, epoch, Some(history))
            .await
            .map_err(Unfinished::Store)?;
        self.context.carried_before = Some(carried_value);
        self.context.touched = crate::context::Touched::default();
        self.shared.lock().history = true;
        self.emit(
            turn,
            Event::ItemCompleted(ItemCompleted {
                item: Item::Summary(item),
            }),
        );
        Ok(after)
    }

    /// Before a request is sent: the context record, and in a debug build
    /// the check that the request begins with the one before it.
    ///
    /// Where the request first differs from the one before is said once it
    /// is known what speed it was sent at ([`settle_sent`](Self::settle_sent)).
    /// The check is made now, before anything is sent, on each speed it may
    /// go at whose settings are the last request's: whichever it is sent at,
    /// an append-only history begins with the last request's bytes.
    pub(super) fn context_request(
        &mut self,
        turn: &Turn,
        model: &RoleModel,
        request: &ModelRequest,
    ) {
        // A request that ended without a word of its end (an interrupt) is
        // settled at the speed the stream said, if it said.
        self.settle_sent();
        let (window, limit) = self.limit(model);
        let info = &model.entry.info;
        let settings_at = |speed: &Speed| {
            hash_of(&(
                info.provider.as_str(),
                info.id.as_str(),
                request.max_output,
                format!("{:?}", request.reasoning),
                speed.as_str(),
            ))
        };
        let settings = settings_at(&request.speed);
        let standard = (!request.speed.is_standard()).then(|| settings_at(&Speed::Standard));
        let tools: Vec<(&str, &str, String)> = request
            .tools
            .iter()
            .map(|tool| {
                (
                    tool.name.as_str(),
                    tool.description.as_str(),
                    tool.schema.to_string(),
                )
            })
            .collect();
        let shape = Shape {
            settings,
            system: hash_of(&request.system),
            tools: hash_of(&tools),
            messages: self.context.history.fingerprints(),
        };
        self.context.sent_bytes = self.context.content_bytes();
        self.pending_context = Some(ContextStat {
            session: self.shared.info.id.clone(),
            turn: turn.id.clone(),
            model: info.id.clone(),
            epoch: self.context.epoch_number,
            sources: self.context.sources(),
            estimate: self.estimate(),
            limit,
            window,
            // Said when the request is settled.
            difference: PrefixDifference::First,
            usage: None,
        });
        let mut in_flight = InFlight {
            shape,
            prefix: None,
            standard: standard.map(|standard| (standard, None)),
            epoch: self.context.epoch_number,
            at_standard: false,
        };
        if cfg!(debug_assertions) {
            in_flight.prefix = model.client.stable_prefix(request);
            self.check_prefix(settings, in_flight.prefix.as_deref());
            if let Some(standard) = standard {
                let mut at = request.clone();
                at.speed = Speed::Standard;
                let bytes = model.client.stable_prefix(&at);
                self.check_prefix(standard, bytes.as_deref());
                in_flight.standard = Some((standard, bytes));
            }
        }
        self.context.in_flight = Some(in_flight);
    }

    /// The stream said the client sends the request in flight at another
    /// speed than it asked for.
    pub(super) fn sent_at(&mut self, moved: &cortex_llm::SpeedMove) {
        if moved.by == cortex_protocol::speed::SpeedFallbackBy::Harness
            && moved.served.is_standard()
            && let Some(in_flight) = &mut self.context.in_flight
        {
            in_flight.at_standard = true;
        }
    }

    /// The request in flight is settled at the speed it was sent at: it is
    /// the request the next is compared with, and its context record says
    /// where it first differs from the one before it. Called when its end
    /// is recorded, and before the next request or a reload otherwise.
    pub(super) fn settle_sent(&mut self) {
        let Some(sent) = self.context.in_flight.take() else {
            return;
        };
        let (settings, prefix) = match sent.standard {
            Some((standard, bytes)) if sent.at_standard => (standard, bytes),
            _ => (sent.shape.settings, sent.prefix),
        };
        let shape = Shape {
            settings,
            ..sent.shape
        };
        let difference = difference(self.context.last_shape.as_ref(), &shape);
        self.context.last_shape = Some(shape);
        if let Some(context) = &mut self.pending_context {
            context.difference = difference;
        }
        if let Some(bytes) = prefix {
            self.context.last_prefix = Some((hash_of(&(settings, sent.epoch)), bytes));
        }
    }

    /// The debug build's check: within an epoch, and with the same
    /// settings, a request's stable prefix begins with the one before it.
    /// A reload that changed nothing keeps the request before it, so the
    /// history read again from the log is held to the one built live.
    fn check_prefix(&self, settings: u64, bytes: Option<&[u8]>) {
        let Some(bytes) = bytes else {
            return;
        };
        let key = hash_of(&(settings, self.context.epoch_number));
        if let Some((last_key, last)) = &self.context.last_prefix
            && *last_key == key
            && let Err(at) = check_prefix(last, bytes)
        {
            panic!(
                "the request does not begin with the one before it in its context epoch: they \
                 part at byte {at}.\nbefore: {}\nnow:    {}",
                excerpt(last, at),
                excerpt(bytes, at)
            );
        }
    }

    /// The provider counted the request that has just been answered, and
    /// the answer is recorded: the estimate goes on from there.
    pub(super) fn context_counted(&mut self, usage: &RequestUsage) {
        let tokens = &usage.tokens;
        let input = tokens.input + tokens.cache_read + tokens.cache_write;
        let after = self.context.content_bytes();
        self.context
            .budget
            .counted(self.context.sent_bytes, input, tokens.output, after);
    }

    /// The turn ends because the provider refused its request as too long
    /// and automatic compaction is off: the loop stops and asks, as it does
    /// at the trigger (the owner's decision after the review of step 15).
    /// The budget has learned from the refusal, so the next turn stops at
    /// the trigger before anything is sent.
    pub(super) fn overflow_without_compaction(&self) -> End {
        End::failed(ErrorData::new(
            ErrorKind::ContextOverflow,
            "compaction_needed",
            "The provider said the conversation does not fit the model's context window, and \
             automatic compaction is off (`context.auto_compact`). Compact it \
             (`session/compact`), or start a new session.",
        ))
    }

    /// The turn ends because the trigger is reached and automatic
    /// compaction is off: the loop stops and asks.
    pub(super) fn compaction_off(&self) -> End {
        let (_, limit) = self.limit(&self.model);
        End::failed(ErrorData::new(
            ErrorKind::ContextOverflow,
            "compaction_needed",
            format!(
                "The conversation is about {} of the {limit} tokens a request may hold, and \
                 automatic compaction is off (`context.auto_compact`). Compact it \
                 (`session/compact`), or start a new session.",
                self.estimate()
            ),
        ))
    }
}

/// Instruction files' names as a notice's `files` gives them to a client:
/// data, as the files are named, not as the model is shown them (the keys
/// the loop holds are escaped where they are made).
fn data_names(shown: &[String]) -> Vec<String> {
    shown
        .iter()
        .map(|name| cortex_base::unescape(name).into_owned())
        .collect()
}
