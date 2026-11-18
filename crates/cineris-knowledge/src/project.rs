//! One project's knowledge: its private store and the repository's shared
//! profile, put together (`docs/architecture/14-adaptation.md`, "Project
//! knowledge").
//!
//! A [`Project`] names the two files and touches neither until it is
//! asked. [`Project::load`] reads both into a [`Snapshot`]: every entry
//! with the standing it has for this user. Everything that uses the
//! profile (the prompt, the safe list, a listing) reads a snapshot, and
//! every change goes through [`Project::update`], [`Project::propose`] or
//! [`Project::lapse`], which change the private store under its lock and
//! give back the snapshot that follows.
//!
//! # What a shared entry is, to this user
//!
//! A proposal, until this user accepts it. The repository's file is text a
//! stranger may have written, and the model can write it too, so nothing
//! of it reaches the prompt or the safe list for having been written. The
//! user's decision is kept in the private store against the entry's
//! [content hash](Entry::content_hash): an entry that changes is another
//! entry, and a proposal again.
//!
//! # Where the shared file is read
//!
//! `<workspace>/.cineris/profile.toml`, as a regular file in a directory of
//! the workspace itself. A `.cineris` that is a link, or a `profile.toml`
//! that is one, is not followed: the file is then reported and not used.
//! It is written only by `promote`, which adds one entry to its end.
//!
//! # An invalid file
//!
//! Is reported ([`Snapshot::problems`]) and never changed. What is used in
//! its place is the state from before: the snapshot a session already
//! holds, or, for a process that starts on an invalid private store, the
//! good copy beside it (`crate::store`).

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; a project's files are read and changed on the blocking pool"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cineris_base::{
    AtomicWriteOptions, ContentHash, FileStamp, SharedClock, UtcTime, write_atomic,
};
use cineris_protocol::ids::{EntryId, SessionId};
use cineris_protocol::project as wire;

use crate::entry::{
    Basis, Check, Entry, Kind, Learned, MAX_TEXT, Scope, Status, command_from_wire, one_spaced,
};
use crate::file::{
    AppendError, MAX_FILE_BYTES, MAX_PRIVATE_ENTRIES, PrivateState, Problem, SharedFile,
    append_shared, parse_shared, shared_id, write_private,
};
use crate::store::{Source, Store, StoreError, Unwritable, read_bounded};

/// The shared file as a person names it: relative to the workspace root.
pub const SHARED_NAME: &str = ".cineris/profile.toml";

/// The most proposals of a model's that wait for the user's review. Past
/// it `remember` is told the store takes no more.
pub const MAX_PENDING_PROPOSALS: usize = 50;

/// Why a change to a project's knowledge was refused. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChangeError {
    /// No entry has this id.
    #[error("there is no entry `{0}` in this project's profile")]
    UnknownEntry(EntryId),
    /// An action this build does not know.
    #[error("`{0}` is not something this version of Cineris can do to an entry")]
    UnknownAction(String),
    /// The entry, or its new text, is not well formed.
    #[error("{0}")]
    Invalid(String),
    /// The entry is the repository's: its text is changed in the shared
    /// file itself.
    #[error(
        "`{0}` is an entry of the project's shared file, `.cineris/profile.toml`: edit or remove \
         it there. It can be accepted or disabled for you here"
    )]
    SharedReadOnly(EntryId),
    /// The entry cannot be moved to the shared file.
    #[error("`{id}` cannot be promoted to the shared file: {why}")]
    NotPromotable {
        /// The entry.
        id: EntryId,
        /// Why not.
        why: String,
    },
    /// The store holds as much as it may.
    #[error("{0}")]
    Full(String),
    /// The shared file cannot be read as it is, or could not be written.
    #[error("{0}")]
    Shared(String),
    /// The shared file holds as much as it may.
    #[error("{0}")]
    SharedFull(String),
}

