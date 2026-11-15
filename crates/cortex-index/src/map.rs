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
//! **How files are ranked**, with no model call, in tiers: the files this
//! session's tools touched; then the files of recent sessions (from the
//! statistics, when the core gives them); then the files changed most
//! recently, when the tree's times say anything (a fresh clone's do not),
//! the newest first; then the rest. Since the independent review of step
//! 18, which found this repository's map naming lockfiles, licences and
//! documents and no source file:
//!
//! - **Code first.** In each tier, the files a language's extractor
//!   outlines come before Markdown's, and those before the files the
//!   indentation fallback outlines (configuration, data, prose).
//! - **No noise.** A file Markdown or the fallback outlines and named as a
//!   lockfile, a licence or notice, a changelog, or a merge's or a patch's
//!   leftover (`*.orig`, `*.rej`, which is how a published crate keeps its
//!   original manifest) is never shown: its outline says nothing of the
//!   code's shape. A code file of such a name (`history.py`) is code.
//! - **Spread over the tree.** In the last tier, each directory's first
//!   file comes before any directory's second, and so on, the shallower
//!   directories first in each round; a directory's files in order of how
//!   many top-level definitions they have, then by path. So the map shows
//!   the tree's breadth before one directory's depth.
//!
//! A file with no definitions is never shown.
//!
//! **Byte-stable.** The same cache and the same request give the same map:
//! every order is total and ends at the path, the touched files are a set,
//! and nothing in it is a time, an absolute path or anything the machine
//! has that the cache does not.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use cortex_base::visible;

use crate::cache::{CacheFile, Record, Shown};
use crate::language::Language;
use crate::outline::Kind;
use crate::walk::Partial;

/// The map's default budget, in tokens.
pub const DEFAULT_TOKENS: usize = 1_500;

/// How many bytes a token is counted as.
pub const BYTES_PER_TOKEN: usize = 3;

/// The most of the budget the tree may take, in percent.
pub const TREE_SHARE: usize = 30;

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

/// How many files left at the end of the ranking are tried whole when the
/// next one does not fit beside the line that says what is left out: so
/// that a map that would fit its budget exactly is not cut for that line.
const LAST_FEW: usize = 16;

