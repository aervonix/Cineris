//! What `cargo xtask rebase` does with each file a rebase stopped on: which
//! class it is in, and the resolution or the refusal for it. Pure functions
//! of a path and three texts; the one check that needs a process (rustfmt)
//! is passed in.
//!
//! | Class | Files | Resolution |
//! |---|---|---|
//! | generated | the files `regen` writes ([`GENERATED`]), not the pages written by hand beside them nor the recorded fixtures | the side rebased onto (either would do), then `regen` at the end |
//! | the open items' index | `docs/outstanding/README.md` | the side rebased onto, then `outstanding` at the end |
//! | additions | `.rs` and `.md` files | both sides' additions, when that is all they are, checked |
//! | anything else | | left for a person |
//!
//! **No side is lost silently.** A union is kept only when every line of
//! each side is in it, as many times as the sides' changes come to and in
//! its order ([`keeps_both`]), and it checks: a Rust
//! file's brackets balance and rustfmt leaves it as it is; a Markdown file's
//! added lines are table rows with their table's shape. Anything less is
//! left conflicted, and named, with the reason.

use std::collections::BTreeMap;

use crate::outstanding;
use crate::rebase::merge::{self, Merged};
use crate::regen::WRITERS;

/// The files each generator writes, as `regen` runs them: a path from the
/// top of the repository, and the endings of the files under it that are
/// its (empty: every file under it; a path that does not end in `/` is that
/// file alone). Only these are taken and written again: a page written by
/// hand beside them (`docs/protocol/README.md`,
/// `docs/architecture/tui-look/README.md`) is not, and nor are the provider
/// fixtures recorded from the real APIs, which sit beside the hand-written
/// ones in `crates/cineris-llm/fixtures/`. A test holds each to a place of
/// its generator in [`WRITERS`].
pub(crate) const GENERATED: [(&str, &[&str], &str); 10] = [
    ("docs/protocol/schema/", &[], "protocol"),
    ("docs/protocol/ts/", &[], "protocol"),
    ("docs/configuration/schema.json", &[], "configuration"),
    ("docs/configuration/reference.md", &[], "configuration"),
    ("docs/architecture/tui-look/", &[".ans", ".txt"], "screens"),
    ("crates/cineris-tui/snapshots/", &[], "snapshots"),
    ("crates/cineris-llm/tests/it/snapshots/", &[], "snapshots"),
    ("crates/cineris-index/tests/it/snapshots/", &[], "snapshots"),
    (
        "crates/cineris-config/tests/it/snapshots/",
        &[],
        "snapshots",
    ),
    ("docs/outstanding/README.md", &[], "outstanding"),
];

/// What a conflicted file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// Written by the `regen` generator of this name.
    Generated(&'static str),
    /// The open items' index, which `outstanding` writes.
    Index,
    /// A file whose two sides may both have added lines.
    Additions(Kind),
    /// Anything else.
    Person,
}

/// The kinds of file whose additions are united, each with its check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Rust,
    Markdown,
}

/// The class of the file at `path`, a path from the top of the repository
/// with `/` between its parts.
pub(crate) fn classify(path: &str) -> Class {
    if path == outstanding::INDEX {
        return Class::Index;
    }
    let generated = GENERATED.iter().find(|(place, endings, _)| {
        let under = if place.ends_with('/') {
            path.starts_with(place)
        } else {
            path == *place
        };
        under && (endings.is_empty() || endings.iter().any(|ending| path.ends_with(ending)))
    });
    if let Some((_, _, writer)) = generated {
        // The name as `WRITERS` has it, for `regen`.
        if let Some(writer) = WRITERS.iter().find(|one| one.name == *writer) {
            return Class::Generated(writer.name);
        }
    }
    if path.ends_with(".rs") {
        Class::Additions(Kind::Rust)
    } else if path.ends_with(".md") {
        Class::Additions(Kind::Markdown)
    } else {
        Class::Person
    }
}

