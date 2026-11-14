//! What a line of markdown starts: the line-by-line half of the parser.
//!
//! A block is decided from whole lines, as they end, so that a block's kind
//! is known before any of it is drawn as final. These are the CommonMark
//! block structures that 09-tui draws, with two simplifications for
//! streaming, which `docs/architecture/09-tui.md` ("The commit rule")
//! records: a heading is written with `#` only (a `---` or `===` under text
//! is a rule or text, never a heading underline, because it would restyle
//! lines already committed), and a table's header row starts with `|`.

/// What a line, with its containers' marks taken off, is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    /// Nothing but white space.
    Blank,
    /// A code fence: three or more backticks or tildes, and an info string.
    Fence { ch: u8, len: usize, info: String },
    /// An ATX heading: its level and text.
    Heading { level: u8, text: String },
    /// A thematic break.
    Rule,
    /// A block quote's mark: the rest of the line after `>` and a space.
    Quote { rest: usize },
    /// A list item: its marker as written (`-`, `1.`), whether it is
    /// ordered, and where its content starts.
    Item {
        marker: String,
        ordered: bool,
        content: usize,
    },
    /// A line that starts with `|`: a table row, or a header row if the
    /// next line is a delimiter row.
    Pipe,
    /// Indented four or more columns: code, where a paragraph is not open.
    Indented,
    /// Anything else: paragraph text.
    Text,
}

/// What `line` starts. `line` has no line break, and its containers' marks
/// are already taken off.
pub(crate) fn classify(line: &str) -> Line {
    let indent = indentation(line);
    let rest = line.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return Line::Blank;
    }
    if indent >= 4 {
        return Line::Indented;
    }
    let bytes = rest.as_bytes();
    let offset = line.len() - rest.len();
    if let Some(fence) = fence(rest) {
        return fence;
    }
    if bytes[0] == b'#' {
        let level = bytes.iter().take_while(|&&b| b == b'#').count();
        let after = &rest[level..];
        if level <= 6 && (after.is_empty() || after.starts_with([' ', '\t'])) {
            return Line::Heading {
                level: u8::try_from(level).unwrap_or(6),
                text: heading_text(after),
            };
        }
    }
    if is_rule(rest) {
        return Line::Rule;
    }
    if bytes[0] == b'>' {
        let skip = if rest[1..].starts_with([' ', '\t']) {
            2
        } else {
            1
        };
        return Line::Quote {
            rest: offset + skip,
        };
    }
    if let Some((marker, ordered, after)) = list_marker(rest) {
        let spaces =
            rest[marker.len()..].len() - rest[marker.len()..].trim_start_matches(' ').len();
        if after {
            // Content after one to four spaces; with five or more, the content
            // is code indented after one.
            let gap = if (1..=4).contains(&spaces) { spaces } else { 1 };
            return Line::Item {
                content: offset + marker.len() + gap.min(rest.len() - marker.len()),
                marker,
                ordered,
            };
        }
    }
    if bytes[0] == b'|' {
        return Line::Pipe;
    }
    Line::Text
}

/// Whether `partial`, the start of a line, is certainly paragraph text
/// from what is there so far: what it starts cannot be anything else
/// whatever follows. A line that might still turn into another block waits
/// until it ends.
pub(crate) fn surely_text(partial: &str) -> bool {
    if indentation(partial) >= 4 {
        return false;
    }
    let rest = partial.trim_start_matches(' ');
    let bytes = rest.as_bytes();
    let Some(&first) = bytes.first() else {
        return false;
    };
    let run = bytes.iter().take_while(|&&b| b == first).count();
    let after = bytes.get(run).copied();
    match first {
        // A heading needs one to six `#` and then a space.
        b'#' => after.is_some_and(|b| b != b' ' && b != b'\t') || run > 6,
        // A list item needs its marker and then a space; a rule needs nothing
        // but its character and spaces.
        b'-' | b'*' | b'+' | b'_' => {
            let not_rule = first == b'+'
                || bytes
                    .iter()
                    .any(|&b| !matches!(b, b' ' | b'\t') && b != first);
            let not_item = first == b'_' || bytes.get(1).is_some_and(|&b| b != b' ' && b != b'\t');
            not_rule && not_item
        }
        // A fence needs three; fewer, then something else, is text.
        b'`' | b'~' => run < 3 && after.is_some(),
        // An ordered item needs up to nine digits, `.` or `)`, and a space.
        b'0'..=b'9' => match after {
            None => false,
            Some(b'.' | b')') => {
                run > 9 || bytes.get(run + 1).is_some_and(|&b| b != b' ' && b != b'\t')
            }
            Some(_) => true,
        },
        b'>' | b'|' | b'\t' => false,
        _ => true,
    }
}

