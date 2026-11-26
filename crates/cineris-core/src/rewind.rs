//! A rewind's plan as a front end is shown it (`session/rewind_plan`, since
//! step 25; `docs/steps/25/answered.md`, point 4), the mark that ties a
//! rewind to the plan the person saw, and the point a rewind, its plan or
//! a fork names.
//!
//! **The point** is an item: after it (`to`, a fork's `at`), or just before
//! it (`before`, since step 25), which is the entry the item's own follows
//! in the session's tree. "Before this prompt" is then the state before
//! the prompt's turn, as `/undo` has it, and not everything after the item
//! before it: the log can hold entries between the two (the turn before's
//! last checkpoint, a plan the person edited), which a rewind to that item
//! would take with it (settled by the coordinator, with the reviewer's
//! agreement, after the review of step 25's second pull request). A prompt
//! that began a turn goes back to before the turn's first entry: what the
//! turn recorded before its prompt (the notices that waited for it, the
//! date and the instruction files' changes, the epoch's map) is the turn's,
//! and goes with it, as `/undo` takes it. Left on the branch, it kept a
//! turn the log records as rewound, which the next `/undo` took for the
//! last turn (the review of that review's fixes). Any other item goes back
//! to just before its own entry.
//!
//! **The mark** is two short hashes: one of the history (the entry the
//! rewind goes back to, the active position it goes back from, and the
//! turns it undoes), one of what it would do to each file (its name, what
//! it goes back to, whether it can be put back and why not).
//! `session/rewind` given a mark makes the plan again and refuses
//! (`plan_changed`), changing nothing, when the part it depends on differs:
//! both parts when it puts files back, the history's alone when it moves
//! the conversation only. The active position moves with every entry of
//! the tree (a turn's, a plan the person edited) and not with a session
//! record (a lease, usage), so only a change of the conversation refuses
//! the rewind. A file edited again in the same way (edited since, and still
//! edited since) does not change the mark: the person was shown that it was
//! edited, and the core asks about it before it is put back.

use cineris_base::ContentHash;
use cineris_protocol::item::Delivery;
use cineris_protocol::methods::{
    RewindFile, RewindFileStatus, RewindTarget, RewindWhyNot, SessionRewindPlanResult,
};
use cineris_protocol::{ErrorKind, ItemId};
use cineris_session::{
    Before, Body, CannotRestore, EntryId, FileRestore, FileStatus, Kind, Log, RewindPlan,
    SessionError,
};

use crate::core::{OpError, error};

/// How many hexadecimal digits of each hash the mark keeps.
const DIGITS: usize = 16;

/// A point of a session's tree, named by an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Point {
    /// After the item: its own entry is the last kept (`to`, `at`).
    After(ItemId),
    /// Just before the item: the entry its own follows is the last kept,
    /// and the item and all after it are not (`before`). For a prompt that
    /// began a turn, the entry the turn's first follows.
    Before(ItemId),
}

impl Point {
    /// The point a rewind or its plan names: exactly one of `to` and
    /// `before`, or `invalid_params`.
    pub(crate) fn one(to: Option<ItemId>, before: Option<ItemId>) -> Result<Self, OpError> {
        match (to, before) {
            (Some(item), None) => Ok(Self::After(item)),
            (None, Some(item)) => Ok(Self::Before(item)),
            _ => Err(error(
                ErrorKind::Usage,
                "invalid_params",
                "Name the point to go back to by exactly one of `to` and `before`.",
            )),
        }
    }

    /// Where a fork ends: at most one of `at` and `before`, or
    /// `invalid_params`. `None`: the end of the session.
    pub(crate) fn at_most_one(
        at: Option<ItemId>,
        before: Option<ItemId>,
    ) -> Result<Option<Self>, OpError> {
        match (at, before) {
            (Some(_), Some(_)) => Err(error(
                ErrorKind::Usage,
                "invalid_params",
                "Name where the fork ends by at most one of `at` and `before`.",
            )),
            (Some(item), None) => Ok(Some(Self::After(item))),
            (None, Some(item)) => Ok(Some(Self::Before(item))),
            (None, None) => Ok(None),
        }
    }