impl ChangeError {
    /// The error's code in the protocol.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownEntry(_) => "unknown_entry",
            Self::UnknownAction(_) => "unknown_action",
            Self::Invalid(_) => "invalid_entry",
            Self::SharedReadOnly(_) => "shared_entry_read_only",
            Self::NotPromotable { .. } => "not_promotable",
            Self::Full(_) => "project_store_full",
            Self::Shared(_) => "shared_profile_invalid",
            Self::SharedFull(_) => "shared_profile_full",
        }
    }
}

/// Why a project's knowledge could not be changed.
pub type UpdateError = StoreError<ChangeError>;

/// The code of an [`UpdateError`] in the protocol.
#[must_use]
pub fn error_code(error: &UpdateError) -> &'static str {
    match error {
        StoreError::Invalid(_) => "project_store_invalid",
        StoreError::Full(_) => "project_store_full",
        StoreError::Io { .. } | StoreError::Write(_) | StoreError::Unreadable(_) => {
            "project_store_unwritable"
        }
        StoreError::Refused(refused) => refused.code(),
    }
}

/// What a model proposes for the project's memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    /// A fact or a convention.
    pub kind: Kind,
    /// The entry, in a sentence or two.
    pub text: String,
    /// Where it was learned.
    pub evidence: String,
    /// A test that shows it still holds. Stored and shown, never run here.
    pub check: Option<Check>,
    /// The session it was made in.
    pub session: Option<SessionId>,
}

/// What became of a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalOutcome {
    /// It is stored as proposed, waiting for the user's review.
    Stored,
    /// The same entry is already known, in whatever standing: accepted,
    /// waiting, or turned down. Nothing was added.
    AlreadyKnown,
}

/// Every entry of a project, with the standing each has for this user, as
/// the two files were at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    private: PrivateState,
    shared: Vec<Entry>,
    entries: Vec<Entry>,
    problems: Vec<String>,
    /// The private store could not be read as it is: what stands in its
    /// place feeds the prompt and never the safe list.
    degraded: bool,
    /// The shared file could not be read as it is, and what an earlier
    /// reading held of it stands in its place: that feeds the prompt, and
    /// none of it the safe list.
    shared_stands_in: bool,
    private_stamp: FileStamp,
    shared_stamp: FileStamp,
}

impl Snapshot {
    /// Every entry: the private store's in its order, then the shared
    /// file's in its order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry with this id.
    #[must_use]
    pub fn find(&self, id: &EntryId) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == *id)
    }

    /// What is wrong with the project's files, each in a sentence, with
    /// what is used in the file's place.
    #[must_use]
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Every entry as the protocol shows it.
    #[must_use]
    pub fn listing(&self) -> Vec<wire::ProjectEntry> {
        self.entries.iter().map(Entry::to_wire).collect()
    }

    /// The entries in use at `now`: what the prompt holds.
    pub fn in_use(&self, now: UtcTime) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |entry| entry.in_use(now))
    }

    /// The command entries that may run without asking at `now`, as far as
    /// the profile is concerned: in use, and accepted by this user.
    ///
    /// **None while the private store cannot be read as it is**
    /// ([`is_degraded`](Self::is_degraded)): the state that stands in its
    /// place is an older one, and the file that cannot be read may hold the
    /// edit that turned a command off.
    ///
    /// **And none of the shared file's while that cannot be read as it
    /// is,** for the same reason: what stands in its place is what an
    /// earlier reading held, and the file as it is now may be the one an
    /// entry was taken out of. An older reading handed on from one load to
    /// the next would otherwise keep a removed command on the safe list
    /// for as long as the file stayed unreadable (found by the focused
    /// read of the rules' hand-over, step 17).
    pub fn runs_unasked(&self, now: UtcTime) -> impl Iterator<Item = &Entry> {
        let degraded = self.degraded;
        let shared_stands_in = self.shared_stands_in;
        self.entries.iter().filter(move |entry| {
            !degraded
                && !(shared_stands_in && entry.scope == Scope::Shared)
                && entry.runs_unasked(now)
        })
    }

    /// Whether the private store could not be read as it is when this was
    /// loaded, so that an older state stands in its place.
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// Whether an entry of the private store is past its expiry at `now`
    /// and still marked active: [`Project::lapse`] then has work to do.
    #[must_use]
    pub fn has_lapsed(&self, now: UtcTime) -> bool {
        self.private
            .entries
            .iter()
            .any(|entry| entry.status == Status::Active && entry.is_expired(now))
    }
}

