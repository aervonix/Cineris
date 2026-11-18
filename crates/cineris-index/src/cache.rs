//! The on-disk cache of a project's outlines: one file,
//! `<state>/projects/<project key>/index/outlines`, owner-only.
//!
//! **It holds what the map shows of each file, and no more:** its stamp,
//! and of its outline the first [`TOP_ENTRIES`] top-level definitions, each
//! with how many members it has and the names of the first [`MEMBERS`]. A
//! file's whole outline, with its lines, heads and docs, is made from the
//! file as it is now when one is asked for (`Index::outline_of`). The first
//! format kept whole outlines: 4.7 KiB a file on real code, and reading it
//! for a pass and again for the map cost a warm map 2.7 times its budget
//! (found by the independent review of step 18).
//!
//! It is text, one record a line, fields split by tabs (a tab, a line
//! break, a carriage return or a backslash in a field is escaped):
//!
//! ```text
//! cineris-index outlines <format> <extractors>
//! R  <the workspace root it was made for>
//! D  <directory>  <files under it>                       one per directory
//! F  <path>  <size>  <modified>  <change>  <flags>  <language>  <top>
//! O  <kind>  <name>  <members>  <member>…     its first top-level definitions
//! S  <files found>  <files indexed>  <why it is partial, or ->
//! E  <files>
//! ```
//!
//! - **`top`** is how many top-level definitions the file's outline has; it
//!   has one `O` line for each of the first [`TOP_ENTRIES`]. **`members`**
//!   is how many definitions one level in follow that one, and the line
//!   names the first [`MEMBERS`] of them.
//! - **The files are in [`path_order`]**, so a pass reads the cache and the
//!   listing side by side. The summary comes after them, so that a pass
//!   writes the file as it goes.
//! - **A cache this build cannot trust is no cache:** another format or
//!   another version of the extractors, another root, a record out of
//!   order, a file with another number of `O` lines than its `top` says, or
//!   no `E` line (cut short). It is read as empty, and the next pass writes
//!   a whole one.
//! - **An `O` line is read when its file is shown,** not when the cache is
//!   read: a pass needs the stamps alone, and a map shows a few dozen files
//!   of thousands. A file whose `O` lines this build did not write is left
//!   out of the map, and kept as it is by the passes until it changes.
//! - **The flags:** `u` unsettled (modified within two seconds of the pass
//!   that read it, or in the future: it is read again by every pass until
//!   it settles), `b` binary, `L` larger than the cap (listed, not read),
//!   `t` an outline cut at its cap.

#![allow(
    clippy::disallowed_methods,
    reason = "the cache is read and written on the blocking pool"
)]

use std::fmt::Write as _;
use std::ops::Range;
use std::path::Path;

use crate::language::Language;
use crate::outline::{Kind, Outline};
use crate::walk::{Partial, Stamp, path_order};

/// The version of the cache's format.
pub(crate) const FORMAT: u32 = 2;

/// The version of the extractors. Raised whenever an extractor finds
/// something else in the same text, so that outlines cached by an older
/// build are made again.
pub(crate) const EXTRACTORS: u32 = 2;

const HEADER: &str = "cineris-index outlines";

/// How many of a file's top-level definitions the cache keeps, and the map
/// names.
pub(crate) const TOP_ENTRIES: usize = 12;

/// How many of a top-level definition's members the cache names.
pub(crate) const MEMBERS: usize = 8;

/// What is known of a file beside its stamp.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub(crate) unsettled: bool,
    pub(crate) binary: bool,
    pub(crate) large: bool,
    pub(crate) truncated: bool,
}

impl Flags {
    fn text(self) -> String {
        let mut text = String::new();
        for (on, letter) in [
            (self.unsettled, 'u'),
            (self.binary, 'b'),
            (self.large, 'L'),
            (self.truncated, 't'),
        ] {
            if on {
                text.push(letter);
            }
        }
        if text.is_empty() {
            text.push('-');
        }
        text
    }

