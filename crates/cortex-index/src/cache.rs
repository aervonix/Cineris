//! The on-disk cache of a project's outlines: one file,
//! `<state>/projects/<project key>/index/outlines`, owner-only.
//!
//! It is text, one record a line, fields split by tabs (a tab, a line
//! break, a carriage return or a backslash in a field is escaped):
//!
//! ```text
//! cortex-index outlines <format> <extractors>
//! R  <the workspace root it was made for>
//! D  <directory>  <files under it>                       one per directory
//! F  <path>  <size>  <modified>  <change>  <flags>  <language>  <entries>
//! O  <line>  <depth>  <kind>  <name>  <head>  <doc>          its entries
//! S  <files found>  <files indexed>  <why it is partial, or ->
//! E  <files>
//! ```
//!
//! - **The files are in [`path_order`]**, so a pass reads the cache and the
//!   listing side by side, and a lookup is a binary search. The summary
//!   comes after them, so that a pass writes the file as it goes.
//! - **A cache this build cannot trust is no cache:** another format or
//!   another version of the extractors, another root, a record out of
//!   order, an entry count that does not add up, or no `E` line (cut
//!   short). It is read as empty, and the next pass writes a whole one.
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
use crate::outline::{Entry, Kind, Outline};
use crate::walk::{Partial, Stamp, path_order};

/// The version of the cache's format.
pub(crate) const FORMAT: u32 = 1;

/// The version of the extractors. Raised whenever an extractor finds
/// something else in the same text, so that outlines cached by an older
/// build are made again.
pub(crate) const EXTRACTORS: u32 = 1;

const HEADER: &str = "cortex-index outlines";

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
    /// How many entries its outline has.
    pub(crate) entries: usize,
    /// How many of them are at the top level.
    pub(crate) top: usize,
    /// Its `F` line and its `O` lines, as bytes of the cache's text.
    pub(crate) lines: Range<usize>,
    /// Its `O` lines alone.
    pub(crate) outline: Range<usize>,
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