/// What a change left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    /// The project's knowledge after the change.
    pub snapshot: Snapshot,
    /// The entries that are new or changed, as they now are.
    pub changed: Vec<Entry>,
    /// The ids of the entries that are gone. An entry moved to the shared
    /// file is gone under its old id and new under its shared one.
    pub removed: Vec<EntryId>,
}

/// One project's knowledge.
#[derive(Debug, Clone)]
pub struct Project {
    store: Store,
    root: PathBuf,
    clock: SharedClock,
}

/// What reading the shared file gave.
enum SharedRead {
    Absent,
    Entries(SharedFile, String),
    Problem(Problem),
}

impl Project {
    /// The knowledge of the project whose private store is `store_file`
    /// (`StateDir::knowledge_file` for the project's key) and whose
    /// workspace root is `root`. Touches nothing.
    #[must_use]
    pub fn new(
        store_file: impl Into<PathBuf>,
        root: impl Into<PathBuf>,
        clock: SharedClock,
    ) -> Self {
        Self {
            store: Store::new(store_file, SharedClock::clone(&clock)),
            root: root.into(),
            clock,
        }
    }

    /// The private store.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The shared file: `<workspace>/.cineris/profile.toml`.
    #[must_use]
    pub fn shared_file(&self) -> PathBuf {
        self.root.join(".cineris").join("profile.toml")
    }

    /// The time now, by the project's clock.
    #[must_use]
    pub fn now(&self) -> UtcTime {
        self.clock.now()
    }