    fn parse(text: &str) -> Option<Self> {
        let mut flags = Self::default();
        if text == "-" {
            return Some(flags);
        }
        for letter in text.chars() {
            match letter {
                'u' => flags.unsettled = true,
                'b' => flags.binary = true,
                'L' => flags.large = true,
                't' => flags.truncated = true,
                _ => return None,
            }
        }
        Some(flags)
    }
}

/// One file of the cache.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    pub(crate) path: String,
    pub(crate) stamp: Stamp,
    pub(crate) flags: Flags,
    pub(crate) language: Option<Language>,
    /// How many top-level definitions its outline has.
    pub(crate) top: usize,
    /// Its `F` line and its `O` lines, as bytes of the cache's text.
    pub(crate) lines: Range<usize>,
    /// Its `O` lines alone.
    pub(crate) outline: Range<usize>,
}

/// A top-level definition as the cache keeps it: what the map shows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shown {
    pub(crate) kind: Kind,
    pub(crate) name: String,
    /// How many definitions one level in follow it.
    pub(crate) members: usize,
    /// The names of the first [`MEMBERS`] of them.
    pub(crate) names: Vec<String>,
}

/// A cache, read and checked.
#[derive(Debug)]
pub(crate) struct CacheFile {
    text: String,
    pub(crate) found: usize,
    pub(crate) indexed: usize,
    pub(crate) partial: Vec<Partial>,
    pub(crate) dirs: Vec<(String, usize)>,
    pub(crate) records: Vec<Record>,
}

/// A field as the cache stores it.
fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
}