/// A conflicted file's three versions, as the index holds them (stages 1, 2
/// and 3): the base, the side being rebased onto (`ours`, in a rebase), and
/// the commit being put on it (`theirs`). `None` where that side has no
/// file: added on both sides, or deleted on one.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Stages {
    pub(crate) base: Option<String>,
    pub(crate) ours: Option<String>,
    pub(crate) theirs: Option<String>,
}

/// What is done with a conflicted file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// The side rebased onto, as it is (git's stage 2, taken whole, by git);
    /// it is written again at the end, by the generator named.
    Taken { then: &'static str },
    /// Both sides' additions, checked.
    United { text: String, places: usize },
    /// Left conflicted, for a person, and why.
    Left(String),
}

/// The resolution of the file at `path` of class `class`. `formatted` says
/// whether a Rust text is as rustfmt leaves it, or what is wrong with it.
pub(crate) fn resolve(
    path: &str,
    class: Class,
    stages: &Stages,
    formatted: &dyn Fn(&str) -> Result<(), String>,
) -> Resolution {
    match class {
        Class::Generated(writer) => take(stages, writer),
        Class::Index => take(stages, "outstanding"),
        Class::Person => Resolution::Left(format!(
            "`{path}` is neither generated nor a Rust or Markdown file: settle it by hand"
        )),
        Class::Additions(kind) => {
            let (Some(base), Some(ours), Some(theirs)) =
                (&stages.base, &stages.ours, &stages.theirs)
            else {
                return Resolution::Left(
                    "one side added the whole file, or deleted it: settle it by hand".to_owned(),
                );
            };
            match unite(kind, base, ours, theirs, formatted) {
                Ok(merged) => Resolution::United {
                    places: merged.unions.len(),
                    text: merged.text,
                },
                Err(why) => Resolution::Left(why),
            }
        }
    }
}

/// The side rebased onto, when it has the file.
fn take(stages: &Stages, then: &'static str) -> Resolution {
    match &stages.ours {
        Some(_) => Resolution::Taken { then },
        None => Resolution::Left(
            "the side rebased onto deleted it: say by hand whether it is still made".to_owned(),
        ),
    }
}

/// Both sides' additions in one text, checked for `kind`; or why not.
pub(crate) fn unite(
    kind: Kind,
    base: &str,
    ours: &str,
    theirs: &str,
    formatted: &dyn Fn(&str) -> Result<(), String>,
) -> Result<Merged, String> {
    let merged = merge::merge(base, ours, theirs)
        .map_err(|refusal| format!("line {}: {}", refusal.line, refusal.why))?;
    if merged.unions.is_empty() {
        // git saw a conflict that this merge does not: say so rather than
        // write a text git would not have.
        return Err("git's merge and this one disagree on where the conflict is".to_owned());
    }
    keeps_both(base, ours, theirs, &merged.text)?;
    match kind {
        Kind::Rust => {
            balanced(&merged.text)?;
            formatted(&merged.text).map_err(|why| format!("rustfmt: {why}"))?;
        }
        Kind::Markdown => rows_keep_their_tables(&merged)?,
    }
    Ok(merged)
}

