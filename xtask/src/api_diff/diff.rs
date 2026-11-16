//! Comparing two lists of public items, and writing what differs
//! (`docs/testing-and-ci.md`, "The public API between two trees").

use std::collections::BTreeSet;

use super::read::{Api, Entry, Key};

/// What happened to an item between the two trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Added,
    Removed,
    Changed,
}

/// One difference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Delta {
    pub(crate) key: Key,
    pub(crate) change: Change,
    pub(crate) before: Option<Entry>,
    pub(crate) after: Option<Entry>,
}

/// The items added, removed and changed from `before` to `after`, in the
/// order of their paths.
///
/// Changed is a different signature or a different kind of item, judged on
/// [`Entry::identity`]. An `impl` (or what an `impl` fixes) of a type or
/// trait that was itself added or removed in the same change is left out:
/// it comes with the item, and a new type with its derives would otherwise
/// be a dozen lines of `impl`.
pub(crate) fn compare(before: &Api, after: &Api) -> Vec<Delta> {
    let added_paths: BTreeSet<&str> = after
        .keys()
        .filter(|key| !before.contains_key(key))
        .map(|key| key.path.as_str())
        .collect();
    let removed_paths: BTreeSet<&str> = before
        .keys()
        .filter(|key| !after.contains_key(key))
        .map(|key| key.path.as_str())
        .collect();

    let mut deltas = Vec::new();
    for (key, entry) in after {
        match before.get(key) {
            None => {
                let with_its_owner = entry
                    .owner
                    .as_deref()
                    .is_some_and(|owner| added_paths.contains(owner));
                if !with_its_owner {
                    deltas.push(Delta {
                        key: key.clone(),
                        change: Change::Added,
                        before: None,
                        after: Some(entry.clone()),
                    });
                }
            }
            Some(old) if old.kind != entry.kind || old.identity != entry.identity => {
                deltas.push(Delta {
                    key: key.clone(),
                    change: Change::Changed,
                    before: Some(old.clone()),
                    after: Some(entry.clone()),
                });
            }
            Some(_) => {}
        }
    }
    for (key, entry) in before {
        if after.contains_key(key) {
            continue;
        }
        let with_its_owner = entry
            .owner
            .as_deref()
            .is_some_and(|owner| removed_paths.contains(owner));
        if !with_its_owner {
            deltas.push(Delta {
                key: key.clone(),
                change: Change::Removed,
                before: Some(entry.clone()),
                after: None,
            });
        }
    }
    deltas.sort_by(|a, b| a.key.cmp(&b.key));
    deltas
}

/// One crate's differences, for a report. `before` is `None` for a crate
/// the other tree does not have.
#[derive(Debug)]
pub(crate) struct CrateDeltas {
    pub(crate) name: String,
    /// What is odd about the comparison: a crate in one tree only.
    pub(crate) note: Option<String>,
    pub(crate) deltas: Vec<Delta>,
}

fn counts(deltas: &[Delta]) -> String {
    let count = |change: Change| deltas.iter().filter(|delta| delta.change == change).count();
    format!(
        "{} added, {} removed, {} changed",
        count(Change::Added),
        count(Change::Removed),
        count(Change::Changed)
    )
}

fn word(change: Change) -> &'static str {
    match change {
        Change::Added => "added",
        Change::Removed => "removed",
        Change::Changed => "changed",
    }
}

fn shown(entry: &Option<Entry>) -> &str {
    entry.as_ref().map_or("", |entry| entry.shown.as_str())
}

/// The crates with no difference, to be named on one line.
fn unchanged(crates: &[CrateDeltas]) -> Vec<&str> {
    crates
        .iter()
        .filter(|krate| krate.deltas.is_empty())
        .map(|krate| krate.name.as_str())
        .collect()
}

fn with_note(krate: &CrateDeltas) -> String {
    krate
        .note
        .as_ref()
        .map_or(String::new(), |note| format!(" ({note})"))
}

/// The report as text: one line for an item added or removed, three for
/// one changed. The crates with no difference are named on the last line.
pub(crate) fn text(heading: &str, crates: &[CrateDeltas]) -> String {
    let mut out = format!("{heading}\n");
    for krate in crates.iter().filter(|krate| !krate.deltas.is_empty()) {
        out.push_str(&format!(
            "{}: {}{}\n",
            krate.name,
            counts(&krate.deltas),
            with_note(krate)
        ));
        for delta in &krate.deltas {
            let path = &delta.key.path;
            match delta.change {
                Change::Added => out.push_str(&format!("  + {path}  {}\n", shown(&delta.after))),
                Change::Removed => {
                    out.push_str(&format!("  - {path}  {}\n", shown(&delta.before)));
                }
                Change::Changed => out.push_str(&format!(
                    "  ~ {path}\n      was  {}\n      now  {}\n",
                    shown(&delta.before),
                    shown(&delta.after)
                )),
            }
        }
    }
    let quiet = unchanged(crates);
    if !quiet.is_empty() {
        out.push_str(&format!("no public API changes: {}\n", quiet.join(", ")));
    }
    out
}

/// A cell of a Markdown table holding code.
fn code(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else {
        format!("`{}`", text.replace('|', "\\|"))
    }
}

/// The report as Markdown: a table for each crate with changes.
pub(crate) fn markdown(heading: &str, crates: &[CrateDeltas]) -> String {
    let mut out = format!("{heading}\n\n");
    for krate in crates.iter().filter(|krate| !krate.deltas.is_empty()) {
        out.push_str(&format!(
            "**{}**: {}{}\n\n| | Item | Before | After |\n|---|---|---|---|\n",
            krate.name,
            counts(&krate.deltas),
            with_note(krate)
        ));
        for delta in &krate.deltas {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                word(delta.change),
                code(&delta.key.path),
                code(shown(&delta.before)),
                code(shown(&delta.after))
            ));
        }
        out.push('\n');
    }
    let quiet = unchanged(crates);
    if !quiet.is_empty() {
        out.push_str(&format!("No public API changes: {}.\n", quiet.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests;
