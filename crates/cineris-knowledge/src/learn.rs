//! Storing what the observer saw
//! (`docs/architecture/14-adaptation.md`, "How entries are learned").
//!
//! [`Project::learn`] counts one thing a session's [`Watch`] saw: a build,
//! test or check command that succeeded, or a test command that flipped.
//!
//! | The store holds | What the observation does |
//! |---|---|
//! | Nothing for this command | A `proposed` entry, seen once. The user may accept it at once. |
//! | An observed entry, proposed | It is seen again: at two it becomes `active`, and the user is told. |
//! | An observed entry, active | It is confirmed, at most once a day: its expiry moves out, and its usual duration follows. |
//! | An observed entry that lapsed | It is active again, and not accepted: the user is told. |
//! | An entry the user disabled | Nothing: it is not learned again. |
//! | An entry in use that someone wrote | Nothing: it is known already. |
//!
//! **Becoming active is not being accepted.** An entry that became active
//! here is in the prompt. It runs without asking, and is offered to
//! verification, only once the user accepts it.
//!
//! **Entries seen once do not fill the store.** At most
//! [`MAX_SEEN_ONCE`] are kept, the one seen longest ago goes when a new
//! one comes, and one not seen again in [`SEEN_ONCE_DAYS`] days is
//! dropped.
//!
//! [`Watch`]: crate::observe::Watch

use std::time::Duration;

use cineris_base::UtcTime;
use cineris_protocol::ids::EntryId;

use crate::entry::{Basis, Check, Command, Entry, Kind, Learned, Scope, Status, normal_dir};
use crate::file::PrivateState;
use crate::observe::Learnt;
use crate::project::{ChangeError, Project, Snapshot, UpdateError, Updated, room};

/// The most entries seen once that the store keeps.
pub const MAX_SEEN_ONCE: usize = 40;

/// How many days an entry seen once waits to be seen again.
pub const SEEN_ONCE_DAYS: u64 = 7;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// What became of an observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changed: the entry is known, was confirmed today already,
    /// or the user turned it off.
    Nothing,
    /// A new entry, proposed: seen once.
    Proposed,
    /// The entry became active: seen twice. The user is told.
    Activated,
    /// An active entry was seen to hold again.
    Confirmed,
    /// An entry that had lapsed is active again. The user is told.
    Revived,
}

/// Whether an entry is the one an observation is of: the same kind, the
/// same line, the same directory.
fn is_of(entry: &Entry, learnt: &Learnt) -> bool {
    entry.kind == learnt.kind
        && entry
            .command
            .as_ref()
            .is_some_and(|command| command.run == learnt.line && command.cwd == learnt.cwd)
}

fn seen_once(entry: &Entry) -> bool {
    entry.status == Status::Proposed && entry.basis == Some(Basis::Observed)
}

/// The entry's words.
fn text_of(learnt: &Learnt) -> String {
    match learnt.kind {
        Kind::Flaky => format!(
            "The test command `{}`{} has failed and then passed with nothing changed in between.",
            learnt.line,
            learnt
                .cwd
                .as_ref()
                .map_or_else(String::new, |cwd| format!(" (in `{cwd}`)"))
        ),
        _ => format!("The project's {} command.", learnt.purpose.as_str()),
    }
}

/// The check the harness writes for an observed command by itself: that
/// its program still exists, where the program is a path inside the
/// workspace (`./scripts/test.sh`). A program found on the search path has
/// none: no form of check says "is on the search path" without running
/// something.
fn check_of(learnt: &Learnt) -> Option<Check> {
    let program = learnt
        .line
        .split(' ')
        .find(|word| !word.contains('=') || word.starts_with("./"))?;
    // A program on the search path, a rooted one, or one on a drive
    // (`C:/tools/jest.cmd`, which the store refuses as a check's path).
    if !program.contains('/') || program.starts_with('/') || program.contains(':') {
        return None;
    }
    let joined = match &learnt.cwd {
        Some(cwd) => format!("{cwd}/{program}"),
        None => program.to_owned(),
    };
    if joined.split('/').any(|part| part == "..") {
        return None;
    }
    normal_dir(Some(&joined)).map(|path| Check::Exists { path })
}

impl Snapshot {
    /// Whether storing `learnt` could change the store at `now`. `false`
    /// spares the lock and the read for the common case: a command that is
    /// known and was confirmed today, or one the user turned off.
    #[must_use]
    pub fn wants(&self, learnt: &Learnt, now: UtcTime) -> bool {
        let known = self.entries().iter().any(|entry| {
            entry.kind == learnt.kind
                && entry.in_use(now)
                && entry.basis != Some(Basis::Observed)
                && entry
                    .command
                    .as_ref()
                    .is_some_and(|command| command.run == learnt.line && command.cwd == learnt.cwd)
        });
        if known || self.is_degraded() {
            return false;
        }
        match self
            .entries()
            .iter()
            .find(|entry| entry.scope == Scope::Private && is_of(entry, learnt))
        {
            None => true,
            Some(entry) => match entry.status {
                Status::Proposed => true,
                Status::Disabled => entry.lapsed.is_some(),
                Status::Active => confirm_due(entry, now),
            },
        }
    }
}

/// Whether an active observed entry was last confirmed a day ago or more.
fn confirm_due(entry: &Entry, now: UtcTime) -> bool {
    entry.basis == Some(Basis::Observed)
        && entry
            .confirmed
            .and_then(|confirmed| now.duration_since(confirmed))
            .is_none_or(|since| since >= DAY)
}

