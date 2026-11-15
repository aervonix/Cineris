//! The map of a repository: its directories, and its most relevant files
//! with their main definitions, cut to a token budget
//! (`docs/architecture/15-efficiency.md`, "Repository map").
//!
//! **How the budget is spent.** The budget is in tokens, counted at
//! [`BYTES_PER_TOKEN`] bytes each, the loop's own ratio before a provider
//! has counted anything. A fixed lead says what the map is. The tree takes
//! at most [`TREE_SHARE`] of the rest: whole levels of directories, each
//! with how many files it holds, as deep as fit. The files take what is
//! left, most relevant first, each file whole or not at all, and the map
//! says how many were left out.
//!
//! **How files are ranked**, with no model call: the files this session's
//! tools touched; then the files of recent sessions (from the statistics,
//! when the core gives them); then the files changed most recently, when
//! the tree's times say anything (a fresh clone's do not); then files with
//! definitions before files without, shallower before deeper, and by path.
//!
//! **Byte-stable.** The same cache and the same request give the same map:
//! every order is total and ends at the path, the touched files are a set,
//! and nothing in it is a time, an absolute path or anything the machine
//! has that the cache does not.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::cache::{CacheFile, Record};
use crate::outline::{Entry, Kind};
use crate::walk::Partial;

/// The map's default budget, in tokens.
pub const DEFAULT_TOKENS: usize = 1_500;

/// How many bytes a token is counted as.
pub const BYTES_PER_TOKEN: usize = 3;

/// The most of the budget the tree may take, in percent.
pub const TREE_SHARE: usize = 30;

/// How many of a file's top-level definitions are named, and how many of
/// each one's members.
const TOP_ENTRIES: usize = 12;
const MEMBERS: usize = 8;

/// How many files count as changed recently, at most.
const RECENT_FILES: usize = 30;

/// How far a file's time must be ahead of the tree's middle to count as
/// changed recently, and how far behind the newest it may be.
const RECENT_LEAD_NANOS: i128 = 60 * 1_000_000_000;
const RECENT_WINDOW_NANOS: i128 = 7 * 24 * 3600 * 1_000_000_000;

/// What a map is ranked by, and how large it may be.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapRequest {
    /// The files this session's tools touched, under the workspace with
    /// `/`. Their order does not matter.
    pub touched: Vec<String>,
    /// The files of recent sessions, the same way.
    pub recent: Vec<String>,
    /// The budget in tokens.
    pub tokens: usize,
}

impl MapRequest {
    /// A request with nothing to rank by but the tree, at the default
    /// budget.
    #[must_use]
    pub fn new() -> Self {
        Self {
            touched: Vec::new(),
            recent: Vec::new(),
            tokens: DEFAULT_TOKENS,
        }
    }
}

/// Where the files that are recent sessions' come from: the statistics
/// (step 16). The core hands the map what it gives.
pub trait RecentFiles: Send + Sync {
    /// The files of recent sessions, under the workspace with `/`, most
    /// recent first.
    fn recent(&self) -> Vec<String>;
}

/// No statistics: no recent files.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRecentFiles;

impl RecentFiles for NoRecentFiles {
    fn recent(&self) -> Vec<String> {
        Vec::new()
    }
}

const LEAD: &str = "Repository map, made by the harness from the workspace's files \
(approximate: definitions are found by reading lines, not by parsing). Directories with \
how many files they hold, then the most relevant files with their main definitions. \
`outline` shows a file's or a directory's definitions with their lines.";