    /// The entry of `log` the point is: `unknown_item` for an item the log
    /// does not hold. With `on_branch` (a rewind, its plan), an item named
    /// by `before` must be on the active branch, as one named by `to` must
    /// (`invalid_entry`). `before` a prompt that began a turn is before
    /// the turn's first entry.
    pub(crate) fn entry(&self, log: &Log, on_branch: bool) -> Result<EntryId, OpError> {
        let (Self::After(item) | Self::Before(item)) = self;
        let own = log.entry_of_item(item).ok_or_else(|| {
            error(
                ErrorKind::Usage,
                "unknown_item",
                format!("The session has no item `{item}`."),
            )
        })?;
        match self {
            Self::After(_) => Ok(own.clone()),
            Self::Before(_) => {
                if on_branch && !log.branch().contains(own) {
                    return Err(not_there(format!("`{item}` is not on the active branch")));
                }
                log.parent_of(turn_start(log, own)?)
                    .cloned()
                    .ok_or_else(|| not_there(format!("nothing comes before `{item}`")))
            }
        }
    }
}

/// Where going back to just before the entry `own` starts: the first entry
/// of its turn when `own` is the prompt that began the turn, `own` itself
/// otherwise. A prompt is a user message that is not steering (a queued
/// one begins a turn of its own); what comes before it in its turn is what
/// the turn recorded as it began.
fn turn_start<'log>(log: &'log Log, own: &'log EntryId) -> Result<&'log EntryId, OpError> {
    let Some(turn) = log.turn_of(own) else {
        return Ok(own);
    };
    if log.kind_of(own) != Some(&Kind::User) {
        return Ok(own);
    }
    let entry = log
        .read(own)
        .map_err(|problem| Box::new(cineris_base::Classified::to_error_data(&problem)))?;
    // Steering came into a turn under way: just before it is just before
    // its own entry.
    if matches!(&entry.body, Body::User(message) if message.delivery == Some(Delivery::Steer)) {
        return Ok(own);
    }
    let mut first = own;
    while let Some(parent) = log.parent_of(first)
        && log.turn_of(parent) == Some(turn)
    {
        first = parent;
    }
    Ok(first)
}

/// A point the session's tree does not have, refused as the session's own
/// check of a rewind's target refuses one.
fn not_there(why: String) -> OpError {
    Box::new(cineris_base::Classified::to_error_data(
        &SessionError::Invalid(why),
    ))
}

/// The plan as `session/rewind_plan` gives it.
pub(crate) fn shown(plan: &RewindPlan) -> SessionRewindPlanResult {
    SessionRewindPlanResult {
        turns: plan.turns.clone(),
        files: plan.files.iter().map(file).collect(),
        unreadable: !plan.unreadable.is_empty(),
        mark: mark(plan),
    }
}

/// Whether `plan`, made again, is still the one `mark` names, as far as a
/// rewind that does or does not put files back depends on it.
pub(crate) fn still(plan: &RewindPlan, mark: &str, restore_files: bool) -> bool {
    let (history, files) = mark.split_once('-').unwrap_or((mark, ""));
    history == history_part(plan)
        && (!restore_files || files == files_part(&plan.files, plan.unreadable.len()))
}

/// The plan's mark: the history's part, a dash, the files' part.
fn mark(plan: &RewindPlan) -> String {
    format!(
        "{}-{}",
        history_part(plan),
        files_part(&plan.files, plan.unreadable.len())
    )
}

fn short(text: &str) -> String {
    let mut hex = ContentHash::of(text.as_bytes()).to_hex();
    hex.truncate(DIGITS);
    hex
}

/// The entry gone back to, the active position gone back from, and the
/// turns undone: each on a line of its own, the position `-` when there is
/// none.
fn history_part(plan: &RewindPlan) -> String {
    let mut text = format!(
        "to {}\nfrom {}\n",
        plan.to,
        plan.from.as_ref().map_or("-", EntryId::as_str)
    );
    for turn in &plan.turns {
        text.push_str(turn.as_str());
        text.push('\n');
    }
    short(&text)
}

/// What happens to each file, whatever order the files come in: each
/// file's lines are sorted before they are hashed (the plan lists them by
/// name too, since a loaded stress of step 25's second pull request found
/// their order differ from run to run).
fn files_part(files: &[FileRestore], unreadable: usize) -> String {
    let mut each = Vec::with_capacity(files.len());
    for restore in files {
        let shown = file(restore);
        let back = match &restore.restore {
            Before::Version(version) => version.hash.to_hex(),
            Before::Absent => "absent".to_owned(),
            Before::Unknown => "unknown".to_owned(),
        };
        // Each field on a line of its own, the path first: a name with a
        // line break in it cannot pass for another file's fields, as its
        // length is said first.
        each.push(format!(
            "{}:{}\n{}\n{}\n{}\n{}\n",
            shown.path.len(),
            shown.path,
            back,
            shown.status.as_str(),
            shown.why_not.as_ref().map_or("", RewindWhyNot::as_str),
            shown.to.as_str(),
        ));
    }
    each.sort();
    let mut text = format!("unreadable {unreadable}\n");
    for one in each {
        text.push_str(&one);
    }
    short(&text)
}