/// Drops the entries seen once that were not seen again in time, and the
/// one seen longest ago when there is no room for another.
fn make_room(state: &mut PrivateState, now: UtcTime, removed: &mut Vec<EntryId>) {
    let stale = |entry: &Entry| {
        seen_once(entry)
            && entry
                .confirmed
                .and_then(|confirmed| now.duration_since(confirmed))
                .is_some_and(|since| since >= DAY * u32::try_from(SEEN_ONCE_DAYS).unwrap_or(7))
    };
    removed.extend(
        state
            .entries
            .iter()
            .filter(|entry| stale(entry))
            .map(|entry| entry.id.clone()),
    );
    state.entries.retain(|entry| !stale(entry));
    while state
        .entries
        .iter()
        .filter(|entry| seen_once(entry))
        .count()
        >= MAX_SEEN_ONCE
    {
        let Some(oldest) = state
            .entries
            .iter()
            .filter(|entry| seen_once(entry))
            .min_by_key(|entry| entry.confirmed)
            .map(|entry| entry.id.clone())
        else {
            break;
        };
        state.entries.retain(|entry| entry.id != oldest);
        removed.push(oldest);
    }
}

impl Project {
    /// Counts one thing the observer saw. See the [module](self).
    ///
    /// # Errors
    ///
    /// When the store cannot be read as it is or written, or is full.
    pub fn learn(
        &self,
        learnt: &Learnt,
        previous: Option<&Snapshot>,
    ) -> Result<(Outcome, Updated), UpdateError> {
        if !matches!(learnt.kind, Kind::Command | Kind::Flaky) {
            return Err(UpdateError::Refused(ChangeError::Invalid(
                "an observation is of a command or of a flaky test".to_owned(),
            )));
        }
        // What the shared file holds in use is known already.
        let shared_knows = previous.is_some_and(|previous| {
            previous.entries().iter().any(|entry| {
                entry.scope == Scope::Shared && entry.in_use(self.now()) && is_of(entry, learnt)
            })
        });
        let ((outcome, id, removed), private, stamp) = self.store().change(|state, now| {
            let mut removed = Vec::new();
            let lifetime = learnt.kind.lifetime();
            let held = state.entries.iter_mut().find(|entry| is_of(entry, learnt));
            let Some(entry) = held else {
                if shared_knows {
                    return Ok(((Outcome::Nothing, None, removed), false));
                }
                make_room(state, now, &mut removed);
                room(state)?;
                let mut entry = Entry::written(
                    state.take_id(),
                    Scope::Private,
                    learnt.kind,
                    text_of(learnt),
                );
                entry.status = Status::Proposed;
                entry.basis = Some(Basis::Observed);
                entry.learned = Some(Learned {
                    session: learnt.session.clone(),
                    item: None,
                    at: now,
                });
                entry.evidence = Some(match learnt.kind {
                    Kind::Flaky => "Seen by Cineris in a session.".to_owned(),
                    _ => "Ran and succeeded.".to_owned(),
                });
                entry.confirmed = Some(now);
                entry.seen = 1;
                entry.command = Some(Command {
                    purpose: learnt.purpose,
                    run: learnt.line.clone(),
                    cwd: learnt.cwd.clone(),
                    usual_ms: Some(learnt.duration_ms),
                    read_only: learnt.read_only,
                });
                entry.check = (learnt.kind == Kind::Command)
                    .then(|| check_of(learnt))
                    .flatten();
                entry.validate().map_err(ChangeError::Invalid)?;
                let id = entry.id.clone();
                state.entries.push(entry);
                return Ok(((Outcome::Proposed, Some(id), removed), true));
            };
            // An entry somebody wrote, or a model proposed: not the
            // observer's to change.
            if entry.basis != Some(Basis::Observed) {
                return Ok(((Outcome::Nothing, None, removed), false));
            }
            let id = entry.id.clone();
            let follow = |entry: &mut Entry| {
                entry.confirmed = Some(now);
                entry.seen = entry.seen.saturating_add(1);
                if let Some(command) = &mut entry.command {
                    command.usual_ms = Some(match command.usual_ms {
                        // A moving average: three parts what it was.
                        Some(usual) => {
                            (usual.saturating_mul(3).saturating_add(learnt.duration_ms)) / 4
                        }
                        None => learnt.duration_ms,
                    });
                }
            };
            let outcome = match entry.status {
                Status::Proposed => {
                    follow(entry);
                    if entry.seen >= 2 {
                        entry.status = Status::Active;
                        entry.expires = now.checked_add(lifetime);
                        Outcome::Activated
                    } else {
                        Outcome::Proposed
                    }
                }
                // Past its expiry and not yet marked: it lapsed, and is
                // seen to hold again. As for one that was marked, the
                // acceptance does not come back with it.
                Status::Active if entry.is_expired(now) => {
                    follow(entry);
                    entry.accepted = None;
                    entry.expires = now.checked_add(lifetime);
                    Outcome::Revived
                }
                Status::Active if confirm_due(entry, now) => {
                    follow(entry);
                    entry.expires = now.checked_add(lifetime);
                    Outcome::Confirmed
                }
                Status::Active => Outcome::Nothing,
                // Lapsed: seen to hold again. It was not accepted when it
                // lapsed, and is not now.
                Status::Disabled if entry.lapsed.is_some() => {
                    follow(entry);
                    entry.status = Status::Active;
                    entry.lapsed = None;
                    entry.accepted = None;
                    entry.expires = now.checked_add(lifetime);
                    Outcome::Revived
                }
                // The user's own word.
                Status::Disabled => Outcome::Nothing,
            };
            let changed = outcome != Outcome::Nothing;
            Ok::<_, ChangeError>(((outcome, changed.then_some(id), removed), changed))
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
                removed,
            },
        ))
    }
}