    fn read_shared(&self) -> SharedRead {
        let problem = |message: String| SharedRead::Problem(Problem::new(SHARED_NAME, message));
        let directory = self.root.join(".cineris");
        match std::fs::symlink_metadata(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return SharedRead::Absent;
            }
            Err(error) => return problem(error.to_string()),
            Ok(found) if found.file_type().is_symlink() => {
                return problem(
                    "`.cineris` is a link, and the shared profile is read only from a directory \
                     of the workspace itself"
                        .to_owned(),
                );
            }
            Ok(found) if !found.is_dir() => return SharedRead::Absent,
            Ok(_) => {}
        }
        match read_bounded(&self.shared_file()) {
            Ok(None) => SharedRead::Absent,
            Ok(Some(Ok(text))) => match parse_shared(&text, SHARED_NAME) {
                Ok(entries) => SharedRead::Entries(entries, text),
                Err(problem) => SharedRead::Problem(problem),
            },
            Ok(Some(Err(message))) => problem(message),
            Err(error) => problem(error.to_string()),
        }
    }

    /// Reads both files. It never fails: a file that cannot be read as it
    /// is, is reported in [`Snapshot::problems`], and what `previous` held
    /// of it is used in its place (for the private store, the good copy
    /// when there is no `previous`).
    #[must_use]
    pub fn load(&self, previous: Option<&Snapshot>) -> Snapshot {
        let loaded = self.store.load();
        let mut problems = Vec::new();
        let degraded = loaded.problem.is_some();
        let private = match (&loaded.problem, previous) {
            (None, _) => loaded.state,
            (Some(problem), Some(previous)) => {
                problems.push(format!(
                    "{problem}. What was read of it before is still used, and nothing is written \
                     to it until it is put right."
                ));
                previous.private.clone()
            }
            (Some(problem), None) => {
                problems.push(match loaded.source {
                    Source::GoodCopy => format!(
                        "{problem}. The copy of Cineris's last write, `{}`, is used in its \
                         place, and nothing is written until the file is put right.",
                        display_name(&self.store.good_copy())
                    ),
                    Source::File | Source::Nothing => format!(
                        "{problem}. Nothing of it is used, and nothing is written to it until it \
                         is put right."
                    ),
                });
                loaded.state
            }
        };
        // The stamp before the read, so that a change between the two is
        // seen at the next look.
        let shared_stamp = FileStamp::of(&self.shared_file());
        let (shared, shared_stands_in) = self.shared_or(previous, &mut problems);
        snapshot(
            private,
            shared,
            problems,
            (degraded, shared_stands_in),
            loaded.stamp,
            shared_stamp,
        )
    }

    /// The shared file's entries, or what `previous` held of it when it
    /// cannot be read as it is, with what is wrong added to `problems`.
    /// And whether it is the earlier reading that stands in.
    fn shared_or(
        &self,
        previous: Option<&Snapshot>,
        problems: &mut Vec<String>,
    ) -> (Vec<Entry>, bool) {
        match self.read_shared() {
            SharedRead::Absent => (Vec::new(), false),
            SharedRead::Entries(shared, _) => {
                problems.extend(shared.left_out);
                (shared.entries, false)
            }
            SharedRead::Problem(problem) => match previous {
                Some(previous) => {
                    problems.push(format!(
                        "{problem}. What was read of it before is still used, and none of its \
                         commands runs without asking until it is put right."
                    ));
                    (previous.shared.clone(), true)
                }
                None => {
                    problems.push(format!("{problem}. Nothing of it is used."));
                    (Vec::new(), false)
                }
            },
        }
    }

    /// Whether both files look as they did when `snapshot` was read: two
    /// `stat` calls. `false` means [`load`](Self::load) should be called
    /// again.
    #[must_use]
    pub fn is_current(&self, snapshot: &Snapshot) -> bool {
        self.store.stamp() == snapshot.private_stamp
            && FileStamp::of(&self.shared_file()) == snapshot.shared_stamp
    }

    /// The snapshot after a change to the private store whose state is
    /// `private`, and which looked like `private_stamp` while the change
    /// still held the lock. Taken any later, the stamp could be of another
    /// process's change beside this one's state, and that change (a command
    /// turned off) would never be seen.
    pub(crate) fn after(
        &self,
        private: PrivateState,
        private_stamp: FileStamp,
        previous: Option<&Snapshot>,
    ) -> Snapshot {
        cineris_base::hook::point(BEFORE_THE_SNAPSHOT);
        let shared_stamp = FileStamp::of(&self.shared_file());
        let mut problems = Vec::new();
        let (shared, shared_stands_in) = self.shared_or(previous, &mut problems);
        // A change was just written, so the store is one this build reads.
        snapshot(
            private,
            shared,
            problems,
            (false, shared_stands_in),
            private_stamp,
            shared_stamp,
        )
    }

    /// Applies a user's changes, then adds the entries the user wrote, all
    /// or nothing, under the store's lock. `previous` is the snapshot the
    /// caller holds, for what it kept of a shared file that is invalid
    /// now.
    ///
    /// # Errors
    ///
    /// When any change is refused, or the store cannot be read as it is or
    /// written. Nothing is changed then.
    pub fn update(
        &self,
        changes: &[wire::EntryChange],
        add: &[wire::NewEntry],
        previous: Option<&Snapshot>,
    ) -> Result<Updated, UpdateError> {
        let ((changed, removed), private, stamp) = self.store.change(|state, now| {
            let (shared, shared_text) = match self.read_shared() {
                SharedRead::Absent => (Ok(Vec::new()), None),
                SharedRead::Entries(shared, text) => (Ok(shared.entries), Some(text)),
                SharedRead::Problem(problem) => (Err(problem), None),
            };
            let mut work = Work {
                state,
                shared,
                shared_text,
                appended: false,
                changed: Vec::new(),
                removed: Vec::new(),
                now,
            };
            for change in changes {
                work.apply(change)?;
            }
            for new in add {
                work.add(new)?;
            }
            // The shared file last, once every change is known to be good:
            // a failure here leaves both files as they were.
            if work.appended
                && let Some(text) = &work.shared_text
            {
                // And only once the store is known to take the change: a
                // store that would refuse it must refuse before the
                // repository's file is written.
                match crate::store::writable(work.state) {
                    Ok(_) => {}
                    Err(Unwritable::Full(message)) => return Err(ChangeError::Full(message)),
                    Err(Unwritable::Unreadable(problem)) => {
                        return Err(ChangeError::Invalid(format!(
                            "the change would leave a store that could not be read again \
                             ({problem}), so it was not made"
                        )));
                    }
                }
                cineris_base::hook::point(BEFORE_THE_SHARED_WRITE);
                self.write_shared(text)?;
            }
            let touched = !work.changed.is_empty() || !work.removed.is_empty();
            Ok::<_, ChangeError>(((work.changed, work.removed), touched))
        })?;
        let snapshot = self.after(private, stamp, previous);
        let changed = changed
            .into_iter()
            .filter_map(|id| snapshot.find(&id).cloned())
            .collect();
        Ok(Updated {
            snapshot,
            changed,
            removed,
        })
    }

    /// Stores a model's proposal as proposed: it waits for the user, and
    /// nothing here can make it active.
    ///
    /// # Errors
    ///
    /// When the store is full of proposals, cannot be read as it is, or
    /// cannot be written.
    pub fn propose(
        &self,
        proposal: Proposed,
        previous: Option<&Snapshot>,
    ) -> Result<(ProposalOutcome, Updated), UpdateError> {
        if !matches!(proposal.kind, Kind::Fact | Kind::Convention) {
            return Err(StoreError::Refused(ChangeError::Invalid(
                "a proposal is a fact or a convention".to_owned(),
            )));
        }
        let shared = match self.read_shared() {
            SharedRead::Entries(shared, _) => shared.entries,
            SharedRead::Absent => Vec::new(),
            SharedRead::Problem(_) => previous
                .map(|previous| previous.shared.clone())
                .unwrap_or_default(),
        };
        let ((outcome, id), private, stamp) = self.store.change(|state, now| {
            let text = one_spaced(&proposal.text);
            let known = state
                .entries
                .iter()
                .chain(&shared)
                .any(|entry| entry.kind == proposal.kind && one_spaced(&entry.text) == text);
            if known {
                return Ok(((ProposalOutcome::AlreadyKnown, None), false));
            }
            let waiting = state
                .entries
                .iter()
                .filter(|entry| {
                    entry.status == Status::Proposed && entry.basis == Some(Basis::Proposed)
                })
                .count();
            if waiting >= MAX_PENDING_PROPOSALS {
                return Err(ChangeError::Full(format!(
                    "{waiting} proposals for this project's memory wait for the user's review, \
                     and no more are taken until some are reviewed"
                )));
            }
            room(state)?;
            let mut entry = Entry::written(state.take_id(), Scope::Private, proposal.kind, text);
            entry.status = Status::Proposed;
            entry.basis = Some(Basis::Proposed);
            entry.learned = Some(Learned {
                session: proposal.session.clone(),
                item: None,
                at: now,
            });
            entry.evidence = Some(proposal.evidence.trim().to_owned());
            entry.check.clone_from(&proposal.check);
            entry.validate().map_err(ChangeError::Invalid)?;
            let id = entry.id.clone();
            state.entries.push(entry);
            Ok(((ProposalOutcome::Stored, Some(id)), true))
        })?;
        let snapshot = self.after(private, stamp, previous);
        let changed = id
            .and_then(|id| snapshot.find(&id).cloned())
            .into_iter()
            .collect();
        Ok((
            outcome,
            Updated {
                snapshot,
                changed,
                removed: Vec::new(),
            },
        ))
    }

    /// Takes every learned entry that is past its expiry out of use: it is
    /// disabled and marked as lapsed, and no longer accepted. A new
    /// observation or the user's `accept` brings it back.
    ///
    /// # Errors
    ///
    /// When the store cannot be read as it is or written.
    pub fn lapse(&self, previous: Option<&Snapshot>) -> Result<Updated, UpdateError> {
        let (ids, private, stamp) = self.store.change(|state, now| {
            let mut ids = Vec::new();
            for entry in &mut state.entries {
                if entry.status == Status::Active && entry.is_expired(now) {
                    entry.status = Status::Disabled;
                    entry.lapsed = Some(now);
                    entry.accepted = None;
                    ids.push(entry.id.clone());
                }
            }
            let changed = !ids.is_empty();
            Ok::<_, ChangeError>((ids, changed))
        })?;
        let snapshot = self.after(private, stamp, previous);
        let changed = ids
            .iter()
            .filter_map(|id| snapshot.find(id).cloned())
            .collect();
        Ok(Updated {
            snapshot,
            changed,
            removed: Vec::new(),
        })
    }

    /// Writes the shared file, in a `.cineris` that is a directory of the
    /// workspace itself, made if it is missing.
    fn write_shared(&self, text: &str) -> Result<(), ChangeError> {
        let refused = |message: String| ChangeError::Shared(format!("`{SHARED_NAME}`: {message}"));
        let directory = self.root.join(".cineris");
        match std::fs::symlink_metadata(&directory) {
            Ok(found) if found.file_type().is_symlink() || !found.is_dir() => {
                return Err(refused(
                    "`.cineris` is not a directory of the workspace itself, so nothing is written \
                     there"
                        .to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&directory).map_err(|error| refused(error.to_string()))?;
            }
            Err(error) => return Err(refused(error.to_string())),
        }
        let options = AtomicWriteOptions {
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&self.shared_file(), text.as_bytes(), &options, &*self.clock)
            .map_err(|error| refused(error.to_string()))
    }
}