/// Whether `merged` holds every line of both sides: the check that no side
/// was lost, made on the text that would be written, whatever made it.
/// Line ends are read alike. Three tests, each of which the others miss:
///
/// - **Each line as many times as the two sides' changes come to** (ours'
///   count, and theirs', less the base's): a lost `}` is found though
///   another `}` is still there to stand in for it. A change both sides
///   made alike counts twice, and so is refused too, and left to a person.
/// - **Each side's added lines in their order.**
/// - **The lines of the base that both sides kept, in their order.**
///
/// And the last line ends with a line end when either side's does. Line
/// ends are otherwise read alike: the union is written in ours' (LF in this
/// repository, whose text files are LF everywhere), every line alike.
pub(crate) fn keeps_both(base: &str, ours: &str, theirs: &str, merged: &str) -> Result<(), String> {
    if (ours.ends_with('\n') || theirs.ends_with('\n'))
        && !merged.is_empty()
        && !merged.ends_with('\n')
    {
        return Err("the union's last line lost the line end a side gave it".to_owned());
    }
    let (base, ours, theirs, merged) = (
        merge::lines(base).lines,
        merge::lines(ours).lines,
        merge::lines(theirs).lines,
        merge::lines(merged).lines,
    );
    let mut counts: BTreeMap<&str, [isize; 4]> = BTreeMap::new();
    for (at, text) in [&base, &ours, &theirs, &merged].into_iter().enumerate() {
        for line in text {
            counts.entry(line).or_default()[at] += 1;
        }
    }
    for (line, [base, ours, theirs, merged]) in &counts {
        let expected = ours + theirs - base;
        if *merged != expected {
            return Err(format!(
                "the union has the line `{line}` {merged} time(s) where the two sides' changes \
                 come to {expected}: a line was lost or doubled"
            ));
        }
    }
    let too_far = || "a side is too far from the base to check".to_owned();
    let mine = merge::hunks(&base, &ours).ok_or_else(too_far)?;
    let other = merge::hunks(&base, &theirs).ok_or_else(too_far)?;
    for (side, text, hunks) in [("ours", &ours, &mine), ("theirs", &theirs, &other)] {
        let added: Vec<&str> = hunks
            .iter()
            .flat_map(|hunk| text[hunk.b_start..hunk.b_end].iter().copied())
            .collect();
        if !in_order(&added, &merged) {
            return Err(format!(
                "the union lost a line {side} added, or moved one out of its order"
            ));
        }
    }
    let changed = |hunks: &[merge::Hunk], at: usize| {
        hunks
            .iter()
            .any(|hunk| (hunk.a_start..hunk.a_end).contains(&at))
    };
    let kept: Vec<&str> = (0..base.len())
        .filter(|at| !changed(&mine, *at) && !changed(&other, *at))
        .map(|at| base[at])
        .collect();
    if !in_order(&kept, &merged) {
        return Err("the union moved a line both sides kept out of its order".to_owned());
    }
    Ok(())
}

/// Whether `lines` are all in `text`, in their order, with others between.
fn in_order(lines: &[&str], text: &[&str]) -> bool {
    let mut rest = text.iter();
    lines.iter().all(|line| rest.any(|each| each == line))
}

/// Whether the brackets of a Rust text balance: each `(`, `[` and `{`
/// closed by its own kind, in order, outside comments, strings and
/// character literals. The first that does not, and its line, otherwise.
pub(crate) fn balanced(text: &str) -> Result<(), String> {
    let chars: Vec<char> = text.chars().collect();
    let mut open: Vec<(char, usize)> = Vec::new();
    let mut line = 1;
    let mut at = 0;
    let identifier = |c: char| c.is_alphanumeric() || c == '_';
    while at < chars.len() {
        let c = chars[at];
        let next = chars.get(at + 1).copied();
        match c {
            '\n' => line += 1,
            '/' if next == Some('/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
                continue;
            }
            '/' if next == Some('*') => {
                let mut depth = 0;
                while at < chars.len() {
                    match (chars[at], chars.get(at + 1)) {
                        ('/', Some('*')) => {
                            depth += 1;
                            at += 2;
                        }
                        ('*', Some('/')) => {
                            depth -= 1;
                            at += 2;
                            if depth == 0 {
                                break;
                            }
                        }
                        ('\n', _) => {
                            line += 1;
                            at += 1;
                        }
                        _ => at += 1,
                    }
                }
                if depth != 0 {
                    return Err(format!("line {line}: a comment is never closed"));
                }
                continue;
            }
            'r' if at == 0 || !identifier(chars[at - 1]) || is_prefix(&chars, at) => {
                // A raw string: r"…", r#"…"#, br"…", cr#"…"#.
                let mut hashes = 0;
                let mut after = at + 1;
                while chars.get(after) == Some(&'#') {
                    hashes += 1;
                    after += 1;
                }
                if chars.get(after) == Some(&'"') {
                    at = after + 1;
                    let closed = loop {
                        let Some(&c) = chars.get(at) else {
                            break false;
                        };
                        if c == '\n' {
                            line += 1;
                        }
                        at += 1;
                        if c == '"' && (0..hashes).all(|n| chars.get(at + n) == Some(&'#')) {
                            at += hashes;
                            break true;
                        }
                    };
                    if !closed {
                        return Err(format!("line {line}: a raw string is never closed"));
                    }
                    continue;
                }
            }
            '"' => {
                at += 1;
                loop {
                    match chars.get(at) {
                        None => return Err(format!("line {line}: a string is never closed")),
                        Some('\\') => at += 2,
                        Some('"') => break,
                        Some('\n') => {
                            line += 1;
                            at += 1;
                        }
                        Some(_) => at += 1,
                    }
                }
            }
            '\'' => {
                // A character literal ('x', '\n', '\u{1F600}'), or a lifetime
                // or a label ('a, 'static), which has no closing quote.
                if next == Some('\\') {
                    // The quote, the backslash and the character escaped.
                    at += 3;
                    while at < chars.len() && chars[at] != '\'' && chars[at] != '\n' {
                        at += 1;
                    }
                } else if chars.get(at + 2) == Some(&'\'') {
                    at += 2;
                }
            }
            '(' | '[' | '{' => open.push((c, line)),
            ')' | ']' | '}' => {
                let wanted = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match open.pop() {
                    Some((opened, _)) if opened == wanted => {}
                    Some((opened, from)) => {
                        return Err(format!(
                            "line {line}: `{c}` closes the `{opened}` of line {from}"
                        ));
                    }
                    None => return Err(format!("line {line}: `{c}` closes nothing")),
                }
            }
            _ => {}
        }
        at += 1;
    }
    match open.pop() {
        Some((opened, from)) => Err(format!("line {from}: `{opened}` is never closed")),
        None => Ok(()),
    }
}