/// One file of the plan as it is shown.
fn file(restore: &FileRestore) -> RewindFile {
    let to = match &restore.restore {
        Before::Version(_) => RewindTarget::Content,
        Before::Absent => RewindTarget::Removed,
        Before::Unknown => RewindTarget::NotRecorded,
    };
    let status = match restore.status {
        FileStatus::AsLeft => RewindFileStatus::AsLeft,
        FileStatus::AlreadyRestored => RewindFileStatus::Already,
        FileStatus::EditedSince => RewindFileStatus::EditedSince,
        FileStatus::CannotRestore => RewindFileStatus::Cannot,
    };
    let why_not = restore.cannot.map(|why| match why {
        CannotRestore::NotRecorded => RewindWhyNot::NotRecorded,
        CannotRestore::TooLarge => RewindWhyNot::TooLarge,
        CannotRestore::Missing => RewindWhyNot::Missing,
        CannotRestore::Unreadable => RewindWhyNot::Unreadable,
        CannotRestore::ThroughALink => RewindWhyNot::ThroughALink,
    });
    RewindFile {
        path: restore.path.clone(),
        to,
        status,
        why_not,
    }
}

#[cfg(test)]
mod tests {
    use cineris_base::ContentHash;
    use cineris_session::entry::FileVersion;

    use super::*;

    fn cannot(why: CannotRestore) -> FileRestore {
        let version = FileVersion::new(ContentHash::of(b"before"), 6, true);
        FileRestore {
            path: "a.txt".to_owned(),
            restore: Before::Version(version),
            left: Some(version),
            now: Some(version),
            status: FileStatus::CannotRestore,
            cannot: Some(why),
        }
    }

    /// Each reason a file cannot be put back reaches the wire as its own
    /// `why_not`, with the status `cannot`: the reason is the session's
    /// enumeration, matched whole (the review of step 25's second pull
    /// request found it matched as text, with a catch-all).
    #[test]
    fn each_reason_a_file_cannot_be_put_back_is_shown_by_its_own_name() {
        for (why, shown) in [
            (CannotRestore::NotRecorded, RewindWhyNot::NotRecorded),
            (CannotRestore::TooLarge, RewindWhyNot::TooLarge),
            (CannotRestore::Missing, RewindWhyNot::Missing),
            (CannotRestore::Unreadable, RewindWhyNot::Unreadable),
            (CannotRestore::ThroughALink, RewindWhyNot::ThroughALink),
        ] {
            let file = file(&cannot(why));
            assert_eq!(file.status, RewindFileStatus::Cannot, "{why:?}");
            assert_eq!(file.why_not, Some(shown), "{why:?}");
        }
    }

    /// The files' part of the mark is the same whatever order the files
    /// come in (a loaded stress of step 25's second pull request found the
    /// order differ from run to run), and moves with what happens to any
    /// one of them.
    #[test]
    fn the_mark_of_the_files_does_not_depend_on_their_order() {
        let mut a = cannot(CannotRestore::Missing);
        let mut b = cannot(CannotRestore::Missing);
        a.path = "a.txt".to_owned();
        b.path = "b.txt".to_owned();
        let one = files_part(&[a.clone(), b.clone()], 0);
        assert_eq!(files_part(&[b.clone(), a.clone()], 0), one);
        b.cannot = Some(CannotRestore::TooLarge);
        assert_ne!(files_part(&[b, a], 0), one);
    }

    /// A rewind names its point by exactly one form, and a fork by at most
    /// one.
    #[test]
    fn a_point_is_named_by_one_form() {
        let item = || Some(ItemId::from("i_3"));
        assert_eq!(
            Point::one(item(), None).unwrap(),
            Point::After(ItemId::from("i_3"))
        );
        assert_eq!(
            Point::one(None, item()).unwrap(),
            Point::Before(ItemId::from("i_3"))
        );
        for (to, before) in [(item(), item()), (None, None)] {
            let refused = Point::one(to, before).unwrap_err();
            assert_eq!(refused.code, "invalid_params");
        }
        assert_eq!(Point::at_most_one(None, None).unwrap(), None);
        assert_eq!(
            Point::at_most_one(None, item()).unwrap(),
            Some(Point::Before(ItemId::from("i_3")))
        );
        let refused = Point::at_most_one(item(), item()).unwrap_err();
        assert_eq!(refused.code, "invalid_params");
    }
}