/// The map of a cache, or `None` when it has nothing to show.
pub(crate) fn render(cache: &CacheFile, request: &MapRequest, max_files: usize) -> Option<String> {
    if cache.records.is_empty() {
        return None;
    }
    let budget = request.tokens.saturating_mul(BYTES_PER_TOKEN);
    let mut text = String::new();
    text.push_str(LEAD);
    text.push('\n');
    let _ = writeln!(text, "{}", counts(cache, max_files));
    if text.len() >= budget {
        return None;
    }

    // The tree, up to its share.
    let tree = tree(cache, (budget - text.len()) * TREE_SHARE / 100);
    if !tree.is_empty() {
        text.push_str("\nDirectories:\n");
        text.push_str(&tree);
    }

    // The files, most relevant first, each whole or not at all.
    let ranked = rank(cache, request);
    let lines: Vec<String> = ranked
        .iter()
        .filter_map(|&at| file_line(cache, &cache.records[at]))
        .collect();
    if lines.is_empty() {
        return Some(text);
    }
    const HEADING: &str = "\nFiles, most relevant first:\n";
    let all: usize = HEADING.len() + lines.iter().map(|line| line.len() + 1).sum::<usize>();
    if text.len() + all <= budget {
        text.push_str(HEADING);
        for line in &lines {
            text.push_str(line);
            text.push('\n');
        }
        return Some(text);
    }
    // Not all fit: keep room for the line that says so.
    let cut_line = |left: usize| {
        format!(
            "[{left} more {} not shown: `outline` or `list` reach them.]\n",
            if left == 1 { "file" } else { "files" }
        )
    };
    let reserve = cut_line(lines.len()).len();
    let mut shown = 0;
    let mut body = String::new();
    for line in &lines {
        if text.len() + HEADING.len() + body.len() + line.len() + 1 + reserve > budget {
            break;
        }
        body.push_str(line);
        body.push('\n');
        shown += 1;
    }
    if shown > 0 {
        text.push_str(HEADING);
        text.push_str(&body);
    }
    let cut = cut_line(lines.len() - shown);
    if text.len() + cut.len() <= budget {
        text.push_str(&cut);
    }
    Some(text)
}

/// What the index covers, and why not all, when it does not.
fn counts(cache: &CacheFile, max_files: usize) -> String {
    let mut line = format!(
        "{} {} indexed",
        cache.indexed,
        if cache.indexed == 1 { "file" } else { "files" }
    );
    if cache.partial.is_empty() {
        line.push('.');
        return line;
    }
    let _ = write!(line, " of {} found. The index is partial:", cache.found);
    for why in &cache.partial {
        line.push(' ');
        line.push_str(match why {
            Partial::Files => "",
            Partial::Listing => {
                "the listing stopped at its cap, so the map covers the part of the tree it reached."
            }
            Partial::Time | Partial::Stopped => {
                "some files are not read yet; a later map has them."
            }
        });
        if *why == Partial::Files {
            let _ = write!(
                line,
                "past the cap of {max_files} files, the map covers the ones nearest the top of the tree."
            );
        }
    }
    line
}

/// Whole levels of directories, as deep as fit in `room` bytes.
fn tree(cache: &CacheFile, room: usize) -> String {
    let dirs: Vec<(&str, usize)> = cache
        .dirs
        .iter()
        .filter(|(dir, _)| !dir.is_empty())
        .map(|(dir, files)| (dir.as_str(), *files))
        .collect();
    let deepest = dirs
        .iter()
        .map(|(dir, _)| dir.split('/').count())
        .max()
        .unwrap_or(0);
    let render = |depth: usize| -> String {
        let mut out = String::new();
        for (dir, files) in &dirs {
            let level = dir.split('/').count();
            if level > depth {
                continue;
            }
            let name = dir.rsplit('/').next().unwrap_or(dir);
            let _ = writeln!(
                out,
                "{:indent$}{name}/ ({files})",
                "",
                indent = (level - 1) * 2
            );
        }
        out
    };
    let mut best = String::new();
    for depth in 1..=deepest {
        let candidate = render(depth);
        if candidate.len() > room {
            break;
        }
        best = candidate;
    }
    if best.is_empty() && deepest > 0 {
        // Not even the top level fits whole: as much of it as does.
        let top = render(1);
        let mut shown = String::new();
        let total = top.lines().count();
        for (count, line) in top.lines().enumerate() {
            let more = format!("[{} more directories]\n", total - count);
            if shown.len() + line.len() + 1 + more.len() > room {
                if !shown.is_empty() || more.len() <= room {
                    shown.push_str(&more);
                }
                break;
            }
            shown.push_str(line);
            shown.push('\n');
        }
        best = shown;
    }
    best
}