/// The columns of white space at the start of `line`, a tab taking it to the
/// next multiple of four.
pub(crate) fn indentation(line: &str) -> usize {
    let mut columns = 0;
    for b in line.bytes() {
        match b {
            b' ' => columns += 1,
            b'\t' => columns += 4 - columns % 4,
            _ => break,
        }
    }
    columns
}

/// `line` with up to `columns` columns of its leading white space taken off,
/// or `None` if it has fewer.
pub(crate) fn dedent(line: &str, columns: usize) -> Option<&str> {
    let mut used = 0;
    for (at, b) in line.bytes().enumerate() {
        if used >= columns {
            return Some(&line[at..]);
        }
        match b {
            b' ' => used += 1,
            // A tab is taken whole, even past the columns asked for.
            b'\t' => used += 4 - used % 4,
            _ => return None,
        }
    }
    (used >= columns || line.trim().is_empty()).then_some("")
}

fn fence(rest: &str) -> Option<Line> {
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|&b| b == ch).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    // A backtick fence's info string may not hold a backtick.
    if ch == b'`' && info.contains('`') {
        return None;
    }
    Some(Line::Fence {
        ch,
        len,
        info: info.split_whitespace().next().unwrap_or("").to_owned(),
    })
}

/// Whether `line` closes a fence of `ch` that was `len` long.
pub(crate) fn closes_fence(line: &str, ch: u8, len: usize) -> bool {
    if indentation(line) >= 4 {
        return false;
    }
    let rest = line.trim_start_matches(' ');
    let run = rest.bytes().take_while(|&b| b == ch).count();
    run >= len && rest[run..].trim().is_empty()
}

fn heading_text(after: &str) -> String {
    let text = after.trim();
    // A closing run of `#` after a space is not part of the text.
    let without = text.trim_end_matches('#');
    if without.len() < text.len() && (without.is_empty() || without.ends_with([' ', '\t'])) {
        without.trim_end().to_owned()
    } else {
        text.to_owned()
    }
}

fn is_rule(rest: &str) -> bool {
    let Some(ch) = rest.bytes().next() else {
        return false;
    };
    if !matches!(ch, b'-' | b'*' | b'_') {
        return false;
    }
    let mut count = 0;
    for b in rest.bytes() {
        match b {
            b' ' | b'\t' => {}
            b if b == ch => count += 1,
            _ => return false,
        }
    }
    count >= 3
}

/// A list marker at the start of `rest`: the marker, whether it is ordered,
/// and whether a space (or the end of the line) follows it.
fn list_marker(rest: &str) -> Option<(String, bool, bool)> {
    let bytes = rest.as_bytes();
    let (marker, ordered) = match bytes[0] {
        b'-' | b'*' | b'+' => (rest[..1].to_owned(), false),
        b'0'..=b'9' => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
                return None;
            }
            (rest[..=digits].to_owned(), true)
        }
        _ => return None,
    };
    let after = rest[marker.len()..].is_empty() || rest[marker.len()..].starts_with([' ', '\t']);
    Some((marker, ordered, after))
}

/// A table's delimiter row: its columns' alignments, or `None`.
pub(crate) fn delimiter_row(line: &str) -> Option<Vec<Align>> {
    let cells = split_row(line.trim());
    if cells.is_empty() {
        return None;
    }
    let mut aligns = Vec::with_capacity(cells.len());
    for cell in cells {
        let cell = cell.trim();
        let left = cell.starts_with(':');
        let right = cell.ends_with(':');
        let dashes = cell.trim_matches(':');
        if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
            return None;
        }
        aligns.push(match (left, right) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    Some(aligns)
}

/// How a table's column is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
}