/// The point of a change where the store is written and its lock let go,
/// before the snapshot that follows is put together: a test holds it
/// (`cineris_base::hook`) to let another process change the store there.
pub const BEFORE_THE_SNAPSHOT: &str =
    "cineris-knowledge: a change is written, before the snapshot that follows it";

/// The point of a `promote` where the shared file has been read and
/// checked and is about to be written: a test holds it to plant a link
/// there.
pub const BEFORE_THE_SHARED_WRITE: &str =
    "cineris-knowledge: the shared file is read, before it is written";

/// A path as a message names it: its last name.
fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// How much of the store's size is kept for the changes that only narrow
/// or decide: turning an entry off, marking one lapsed, a decision about a
/// shared entry. A new entry is refused once the store is within this of
/// its cap, so that a full store can still be told to use less.
pub const RESERVED_BYTES: u64 = 32 * 1024;

/// Whether the store has room for one more entry: under its cap on
/// entries, and with [`RESERVED_BYTES`] of its size left.
pub(crate) fn room(state: &PrivateState) -> Result<(), ChangeError> {
    if state.entries.len() >= MAX_PRIVATE_ENTRIES {
        return Err(ChangeError::Full(format!(
            "this project's store holds {MAX_PRIVATE_ENTRIES} entries, the most it may: delete \
             some with `cineris project delete`"
        )));
    }
    let bytes = write_private(state).len() as u64;
    if bytes + RESERVED_BYTES > MAX_FILE_BYTES {
        return Err(ChangeError::Full(format!(
            "this project's store is {bytes} bytes, and takes no new entry within \
             {RESERVED_BYTES} bytes of the {MAX_FILE_BYTES} it may be: delete some entries with \
             `cineris project delete`"
        )));
    }
    Ok(())
}