impl CacheFile {
    /// The cache at `path`, if there is one this build can trust for the
    /// workspace at `root`.
    pub(crate) fn read(path: &Path, root: &str) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let text = String::from_utf8(bytes).ok()?;
        Self::parse(text, root)
    }

    /// Reads and checks a cache's text.
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
        let mut open: Option<(Record, usize)> = None;
        let mut summary: Option<(usize, usize, Vec<Partial>)> = None;
        let mut ended = false;
        while let Some((line, range)) = lines.next() {
            if ended {
                return None;
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
                    if let Some((record, left)) = open.take() {
                        if left != 0 {
                            return None;
                        }
                        records.push(record);
                    }
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
                    let entries: usize = fields.next()?.parse().ok()?;
                    if fields.next().is_some() {
                        return None;
                    }
                    if records
                        .last()
                        .is_some_and(|last| path_order(&last.path, &path).is_ge())
                    {
                        return None;
                    }
                    open = Some((
                        Record {
                            path,
                            stamp: Stamp {
                                size,
                                modified,
                                change,
                            },
                            flags,
                            language,
                            entries,
                            top: 0,
                            lines: range.start..range.end,
                            outline: range.end..range.end,
                        },
                        entries,
                    ));
                }
                "O" => {
                    let (record, left) = open.as_mut()?;
                    *left = left.checked_sub(1)?;
                    if check_entry(rest)? == 0 {
                        record.top += 1;
                    }
                    record.lines.end = range.end;
                    record.outline.end = range.end;
                }
                "S" => {
                    if let Some((record, left)) = open.take() {
                        if left != 0 {
                            return None;
                        }
                        records.push(record);
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
    pub(crate) fn find(&self, path: &str) -> Option<&Record> {
        self.records
            .binary_search_by(|record| path_order(&record.path, path))
            .ok()
            .map(|at| &self.records[at])
    }

    /// A record's lines as they stand in the cache, to be written again
    /// unchanged.
    pub(crate) fn lines_of(&self, record: &Record) -> &str {
        &self.text[record.lines.clone()]
    }

    /// A record's outline.
    pub(crate) fn outline_of(&self, record: &Record) -> Outline {
        let entries = self.text[record.outline.clone()]
            .lines()
            .filter_map(|line| parse_entry(line.strip_prefix("O\t")?))
            .collect();
        Outline {
            language: record.language,
            entries,
            truncated: record.flags.truncated,
        }
    }
}

/// An entry's line checked without reading its texts: its depth, or
/// `None` when it is not an entry this build wrote. The texts are read
/// when an outline is asked for ([`parse_entry`]).
fn check_entry(rest: &str) -> Option<u8> {
    let mut fields = rest.split('\t');
    let _line: u32 = fields.next()?.parse().ok()?;
    let depth: u8 = fields.next()?.parse().ok()?;
    Kind::from_word(fields.next()?)?;
    for _ in 0..3 {
        let text = fields.next()?;
        // A backslash only begins one of the four escapes.
        let mut escaped = false;
        for byte in text.bytes() {
            if escaped {
                if !matches!(byte, b'\\' | b't' | b'n' | b'r') {
                    return None;
                }
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            }
        }
        if escaped {
            return None;
        }
    }
    fields.next().is_none().then_some(depth)
}

fn parse_entry(rest: &str) -> Option<Entry> {
    let mut fields = rest.split('\t');
    let line = fields.next()?.parse().ok()?;
    let depth = fields.next()?.parse().ok()?;
    let kind = Kind::from_word(fields.next()?)?;
    let name = unescape(fields.next()?)?;
    let head = unescape(fields.next()?)?;
    let doc = fields.next()?;
    let doc = if doc.is_empty() {
        None
    } else {
        Some(unescape(doc)?)
    };
    if fields.next().is_some() {
        return None;
    }
    Some(Entry {
        line,
        depth,
        kind,
        name,
        head,
        doc,
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

/// A cache being written, as a pass goes.
#[derive(Debug)]
pub(crate) struct Writer {
    text: String,
    files: usize,
}

/// A file's record as the cache writes it, from what a pass found.
pub(crate) fn record(path: &str, stamp: &Stamp, flags: Flags, outline: &Outline) -> String {
    let mut text = String::new();
    text.push_str("F\t");
    escape(path, &mut text);
    let _ = writeln!(
        text,
        "\t{}\t{}\t{}\t{}",
        stamp_text(stamp),
        flags.text(),
        outline.language.map_or("-", Language::name),
        outline.entries.len()
    );
    for entry in &outline.entries {
        let _ = write!(
            text,
            "O\t{}\t{}\t{}\t",
            entry.line,
            entry.depth,
            entry.kind.word()
        );
        escape(&entry.name, &mut text);
        text.push('\t');
        escape(&entry.head, &mut text);
        text.push('\t');
        if let Some(doc) = &entry.doc {
            escape(doc, &mut text);
        }
        text.push('\n');
    }
    text
}

impl Writer {
    pub(crate) fn new(root: &str) -> Self {
        let mut text = String::new();
        let _ = writeln!(text, "{HEADER} {FORMAT} {EXTRACTORS}");
        text.push_str("R\t");
        escape(root, &mut text);
        text.push('\n');
        Self { text, files: 0 }
    }

    pub(crate) fn dir(&mut self, dir: &str, files: usize) {
        self.text.push_str("D\t");
        escape(dir, &mut self.text);
        let _ = writeln!(self.text, "\t{files}");
    }

    /// A file's record, written from what a pass found.
    #[cfg(test)]
    pub(crate) fn file(&mut self, path: &str, stamp: &Stamp, flags: Flags, outline: &Outline) {
        self.copy(&record(path, stamp, flags, outline));
    }

    /// A file's record, as text already made: [`record`]'s, or the lines
    /// of the cache it was read from.
    pub(crate) fn copy(&mut self, lines: &str) {
        self.files += 1;
        self.text.push_str(lines);
    }

    /// The cache's text, with what the pass found in all.
    pub(crate) fn finish(mut self, found: usize, partial: &[Partial]) -> String {
        let partial: String = if partial.is_empty() {
            "-".to_owned()
        } else {
            partial.iter().map(|why| why.letter()).collect()
        };
        let _ = writeln!(self.text, "S\t{found}\t{}\t{partial}", self.files);
        let _ = writeln!(self.text, "E\t{}", self.files);
        self.text
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

    fn written() -> String {
        let mut writer = Writer::new("/w");
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
                entries: vec![Entry {
                    line: 1,
                    depth: 0,
                    kind: Kind::Function,
                    name: "a".into(),
                    head: "fn a()\\".into(),
                    doc: Some("Does\na.".into()),
                }],
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
        let text = written();
        let cache = CacheFile::parse(text, "/w").expect("a cache");
        assert_eq!((cache.found, cache.indexed), (3, 2));
        assert_eq!(cache.partial, vec![Partial::Files]);
        assert_eq!(cache.dirs, vec![(String::new(), 3), ("src".to_owned(), 2)]);
        assert_eq!(cache.records.len(), 2);
        let first = cache.find("src/a\tb.rs").expect("the file");
        assert!(first.flags.unsettled);
        assert_eq!(first.stamp, stamp(10));
        let outline = cache.outline_of(first);
        assert_eq!(outline.entries[0].head, "fn a()\\");
        assert_eq!(outline.entries[0].doc.as_deref(), Some("Does\na."));
        assert!(cache.find("src/b.bin").expect("the other").flags.binary);
        assert!(cache.find("src/c.rs").is_none());
    }

    #[test]
    fn a_cache_that_cannot_be_trusted_is_none() {
        let text = written();
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
        let older = text.replacen(&format!("{FORMAT} {EXTRACTORS}"), "1 0", 1);
        assert!(CacheFile::parse(older, "/w").is_none());
        // A count that does not add up, and records out of order.
        let wrong = text.replacen("\t1\nO", "\t2\nO", 1);
        assert!(CacheFile::parse(wrong, "/w").is_none());
        let mut writer = Writer::new("/w");
        writer.file("b", &stamp(1), Flags::default(), &Outline::default());
        writer.file("a", &stamp(1), Flags::default(), &Outline::default());
        assert!(CacheFile::parse(writer.finish(2, &[]), "/w").is_none());
        // Something after the end.
        assert!(CacheFile::parse(format!("{text}F\tz\n"), "/w").is_none());
    }
}