/// The map of a cache, or `None` when it has nothing to show. `allowed`
/// says whether a path under the workspace may be named: the session's
/// gate, asked again, because the cache is the project's and another
/// session's rules may have let more in.
pub(crate) fn render(
    cache: &CacheFile,
    request: &MapRequest,
    max_files: usize,
    allowed: &dyn Fn(&str) -> bool,
) -> Option<String> {
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
    let tree = tree(cache, (budget - text.len()) * TREE_SHARE / 100, allowed);
    if !tree.is_empty() {
        text.push_str("\nDirectories:\n");
        text.push_str(&tree);
    }

    // The files, most relevant first, each whole or not at all. A line is
    // made only when it is tried, so a map of 20,000 files reads the
    // outlines of the few it shows.
    let showable = rank(cache, request);
    const HEADING: &str = "\nFiles, most relevant first:\n";
    let cut_line = |left: usize| {
        format!(
            "[{left} more {} not shown.]\n",
            if left == 1 { "file" } else { "files" }
        )
    };
    let line_of = |at: usize| {
        let record = &cache.records[at];
        allowed(&record.path)
            .then(|| file_line(cache, record))
            .flatten()
    };
    let room = budget.saturating_sub(text.len() + HEADING.len());
    let mut body = String::new();
    let mut tried = 0;
    while tried < showable.len() {
        let Some(line) = line_of(showable[tried]) else {
            tried += 1;
            continue;
        };
        let left = showable.len() - tried - 1;
        let reserve = if left == 0 { 0 } else { cut_line(left).len() };
        if body.len() + line.len() + 1 + reserve <= room {
            body.push_str(&line);
            body.push('\n');
            tried += 1;
            continue;
        }
        // It does not fit beside the cut line: near the end, all that is
        // left may fit without it.
        if showable.len() - tried <= LAST_FEW {
            let rest: String = showable[tried..]
                .iter()
                .filter_map(|&at| line_of(at))
                .map(|line| line + "\n")
                .collect();
            if body.len() + rest.len() <= room {
                body.push_str(&rest);
                tried = showable.len();
            }
        }
        break;
    }
    if !body.is_empty() {
        text.push_str(HEADING);
        text.push_str(&body);
    }
    let left = showable.len() - tried;
    if left > 0 {
        let cut = cut_line(left);
        if text.len() + cut.len() <= budget {
            text.push_str(&cut);
        }
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

/// Whole levels of directories, as deep as fit in `room` bytes: the ones
/// `allowed` lets through, with every directory above them.
fn tree(cache: &CacheFile, room: usize, allowed: &dyn Fn(&str) -> bool) -> String {
    let mut known: std::collections::BTreeMap<&str, bool> = std::collections::BTreeMap::new();
    let mut dirs: Vec<(&str, usize)> = Vec::new();
    for (dir, files) in &cache.dirs {
        if dir.is_empty() {
            continue;
        }
        // The cache lists a directory after the ones above it.
        let parent_ok = dir
            .rsplit_once('/')
            .is_none_or(|(parent, _)| known.get(parent).copied().unwrap_or(false));
        let ok = parent_ok && allowed(dir);
        known.insert(dir.as_str(), ok);
        if ok {
            dirs.push((dir.as_str(), *files));
        }
    }
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
            let name = visible(dir.rsplit('/').next().unwrap_or(dir));
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

/// What kind of outline a file has, in the order the map prefers them: a
/// language's extractor's, Markdown's headings, the indentation fallback's.
fn class(record: &Record) -> u8 {
    match record.language {
        Some(Language::Markdown) => 1,
        Some(_) => 0,
        None => 2,
    }
}

/// Whether a file is one the map never shows: outlined by Markdown or the
/// fallback, and named as a lockfile, a licence or a notice, a changelog,
/// or the leftover of a merge or a patch.
fn noise(record: &Record) -> bool {
    if class(record) == 0 {
        return false;
    }
    let name = record
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&record.path)
        .to_ascii_lowercase();
    let lockfile = [
        ".lock",
        ".lockb",
        ".lockfile",
        ".lock.json",
        "-lock.json",
        "-lock.yaml",
    ]
    .iter()
    .any(|end| name.ends_with(end))
        || matches!(name.as_str(), "go.sum" | "npm-shrinkwrap.json");
    let leftover = name.ends_with(".orig") || name.ends_with(".rej");
    // The name before its first `.`, `-` or `_`: `LICENSE-APACHE`,
    // `CHANGELOG.md`, `COPYING.LESSER`.
    let stem = name.split(['.', '-', '_']).next().unwrap_or("");
    let named = matches!(
        stem,
        "license"
            | "licence"
            | "unlicense"
            | "copying"
            | "copyright"
            | "notice"
            | "changelog"
            | "changes"
            | "history"
            | "news"
    );
    lockfile || leftover || named
}

/// The positions of the records the map may show, most relevant first.
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
    let mut changed_rank = vec![usize::MAX; cache.records.len()];
    for (rank, &at) in changed.iter().enumerate() {
        changed_rank[at] = rank;
    }

    let tier = |at: usize| {
        let record = &cache.records[at];
        if touched.contains(record.path.as_str()) {
            0
        } else if recent.contains(record.path.as_str()) {
            1
        } else if changed_rank[at] != usize::MAX {
            2
        } else {
            3
        }
    };
    let mut order: Vec<usize> = (0..cache.records.len())
        .filter(|&at| cache.records[at].top > 0 && !noise(&cache.records[at]))
        .collect();

    // The last tier spread over the tree: each file's round is its place
    // among its directory's files of its class, the most definitions first.
    let mut round = vec![0usize; cache.records.len()];
    let mut by_directory: std::collections::BTreeMap<(&str, u8), Vec<usize>> =
        std::collections::BTreeMap::new();
    for &at in &order {
        if tier(at) == 3 {
            let record = &cache.records[at];
            let directory = record.path.rsplit_once('/').map_or("", |(dir, _)| dir);
            by_directory
                .entry((directory, class(record)))
                .or_default()
                .push(at);
        }
    }
    for files in by_directory.values_mut() {
        files.sort_by(|&one, &other| {
            cache.records[other]
                .top
                .cmp(&cache.records[one].top)
                .then(one.cmp(&other))
        });
        for (place, &at) in files.iter().enumerate() {
            round[at] = place;
        }
    }

    order.sort_by_cached_key(|&at| {
        let record = &cache.records[at];
        (
            tier(at),
            class(record),
            changed_rank[at],
            round[at],
            record.path.split('/').count(),
            at,
        )
    });
    order
}

/// A file's line of the map: its path and its main definitions. `None` for
/// a file with nothing to show (no definitions), or whose definitions'
/// lines in the cache this build did not write.
fn file_line(cache: &CacheFile, record: &Record) -> Option<String> {
    if record.top == 0 {
        return None;
    }
    let shown = cache.shown(record)?;
    let mut line = format!("{}:", visible(&record.path));
    for (count, entry) in shown.iter().enumerate() {
        line.push_str(if count == 0 { " " } else { ", " });
        line.push_str(&named(entry));
        if entry.members > 0 {
            line.push_str(" {");
            for (index, member) in entry.names.iter().enumerate() {
                line.push_str(if index == 0 { " " } else { ", " });
                line.push_str(&visible(member));
            }
            if entry.members > entry.names.len() {
                let _ = write!(line, ", +{}", entry.members - entry.names.len());
            }
            line.push_str(" }");
        }
    }
    if record.top > shown.len() {
        let _ = write!(line, ", +{} more", record.top - shown.len());
    }
    Some(line)
}

/// A top-level definition as the map names it: its kind's word and its
/// name, or a heading's text.
fn named(entry: &Shown) -> String {
    match entry.kind {
        Kind::Section | Kind::Block => visible(&entry.name).into_owned(),
        kind => format!("{} {}", kind.word(), visible(&entry.name)),
    }
}