/// A field as it was before [`escape`].
fn unescape(text: &str) -> Option<String> {
    if !text.contains('\\') {
        return Some(text.to_owned());
    }
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        out.push(match characters.next()? {
            '\\' => '\\',
            't' => '\t',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(out)
}

fn stamp_text(stamp: &Stamp) -> String {
    let change = stamp
        .change
        .map_or_else(|| "-".to_owned(), |(inode, at)| format!("{inode}:{at}"));
    format!("{}\t{}\t{change}", stamp.size, stamp.modified)
}

fn parse_change(text: &str) -> Option<Option<(u64, i128)>> {
    if text == "-" {
        return Some(None);
    }
    let (inode, at) = text.split_once(':')?;
    Some(Some((inode.parse().ok()?, at.parse().ok()?)))
}

/// A record being read: its `O` lines are counted, not read.
struct Open {
    record: Record,
    /// How many `O` lines it has still to come.
    left: usize,
}

impl Open {
    /// The record, if all its `O` lines came.
    fn close(self) -> Option<Record> {
        (self.left == 0).then_some(self.record)
    }
}

impl CacheFile {
    /// The cache at `path`, if there is one this build can trust for the
    /// workspace at `root`.
    pub(crate) fn read(path: &Path, root: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        Self::parse(text, root)
    }

    /// Reads and checks a cache's text: every line but the `O` lines, which
    /// are counted ([`CacheFile::shown`] reads them).
    pub(crate) fn parse(text: String, root: &str) -> Option<Self> {
        let mut lines = LineIter { text: &text, at: 0 };
        let (header, _) = lines.next()?;
        if header != format!("{HEADER} {FORMAT} {EXTRACTORS}") {
            return None;
        }
        let (root_line, _) = lines.next()?;
        if unescape(root_line.strip_prefix("R\t")?)? != root {
            return None;
        }
        let mut dirs = Vec::new();
        let mut records: Vec<Record> = Vec::new();
        let mut open: Option<Open> = None;
        let mut summary: Option<(usize, usize, Vec<Partial>)> = None;
        let mut ended = false;
        for (line, range) in lines {
            if ended {
                return None;
            }
            // The most lines by far, and only counted.
            if line.starts_with("O\t") {
                let record = open.as_mut()?;
                record.left = record.left.checked_sub(1)?;
                record.record.lines.end = range.end;
                record.record.outline.end = range.end;
                continue;
            }
            let (kind, rest) = line.split_once('\t')?;
            if summary.is_some() && kind != "E" {
                return None;
            }
            match kind {
                "D" if records.is_empty() && open.is_none() => {
                    let (dir, count) = rest.split_once('\t')?;
                    dirs.push((unescape(dir)?, count.parse().ok()?));
                }
                "F" => {
                    if let Some(done) = open.take() {
                        records.push(done.close()?);
                    }
                    let record = parse_file(rest, range)?;
                    if records
                        .last()
                        .is_some_and(|last| path_order(&last.path, &record.path).is_ge())
                    {
                        return None;
                    }
                    open = Some(Open {
                        left: record.top.min(TOP_ENTRIES),
                        record,
                    });
                }
                "S" => {
                    if let Some(done) = open.take() {
                        records.push(done.close()?);
                    }
                    let mut fields = rest.split('\t');
                    let found: usize = fields.next()?.parse().ok()?;
                    let indexed: usize = fields.next()?.parse().ok()?;
                    let partial_text = fields.next()?;
                    let partial: Vec<Partial> = if partial_text == "-" {
                        Vec::new()
                    } else {
                        partial_text
                            .chars()
                            .map(Partial::from_letter)
                            .collect::<Option<_>>()?
                    };
                    if fields.next().is_some() || indexed != records.len() {
                        return None;
                    }
                    summary = Some((found, indexed, partial));
                }
                "E" => {
                    if summary.is_none() || rest.parse::<usize>().ok()? != records.len() {
                        return None;
                    }
                    ended = true;
                }
                _ => return None,
            }
        }
        if !ended {
            return None;
        }
        let (found, indexed, partial) = summary?;
        Some(Self {
            text,
            found,
            indexed,
            partial,
            dirs,
            records,
        })
    }

    /// The record of the file at `path`, under the workspace with `/`.
    #[cfg(test)]
    pub(crate) fn find(&self, path: &str) -> Option<&Record> {
        self.records
            .binary_search_by(|record| path_order(&record.path, path))
            .ok()
            .map(|at| &self.records[at])
    }

    /// The cache's whole text, as it is written.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// A record's lines as they stand in the cache, to be written again
    /// unchanged.
    pub(crate) fn lines_of(&self, record: &Record) -> &str {
        &self.text[record.lines.clone()]
    }

    /// What the map shows of a record's outline: its first top-level
    /// definitions. `None` when one of its `O` lines is not one this build
    /// writes.
    pub(crate) fn shown(&self, record: &Record) -> Option<Vec<Shown>> {
        LineIter {
            text: &self.text[record.outline.clone()],
            at: 0,
        }
        .map(|(line, _)| parse_shown(line.strip_prefix("O\t")?))
        .collect()
    }
}

/// An `F` line's fields, after its `F`.
fn parse_file(rest: &str, range: Range<usize>) -> Option<Record> {
    let mut fields = rest.split('\t');
    let path = unescape(fields.next()?)?;
    let size = fields.next()?.parse().ok()?;
    let modified = fields.next()?.parse().ok()?;
    let change = parse_change(fields.next()?)?;
    let flags = Flags::parse(fields.next()?)?;
    let language = match fields.next()? {
        "-" => None,
        name => Some(Language::from_name(name)?),
    };
    let top = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some(Record {
        path,
        stamp: Stamp {
            size,
            modified,
            change,
        },
        flags,
        language,
        top,
        lines: range.clone(),
        outline: range.end..range.end,
    })
}

/// An `O` line's fields, after its `O`.
fn parse_shown(rest: &str) -> Option<Shown> {
    let mut fields = rest.split('\t');
    let kind = Kind::from_word(fields.next()?)?;
    let name = unescape(fields.next()?)?;
    let members: usize = fields.next()?.parse().ok()?;
    let names = fields.map(unescape).collect::<Option<Vec<_>>>()?;
    (names.len() == members.min(MEMBERS)).then_some(Shown {
        kind,
        name,
        members,
        names,
    })
}

/// The lines of a text with the range of bytes each takes, its line break
/// included. A last line with no line break is not a line: the cache always
/// ends with one, so such a line is what a cut left.
struct LineIter<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Iterator for LineIter<'a> {
    type Item = (&'a str, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let rest = &self.text[self.at..];
        let end = rest.find('\n')?;
        let start = self.at;
        self.at += end + 1;
        Some((&rest[..end], start..self.at))
    }
}

/// A file's record as a pass made it, before its place in a cache is known.
#[derive(Debug)]
pub(crate) struct Made {
    /// Its `F` line and its `O` lines.
    text: String,
    /// Where its `O` lines begin in `text`.
    outline_at: usize,
    path: String,
    stamp: Stamp,
    flags: Flags,
    language: Option<Language>,
    top: usize,
}

/// A file's record as the cache writes it, from what a pass found: its
/// stamp, and what the map shows of its outline.
pub(crate) fn record(path: &str, stamp: &Stamp, flags: Flags, outline: &Outline) -> Made {
    let top = outline
        .entries
        .iter()
        .filter(|entry| entry.depth == 0)
        .count();
    let mut text = String::new();
    text.push_str("F\t");
    escape(path, &mut text);
    let _ = writeln!(
        text,
        "\t{}\t{}\t{}\t{top}",
        stamp_text(stamp),
        flags.text(),
        outline.language.map_or("-", Language::name),
    );
    let outline_at = text.len();
    // Each shown definition with the members that follow it: a definition
    // one level in before any top-level one has nothing to be a member of.
    let mut shown: Vec<(&crate::outline::Entry, usize, Vec<&str>)> = Vec::new();
    let mut seen_top = 0usize;
    for entry in &outline.entries {
        match entry.depth {
            0 => {
                seen_top += 1;
                if seen_top <= TOP_ENTRIES {
                    shown.push((entry, 0, Vec::new()));
                }
            }
            1 if (1..=TOP_ENTRIES).contains(&seen_top) => {
                if let Some((_, members, names)) = shown.last_mut() {
                    *members += 1;
                    if names.len() < MEMBERS {
                        names.push(&entry.name);
                    }
                }
            }
            _ => {}
        }
    }
    for (entry, members, names) in shown {
        text.push_str("O\t");
        text.push_str(entry.kind.word());
        text.push('\t');
        escape(&entry.name, &mut text);
        let _ = write!(text, "\t{members}");
        for name in names {
            text.push('\t');
            escape(name, &mut text);
        }
        text.push('\n');
    }
    Made {
        text,
        outline_at,
        path: path.to_owned(),
        stamp: *stamp,
        flags,
        language: outline.language,
        top,
    }
}

/// A cache being written, as a pass goes: its text, and its records as they
/// are added, so that what it makes is a [`CacheFile`] without reading it
/// back.
#[derive(Debug)]
pub(crate) struct Writer {
    text: String,
    dirs: Vec<(String, usize)>,
    records: Vec<Record>,
}

impl Writer {
    /// A cache for the workspace at `root`, of about `size` bytes.
    pub(crate) fn new(root: &str, size: usize) -> Self {
        let mut text = String::with_capacity(size);
        let _ = writeln!(text, "{HEADER} {FORMAT} {EXTRACTORS}");
        text.push_str("R\t");
        escape(root, &mut text);
        text.push('\n');
        Self {
            text,
            dirs: Vec::new(),
            records: Vec::new(),
        }
    }

    pub(crate) fn dir(&mut self, dir: &str, files: usize) {
        self.text.push_str("D\t");
        escape(dir, &mut self.text);
        let _ = writeln!(self.text, "\t{files}");
        self.dirs.push((dir.to_owned(), files));
    }

    /// A file's record, written from what a pass found.
    #[cfg(test)]
    pub(crate) fn file(&mut self, path: &str, stamp: &Stamp, flags: Flags, outline: &Outline) {
        self.add(record(path, stamp, flags, outline));
    }

    /// A record a pass made.
    pub(crate) fn add(&mut self, made: Made) {
        let start = self.text.len();
        self.text.push_str(&made.text);
        self.records.push(Record {
            path: made.path,
            stamp: made.stamp,
            flags: made.flags,
            language: made.language,
            top: made.top,
            lines: start..self.text.len(),
            outline: start + made.outline_at..self.text.len(),
        });
    }

    /// A record of the cache a pass read, written again unchanged.
    pub(crate) fn keep(&mut self, old: &CacheFile, record: &Record) {
        let start = self.text.len();
        self.text.push_str(old.lines_of(record));
        let offset = |at: usize| start + (at - record.lines.start);
        self.records.push(Record {
            lines: start..self.text.len(),
            outline: offset(record.outline.start)..offset(record.outline.end),
            ..record.clone()
        });
    }

    /// The cache, with what the pass found in all.
    pub(crate) fn finish(mut self, found: usize, partial: &[Partial]) -> CacheFile {
        let partial_text: String = if partial.is_empty() {
            "-".to_owned()
        } else {
            partial.iter().map(|why| why.letter()).collect()
        };
        let files = self.records.len();
        let _ = writeln!(self.text, "S\t{found}\t{files}\t{partial_text}");
        let _ = writeln!(self.text, "E\t{files}");
        CacheFile {
            text: self.text,
            found,
            indexed: files,
            partial: partial.to_vec(),
            dirs: self.dirs,
            records: self.records,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::{Entry, Kind};

    fn stamp(size: u64) -> Stamp {
        Stamp {
            size,
            modified: -5,
            change: Some((7, 9)),
        }
    }

    fn entry(depth: u8, kind: Kind, name: &str) -> Entry {
        Entry {
            line: 1,
            depth,
            kind,
            name: name.into(),
            head: format!("{} {name}", kind.word()),
            doc: Some("Does\na.".into()),
        }
    }

    fn written() -> CacheFile {
        let mut writer = Writer::new("/w", 0);
        writer.dir("", 3);
        writer.dir("src", 2);
        writer.file(
            "src/a\tb.rs",
            &stamp(10),
            Flags {
                unsettled: true,
                ..Flags::default()
            },
            &Outline {
                language: Some(Language::Rust),
                entries: vec![
                    entry(1, Kind::Method, "orphan"),
                    entry(0, Kind::Impl, "A\\\nB"),
                    entry(1, Kind::Method, "one"),
                    entry(2, Kind::Function, "deeper"),
                    entry(1, Kind::Method, "two"),
                    entry(0, Kind::Function, "f"),
                ],
                truncated: false,
            },
        );
        writer.file(
            "src/b.bin",
            &stamp(4),
            Flags {
                binary: true,
                ..Flags::default()
            },
            &Outline::default(),
        );
        writer.finish(3, &[Partial::Files])
    }

    #[test]
    fn a_cache_reads_back_what_was_written() {
        let made = written();
        let cache = CacheFile::parse(made.text.clone(), "/w").expect("a cache");
        assert_eq!((cache.found, cache.indexed), (3, 2));
        assert_eq!(cache.partial, vec![Partial::Files]);
        assert_eq!(cache.dirs, vec![(String::new(), 3), ("src".to_owned(), 2)]);
        assert_eq!(cache.dirs, made.dirs);
        assert_eq!(cache.records.len(), 2);
        let first = cache.find("src/a\tb.rs").expect("the file");
        assert!(first.flags.unsettled);
        assert_eq!(first.stamp, stamp(10));
        assert_eq!(first.top, 2);
        let shown = cache.shown(first).expect("its definitions");
        assert_eq!(
            shown,
            vec![
                Shown {
                    kind: Kind::Impl,
                    name: "A\\\nB".into(),
                    members: 2,
                    names: vec!["one".into(), "two".into()],
                },
                Shown {
                    kind: Kind::Function,
                    name: "f".into(),
                    members: 0,
                    names: Vec::new(),
                },
            ]
        );
        // What the writer made is what reading it back gives.
        let made_first = made.find("src/a\tb.rs").expect("the file");
        assert_eq!(made.shown(made_first), Some(shown));
        assert_eq!(made_first.lines, first.lines);
        assert!(cache.find("src/b.bin").expect("the other").flags.binary);
        assert_eq!(
            cache.shown(cache.find("src/b.bin").expect("it")),
            Some(vec![])
        );
        assert!(cache.find("src/c.rs").is_none());
    }

    /// The cache keeps the first top-level definitions and the first
    /// members of each, and counts the rest.
    #[test]
    fn a_record_keeps_what_the_map_shows_and_counts_the_rest() {
        let mut entries = Vec::new();
        for n in 0..TOP_ENTRIES + 3 {
            entries.push(entry(0, Kind::Struct, &format!("S{n}")));
            for m in 0..MEMBERS + 2 {
                entries.push(entry(1, Kind::Method, &format!("m{m}")));
            }
        }
        let mut writer = Writer::new("/w", 0);
        writer.file(
            "a.rs",
            &stamp(1),
            Flags::default(),
            &Outline {
                language: Some(Language::Rust),
                entries,
                truncated: false,
            },
        );
        let cache = CacheFile::parse(writer.finish(1, &[]).text, "/w").expect("a cache");
        let record = cache.find("a.rs").expect("the file");
        assert_eq!(record.top, TOP_ENTRIES + 3);
        let shown = cache.shown(record).expect("its definitions");
        assert_eq!(shown.len(), TOP_ENTRIES);
        assert!(shown.iter().all(|one| one.members == MEMBERS + 2));
        assert!(shown.iter().all(|one| one.names.len() == MEMBERS));
        assert!(!cache.text().contains("\tS12\t"), "past the first ones");
    }

    /// A record kept by a pass is the same record in the new cache.
    #[test]
    fn a_kept_record_reads_back_as_it_was() {
        let old = written();
        let mut writer = Writer::new("/w", 0);
        writer.dir("", 1);
        let record = old.find("src/a\tb.rs").expect("the file").clone();
        writer.keep(&old, &record);
        let new = writer.finish(1, &[]);
        let kept = new.find("src/a\tb.rs").expect("kept");
        assert_eq!(new.shown(kept), old.shown(&record));
        let read = CacheFile::parse(new.text.clone(), "/w").expect("a cache");
        assert_eq!(read.find("src/a\tb.rs").expect("read").lines, kept.lines);
    }

    #[test]
    fn a_cache_that_cannot_be_trusted_is_none() {
        let text = written().text;
        // Another root.
        assert!(CacheFile::parse(text.clone(), "/other").is_none());
        // Cut short: every cut but the whole text.
        for cut in 0..text.len() {
            if text.is_char_boundary(cut) {
                assert!(
                    CacheFile::parse(text[..cut].to_owned(), "/w").is_none(),
                    "cut at {cut} was read"
                );
            }
        }
        // Another version.
        let older = text.replacen(&format!("{FORMAT} {EXTRACTORS}"), "1 1", 1);
        assert!(CacheFile::parse(older, "/w").is_none());
        // A count of definitions that does not match its lines, and records
        // out of order.
        let wrong = text.replacen("\trust\t2\n", "\trust\t3\n", 1);
        assert_ne!(wrong, text);
        assert!(CacheFile::parse(wrong, "/w").is_none());
        let mut writer = Writer::new("/w", 0);
        writer.file("b", &stamp(1), Flags::default(), &Outline::default());
        writer.file("a", &stamp(1), Flags::default(), &Outline::default());
        assert!(CacheFile::parse(writer.finish(2, &[]).text, "/w").is_none());
        // Something after the end.
        assert!(CacheFile::parse(format!("{text}F\tz\n"), "/w").is_none());
    }

    /// A definition's line this build did not write leaves its file out of
    /// the map and nothing else: it is read when the file is shown.
    #[test]
    fn a_bad_definition_line_is_found_when_its_file_is_shown() {
        let text = written()
            .text
            .replacen("O\tfn\tf\t0\n", "O\tnothing\tf\t0\n", 1);
        let cache = CacheFile::parse(text, "/w").expect("still a cache");
        assert_eq!(cache.shown(cache.find("src/a\tb.rs").expect("it")), None);
    }
}
