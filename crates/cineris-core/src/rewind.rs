//! A rewind's plan as a front end is shown it (`session/rewind_plan`, since
//! step 25; `docs/steps/25/answered.md`, point 4), and the mark that ties a
//! rewind to the plan the person saw.
//!
//! **The mark** is two short hashes: one of the turns the plan undoes, one
//! of what it would do to each file (its name, what it goes back to, whether
//! it can be put back and why not). `session/rewind` given a mark makes the
//! plan again and refuses (`plan_changed`), changing nothing, when the part
//! it depends on differs: both parts when it puts files back, the turns'
//! alone when it moves the conversation only. A file edited again in the
//! same way (edited since, and still edited since) does not change the
//! mark: the person was shown that it was edited, and the core asks about
//! it before it is put back.

use cineris_base::ContentHash;
use cineris_protocol::methods::{
    RewindFile, RewindFileStatus, RewindTarget, RewindWhyNot, SessionRewindPlanResult,
};
use cineris_session::{Before, CannotRestore, FileRestore, FileStatus, RewindPlan};

/// How many hexadecimal digits of each hash the mark keeps.
const DIGITS: usize = 16;

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
    let (turns, files) = mark.split_once('-').unwrap_or((mark, ""));
    turns == turns_part(plan) && (!restore_files || files == files_part(plan))
}

/// The plan's mark: the turns' part, a dash, the files' part.
fn mark(plan: &RewindPlan) -> String {
    format!("{}-{}", turns_part(plan), files_part(plan))
}

fn short(text: &str) -> String {
    let mut hex = ContentHash::of(text.as_bytes()).to_hex();
    hex.truncate(DIGITS);
    hex
}

fn turns_part(plan: &RewindPlan) -> String {
    let mut text = String::new();
    for turn in &plan.turns {
        text.push_str(turn.as_str());
        text.push('\n');
    }
    short(&text)
}

fn files_part(plan: &RewindPlan) -> String {
    let mut text = format!("unreadable {}\n", plan.unreadable.len());
    for restore in &plan.files {
        let shown = file(restore);
        let back = match &restore.restore {
            Before::Version(version) => version.hash.to_hex(),
            Before::Absent => "absent".to_owned(),
            Before::Unknown => "unknown".to_owned(),
        };
        // Each field on a line of its own, the path first: a name with a
        // line break in it cannot pass for another file's fields, as its
        // length is said first.
        text.push_str(&format!(
            "{}:{}\n{}\n{}\n{}\n{}\n",
            shown.path.len(),
            shown.path,
            back,
            shown.status.as_str(),
            shown.why_not.as_ref().map_or("", RewindWhyNot::as_str),
            shown.to.as_str(),
        ));
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
    let why_not = restore.cannot().map(|why| match why {
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