/// Puts the two files together: each shared entry with what this user
/// decided about it.
fn snapshot(
    private: PrivateState,
    shared: Vec<Entry>,
    problems: Vec<String>,
    (degraded, shared_stands_in): (bool, bool),
    private_stamp: FileStamp,
    shared_stamp: FileStamp,
) -> Snapshot {
    let mut entries = private.entries.clone();
    let own: BTreeSet<[u8; 32]> = private
        .entries
        .iter()
        .map(|entry| *entry.content_hash().as_bytes())
        .collect();
    for entry in &shared {
        let hash = entry.content_hash();
        // The user's own copy of the same entry stands for it: a promote
        // that stopped between its two writes leaves one in each file.
        if own.contains(hash.as_bytes()) {
            continue;
        }
        entries.push(standing(entry, &private, &hash));
    }
    Snapshot {
        private,
        shared,
        entries,
        problems,
        degraded,
        shared_stands_in,
        private_stamp,
        shared_stamp,
    }
}

/// A shared entry with the standing this user gave it: a proposal until
/// they decide.
fn standing(entry: &Entry, private: &PrivateState, hash: &ContentHash) -> Entry {
    let mut entry = entry.clone();
    entry.accepted = None;
    match private.decision(hash) {
        Some(decision) if decision.status == Status::Active => {
            entry.status = Status::Active;
            entry.accepted = Some(decision.at);
            entry.confirmed = Some(decision.at);
        }
        Some(_) => entry.status = Status::Disabled,
        None => entry.status = Status::Proposed,
    }
    entry
}