/// The records' positions, most relevant first.
fn rank(cache: &CacheFile, request: &MapRequest) -> Vec<usize> {
    let touched: BTreeSet<&str> = request.touched.iter().map(String::as_str).collect();
    let recent: BTreeSet<&str> = request.recent.iter().map(String::as_str).collect();

    // Changed recently: well ahead of the tree's middle, and not far
    // behind the newest. A tree whose files all have one time (a fresh
    // clone) has none.
    let mut times: Vec<i128> = cache
        .records
        .iter()
        .map(|record| record.stamp.modified)
        .collect();
    times.sort_unstable();
    let middle = times.get(times.len() / 2).copied().unwrap_or(0);
    let newest = times.last().copied().unwrap_or(0);
    let mut changed: Vec<usize> = (0..cache.records.len())
        .filter(|&at| {
            let modified = cache.records[at].stamp.modified;
            modified >= middle.saturating_add(RECENT_LEAD_NANOS)
                && modified >= newest.saturating_sub(RECENT_WINDOW_NANOS)
        })
        .collect();
    changed.sort_by(|&one, &other| {
        let (one, other) = (&cache.records[one], &cache.records[other]);
        other
            .stamp
            .modified
            .cmp(&one.stamp.modified)
            .then_with(|| crate::walk::path_order(&one.path, &other.path))
    });
    changed.truncate(RECENT_FILES);
    let changed_rank = |at: usize| changed.iter().position(|&known| known == at);

    let mut order: Vec<usize> = (0..cache.records.len()).collect();
    order.sort_by_key(|&at| {
        let record = &cache.records[at];
        let tier = if touched.contains(record.path.as_str()) {
            0
        } else if recent.contains(record.path.as_str()) {
            1
        } else if changed_rank(at).is_some() {
            2
        } else {
            3
        };
        (
            tier,
            changed_rank(at).unwrap_or(usize::MAX),
            usize::from(record.top == 0),
            record.path.split('/').count(),
            at,
        )
    });
    order
}

/// A file's line of the map: its path and its main definitions. `None` for
/// a file with nothing to show (no definitions).
fn file_line(cache: &CacheFile, record: &Record) -> Option<String> {
    if record.entries == 0 {
        return None;
    }
    let outline = cache.outline_of(record);
    let mut line = format!("{}:", record.path);
    let mut top: Vec<(&Entry, Vec<&Entry>)> = Vec::new();
    for entry in &outline.entries {
        match entry.depth {
            0 => top.push((entry, Vec::new())),
            1 => {
                if let Some((_, members)) = top.last_mut() {
                    members.push(entry);
                }
            }
            _ => {}
        }
    }
    let shown = top.len().min(TOP_ENTRIES);
    for (count, (entry, members)) in top.iter().take(TOP_ENTRIES).enumerate() {
        line.push_str(if count == 0 { " " } else { ", " });
        line.push_str(&named(entry));
        if !members.is_empty() {
            line.push_str(" {");
            for (index, member) in members.iter().take(MEMBERS).enumerate() {
                line.push_str(if index == 0 { " " } else { ", " });
                line.push_str(&member.name);
            }
            if members.len() > MEMBERS {
                let _ = write!(line, ", +{}", members.len() - MEMBERS);
            }
            line.push_str(" }");
        }
    }
    if top.len() > shown {
        let _ = write!(line, ", +{} more", top.len() - shown);
    }
    Some(line)
}

/// A top-level definition as the map names it: its kind's word and its
/// name, or a heading's text.
fn named(entry: &Entry) -> String {
    match entry.kind {
        Kind::Section | Kind::Block => entry.name.clone(),
        kind => format!("{} {}", kind.word(), entry.name),
    }
}
