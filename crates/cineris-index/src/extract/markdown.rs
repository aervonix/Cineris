//! Markdown's outline: its headings, nested by level.
//!
//! ATX headings (`## Title`) and setext ones (a line of `=` or `-` under
//! a paragraph's line), outside fenced code blocks and front matter. The
//! shallowest level in the file is depth 0, so a file that starts at `##`
//! is not shown one level in. Only the first [`MAX_ENTRIES`] headings are
//! kept, each cut as it is found, but the shallowest level is the whole
//! file's.

use super::{Builder, LINE_BYTES, line_number, prefix};
use crate::outline::{Entry, HEAD_CHARS, Kind, MAX_ENTRIES, clip, squeeze};

/// The headings found so far.
#[derive(Debug, Default)]
struct Headings {
    /// The first ones: each one's line, level and text, cut.
    kept: Vec<(usize, usize, String)>,
    /// The shallowest level of all of them, kept or not.
    shallowest: Option<usize>,
    /// More were found than are kept.
    more: bool,
}

impl Headings {
    fn add(&mut self, index: usize, level: usize, text: &str) {
        self.shallowest = Some(self.shallowest.map_or(level, |least| least.min(level)));
        if self.kept.len() < MAX_ENTRIES {
            let text = clip(&squeeze(prefix(text, LINE_BYTES)), HEAD_CHARS);
            self.kept.push((index, level, text));
        } else {
            self.more = true;
        }
    }
}

/// The fence that opens a code block: its character and its length.
fn fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let mark = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = trimmed.chars().take_while(|c| *c == mark).count();
    (length >= 3).then_some((mark, length))
}

/// An ATX heading: its level and its text.
fn atx(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    // A closing run of `#` is not part of the text.
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((level, text))
}

/// A setext underline: the level it gives the line above it.
fn underline(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    if trimmed.is_empty() || line.len() - line.trim_start().len() > 3 {
        return None;
    }
    if trimmed.chars().all(|c| c == '=') {
        Some(1)
    } else if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 2 {
        Some(2)
    } else {
        None
    }
}

/// Whether a line can be the text of a setext heading: a paragraph's line,
/// not a list item, a quote, a table row or an indented code line.
fn paragraph(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && line.len() - line.trim_start().len() < 4
        && !trimmed.starts_with(['-', '*', '+', '>', '|', '#', '<'])
        && !trimmed.starts_with(|c: char| c.is_ascii_digit())
}

pub(crate) fn markdown(text: &str, out: &mut Builder) {
    let mut found = Headings::default();
    let mut open_fence: Option<(char, usize)> = None;
    let mut front_matter = false;
    let mut previous: Option<(usize, &str)> = None;
    for (index, line) in text.lines().enumerate() {
        if index == 0 && line.trim_end() == "---" {
            front_matter = true;
            continue;
        }
        if front_matter {
            if matches!(line.trim_end(), "---" | "...") {
                front_matter = false;
            }
            continue;
        }
        if let Some((mark, length)) = open_fence {
            if let Some((closing, closing_length)) = fence(line)
                && closing == mark
                && closing_length >= length
                && line.trim().chars().all(|c| c == mark)
            {
                open_fence = None;
            }
            continue;
        }
        if let Some(opened) = fence(line) {
            open_fence = Some(opened);
            previous = None;
            continue;
        }
        if let Some((level, heading)) = atx(line) {
            if !heading.is_empty() {
                found.add(index, level, heading);
            }
            previous = None;
            continue;
        }
        if let Some(level) = underline(line)
            && let Some((above, text)) = previous
        {
            found.add(above, level, text.trim());
            previous = None;
            continue;
        }
        previous = paragraph(line).then_some((index, line));
    }
    let shallowest = found.shallowest.unwrap_or(1);
    for (index, level, heading) in found.kept {
        let depth = u8::try_from(level - shallowest).unwrap_or(u8::MAX);
        let _ = out.push(Entry {
            line: line_number(index),
            depth,
            kind: Kind::Section,
            name: heading.clone(),
            head: clip(&format!("{} {heading}", "#".repeat(level)), HEAD_CHARS),
            doc: None,
        });
    }
    if found.more {
        out.cut();
    }
}