/// One `update`, in progress.
struct Work<'a> {
    state: &'a mut PrivateState,
    /// The shared file's entries, or why it cannot be read.
    shared: Result<Vec<Entry>, Problem>,
    /// The shared file's text, with what this update adds to it.
    shared_text: Option<String>,
    appended: bool,
    changed: Vec<EntryId>,
    removed: Vec<EntryId>,
    now: UtcTime,
}

impl Work<'_> {
    fn touched(&mut self, id: &EntryId) {
        if !self.changed.contains(id) {
            self.changed.push(id.clone());
        }
    }

    fn private(&mut self, id: &EntryId) -> Option<&mut Entry> {
        self.state.entries.iter_mut().find(|entry| entry.id == *id)
    }

    /// The shared entry with this id.
    fn shared(&self, id: &EntryId) -> Result<Option<Entry>, ChangeError> {
        match &self.shared {
            Ok(entries) => Ok(entries.iter().find(|entry| entry.id == *id).cloned()),
            // An id of the shared file cannot be told from an unknown one
            // while the file cannot be read: say why.
            Err(problem) if id.as_str().starts_with("s_") => {
                Err(ChangeError::Shared(problem.to_string()))
            }
            Err(_) => Ok(None),
        }
    }

    fn apply(&mut self, change: &wire::EntryChange) -> Result<(), ChangeError> {
        let id = &change.id;
        let now = self.now;
        match &change.action {
            wire::EntryAction::Accept => {
                if let Some(entry) = self.private(id) {
                    entry.status = Status::Active;
                    entry.accepted = Some(now);
                    entry.confirmed = Some(now);
                    entry.lapsed = None;
                    if entry.learned.is_some() {
                        entry.expires = now.checked_add(entry.kind.lifetime());
                    }
                } else {
                    let entry = self
                        .shared(id)?
                        .ok_or_else(|| ChangeError::UnknownEntry(id.clone()))?;
                    self.state.decide(entry.content_hash(), Status::Active, now);
                }
            }
            wire::EntryAction::Disable => {
                if let Some(entry) = self.private(id) {
                    entry.status = Status::Disabled;
                    entry.accepted = None;
                    // The user's own word: it is not learned again.
                    entry.lapsed = None;
                } else {
                    let entry = self
                        .shared(id)?
                        .ok_or_else(|| ChangeError::UnknownEntry(id.clone()))?;
                    self.state
                        .decide(entry.content_hash(), Status::Disabled, now);
                }
            }
            wire::EntryAction::Edit => {
                let text = one_spaced(change.text.as_deref().unwrap_or(""));
                if text.is_empty() {
                    return Err(ChangeError::Invalid(
                        "an edit needs the entry's new `text`".to_owned(),
                    ));
                }
                if text.chars().count() > MAX_TEXT {
                    return Err(ChangeError::Invalid(format!(
                        "the new text is {} characters long, and may be up to {MAX_TEXT}",
                        text.chars().count()
                    )));
                }
                match self.private(id) {
                    Some(entry) => entry.text = text,
                    None => return Err(self.not_private(id)),
                }
            }
            wire::EntryAction::Delete => {
                if self.private(id).is_none() {
                    return Err(self.not_private(id));
                }
                self.state.entries.retain(|entry| entry.id != *id);
                self.changed.retain(|changed| changed != id);
                self.removed.push(id.clone());
                return Ok(());
            }
            wire::EntryAction::Promote => return self.promote(id),
            other => return Err(ChangeError::UnknownAction(other.as_str().to_owned())),
        }
        self.touched(id);
        Ok(())
    }

    /// Why a change that is for a private entry cannot be made to `id`.
    fn not_private(&self, id: &EntryId) -> ChangeError {
        match self.shared(id) {
            Ok(Some(_)) => ChangeError::SharedReadOnly(id.clone()),
            Ok(None) => ChangeError::UnknownEntry(id.clone()),
            Err(error) => error,
        }
    }

    fn promote(&mut self, id: &EntryId) -> Result<(), ChangeError> {
        let now = self.now;
        let refused = |why: &str| ChangeError::NotPromotable {
            id: id.clone(),
            why: why.to_owned(),
        };
        let Some(entry) = self.private(id).map(|entry| entry.clone()) else {
            return Err(match self.shared(id)? {
                Some(_) => refused("it is in the shared file already"),
                None => ChangeError::UnknownEntry(id.clone()),
            });
        };
        if !entry.in_use(now) {
            return Err(refused("only an entry in use is shared; accept it first"));
        }
        if !entry.is_accepted() {
            return Err(refused(
                "it became active by itself and you have not accepted it; accept it first",
            ));
        }
        if let Err(problem) = &self.shared {
            return Err(ChangeError::Shared(problem.to_string()));
        }
        // The entry as the shared file holds it, and this user's
        // acceptance of exactly that.
        let hash = {
            let mut shared = entry.clone();
            if let Some(command) = &mut shared.command {
                command.usual_ms = None;
            }
            shared.content_hash()
        };
        // A promote that stopped between its two writes left the entry in
        // the shared file already: this one finishes it, and adds nothing.
        let there = self
            .shared
            .as_ref()
            .is_ok_and(|entries| entries.iter().any(|held| held.content_hash() == hash));
        if !there {
            let text = append_shared(self.shared_text.as_deref(), &entry, SHARED_NAME).map_err(
                |error| match error {
                    AppendError::Full(problem) => ChangeError::SharedFull(problem.to_string()),
                    AppendError::Invalid(problem) => ChangeError::Shared(problem.to_string()),
                },
            )?;
            self.shared_text = Some(text);
            self.appended = true;
        }
        self.state.entries.retain(|held| held.id != *id);
        self.state.decide(hash, Status::Active, now);
        self.changed.retain(|changed| changed != id);
        self.removed.push(id.clone());
        self.changed.push(shared_id(&hash));
        Ok(())
    }

    fn add(&mut self, new: &wire::NewEntry) -> Result<(), ChangeError> {
        let kind = Kind::from_wire(&new.kind).ok_or_else(|| {
            ChangeError::Invalid(format!(
                "`{}` is not a kind of entry: command, convention, flaky or fact",
                new.kind.as_str()
            ))
        })?;
        room(self.state)?;
        let mut entry = Entry::written(
            self.state.take_id(),
            Scope::Private,
            kind,
            one_spaced(&new.text),
        );
        entry.command = new
            .command
            .as_ref()
            .map(command_from_wire)
            .transpose()
            .map_err(ChangeError::Invalid)?;
        entry.check = new
            .check
            .as_ref()
            .map(Check::from_wire)
            .transpose()
            .map_err(ChangeError::Invalid)?;
        entry.validate().map_err(ChangeError::Invalid)?;
        let hash = entry.content_hash();
        if let Some(known) = self
            .state
            .entries
            .iter()
            .find(|known| known.content_hash() == hash)
        {
            return Err(ChangeError::Invalid(format!(
                "this entry is already in the project's store, as `{}`",
                known.id
            )));
        }
        let id = entry.id.clone();
        self.state.entries.push(entry);
        self.touched(&id);
        Ok(())
    }
}