/// Whether the `r` at `at` follows a `b` or a `c` that starts a byte or C
/// raw string (`br"…"`, `cr#"…"#`).
fn is_prefix(chars: &[char], at: usize) -> bool {
    matches!(chars.get(at.wrapping_sub(1)), Some('b' | 'c'))
        && (at < 2 || !(chars[at - 2].is_alphanumeric() || chars[at - 2] == '_'))
}

/// Whether a line of Markdown is a table's row.
pub(crate) fn is_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// The cells' dividers of a row: each `|` that no `\` escapes.
pub(crate) fn dividers(row: &str) -> usize {
    let mut count = 0;
    let mut escaped = false;
    for c in row.chars() {
        if c == '|' && !escaped {
            count += 1;
        }
        escaped = c == '\\' && !escaped;
    }
    count
}

/// Whether a row is a table's divider of its header from its body
/// (`|---|:--:|`).
fn is_divider_row(line: &str) -> bool {
    is_row(line)
        && line
            .trim()
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && line.contains('-')
}

/// Whether each line of each place both sides' lines were put side by side
/// in a Markdown text is a row of a table, with as many cells as the table's
/// header: rows added at the end of one table on both sides, or a row added
/// after one the other side reworded. Prose added on both sides at one place
/// is not put together by machine.
pub(crate) fn rows_keep_their_tables(merged: &Merged) -> Result<(), String> {
    let lines = merge::lines(&merged.text).lines;
    for union in &merged.unions {
        let added = union.first.len() + union.second.len();
        for offset in 0..added {
            let at = union.line - 1 + offset;
            let row = lines.get(at).copied().unwrap_or_default();
            if !is_row(row) {
                return Err(format!(
                    "line {}: both sides added text that is not a table's row: `{row}`",
                    at + 1
                ));
            }
            // The table's header: the first row of the rows around this one,
            // followed by its divider.
            let mut first = at;
            while first > 0 && is_row(lines[first - 1]) {
                first -= 1;
            }
            let header = lines[first];
            if !lines
                .get(first + 1)
                .is_some_and(|line| is_divider_row(line))
            {
                return Err(format!(
                    "line {}: the rows both sides added are not in a table with a header",
                    at + 1
                ));
            }
            if dividers(row) != dividers(header) {
                return Err(format!(
                    "line {}: a row of {} cell dividers in a table whose header has {}",
                    at + 1,
                    dividers(row),
                    dividers(header)
                ));
            }
        }
    }
    Ok(())
}