/// A table row's cells: split at each `|` that is not escaped and not in a
/// code span, with the outer pipes taken off.
pub(crate) fn split_row(line: &str) -> Vec<&str> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut start = 0;
    let mut ticks = 0usize;
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 1,
            b'`' => {
                let run = bytes[at..].iter().take_while(|&&b| b == b'`').count();
                ticks = if ticks == 0 {
                    run
                } else if ticks == run {
                    0
                } else {
                    ticks
                };
                at += run - 1;
            }
            b'|' if ticks == 0 => {
                cells.push(&line[start..at]);
                start = at + 1;
            }
            _ => {}
        }
        at += 1;
    }
    let last = &line[start.min(line.len())..];
    if !last.trim().is_empty() || !line.trim_end().ends_with('|') {
        cells.push(last);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_what_they_start() {
        assert_eq!(classify(""), Line::Blank);
        assert_eq!(classify("   \t"), Line::Blank);
        assert_eq!(classify("    code"), Line::Indented);
        assert_eq!(
            classify("```rust title"),
            Line::Fence {
                ch: b'`',
                len: 3,
                info: "rust".to_owned()
            }
        );
        assert_eq!(classify("``` a`b"), Line::Text);
        assert_eq!(classify("``inline``"), Line::Text);
        assert_eq!(
            classify("~~~~"),
            Line::Fence {
                ch: b'~',
                len: 4,
                info: String::new()
            }
        );
        assert_eq!(
            classify("## Title ##"),
            Line::Heading {
                level: 2,
                text: "Title".to_owned()
            }
        );
        assert_eq!(
            classify("# C#"),
            Line::Heading {
                level: 1,
                text: "C#".to_owned()
            }
        );
        assert_eq!(classify("#hashtag"), Line::Text);
        assert_eq!(classify("####### seven"), Line::Text);
        assert_eq!(classify("---"), Line::Rule);
        assert_eq!(classify(" * * *"), Line::Rule);
        assert_eq!(classify("__"), Line::Text);
        assert_eq!(classify("> quote"), Line::Quote { rest: 2 });
        assert_eq!(classify(">quote"), Line::Quote { rest: 1 });
        assert_eq!(
            classify("- item"),
            Line::Item {
                marker: "-".to_owned(),
                ordered: false,
                content: 2
            }
        );
        assert_eq!(
            classify("10) ten"),
            Line::Item {
                marker: "10)".to_owned(),
                ordered: true,
                content: 4
            }
        );
        assert_eq!(classify("-not"), Line::Text);
        assert_eq!(classify("1.5 is a number"), Line::Text);
        assert_eq!(classify("**bold** start"), Line::Text);
        assert_eq!(classify("| a | b |"), Line::Pipe);
        assert_eq!(classify("a | b"), Line::Text);
        assert_eq!(classify("==="), Line::Text);
    }

    #[test]
    fn a_partial_line_is_text_only_when_nothing_else_can_start_it() {
        for text in [
            "Hello",
            "  the",
            "**bold",
            "*em",
            "-x",
            "--x",
            "+x",
            "_x_",
            "#hashtag",
            "####### 7",
            "`x",
            "``x",
            "~x",
            "1.5",
            "12ab",
            "=== x",
        ] {
            assert!(surely_text(text), "{text:?}");
            assert_eq!(
                classify(&format!("{text} and more")),
                Line::Text,
                "{text:?}"
            );
        }
        for undecided in [
            "", "  ", "1", "1.", "-", "*", "**", "* ", "- x", "#", "## ", ">", "`", "```", "|",
            "~", "    x", "_", "__ _", "* * ",
        ] {
            assert!(!surely_text(undecided), "{undecided:?}");
        }
    }

    #[test]
    fn tables_split_their_rows_and_read_their_delimiters() {
        assert_eq!(split_row("| a | b |"), [" a ", " b "]);
        assert_eq!(split_row("| a \\| b | `c|d` |"), [" a \\| b ", " `c|d` "]);
        assert_eq!(split_row("|a|"), ["a"]);
        assert_eq!(
            delimiter_row("|:---|:--:|---:|"),
            Some(vec![Align::Left, Align::Center, Align::Right])
        );
        assert_eq!(delimiter_row("| a | b |"), None);
        assert_eq!(delimiter_row("|---|-x-|"), None);
    }

    #[test]
    fn fences_close_and_lines_dedent() {
        assert!(closes_fence("```", b'`', 3));
        assert!(closes_fence("  `````  ", b'`', 3));
        assert!(!closes_fence("``", b'`', 3));
        assert!(!closes_fence("``` rust", b'`', 3));
        assert!(!closes_fence("~~~", b'`', 3));
        assert_eq!(dedent("    code", 4), Some("code"));
        assert_eq!(dedent("  two", 4), None);
        assert_eq!(dedent("", 4), Some(""));
        assert_eq!(dedent("\tx", 2), Some("x"));
        assert_eq!(indentation("\t  x"), 6);
    }
}
