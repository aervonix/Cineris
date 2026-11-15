//! The extractors: text in, outline out (decision 0022).
//!
//! Each is a line-oriented scanner over the lexer's code (comments and
//! string contents blanked), so a keyword or a brace inside a string is
//! not taken for one. There are four engines, and the languages are tables
//! and small recognisers on top of them:
//!
//! | Engine | Languages | How nesting is found |
//! |---|---|---|
//! | [`brace`] | Rust, Go, Java, Kotlin, C#, C, C++, JavaScript, TypeScript, Swift, PHP, shell | Braces: a definition whose body opens with `{` holds what is inside it when it is a type, an `impl`, an interface or a module, and nothing when it is a function. |
//! | [`indent`] | Python, Ruby | Indentation: a `class` or `module` holds what is indented under it; a function holds nothing. |
//! | [`markdown`] | Markdown | Heading levels. |
//! | [`indent`]'s fallback | every other text file | A line with more indented lines under it is a block, two levels deep. |
//!
//! The interface is the one a parser-based extractor would implement
//! ([`extract`]): accuracy can be bought later, per language, behind a
//! cargo feature.

mod brace;
mod docs;
mod indent;
mod langs;
mod markdown;

use crate::language::Language;
use crate::outline::{Entry, MAX_ENTRIES, Outline};

/// The outline of a file's text. `file_name` is the file's last name,
/// which tells a C header from a source file. `language` is `None` for a
/// file no extractor reads, which the indentation fallback outlines.
#[must_use]
pub fn extract(language: Option<Language>, file_name: &str, text: &str) -> Outline {
    let mut out = Builder::default();
    match language {
        Some(Language::Python) => indent::python(text, &mut out),
        Some(Language::Ruby) => indent::ruby(text, &mut out),
        Some(Language::Markdown) => markdown::markdown(text, &mut out),
        Some(language) => brace::run(
            langs::recogniser(language, Language::is_header(file_name)),
            text,
            &mut out,
        ),
        None => indent::fallback(text, &mut out),
    }
    // A definition whose name was to come at its close and never did.
    out.entries.retain(|entry| !entry.name.is_empty());
    Outline {
        language,
        entries: out.entries,
        truncated: out.truncated,
    }
}

/// The entries an extractor has found so far.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    entries: Vec<Entry>,
    truncated: bool,
}

impl Builder {
    /// Adds an entry, and returns its index; `None` past the cap.
    pub(crate) fn push(&mut self, entry: Entry) -> Option<usize> {
        if self.entries.len() >= MAX_ENTRIES {
            self.truncated = true;
            return None;
        }
        self.entries.push(entry);
        Some(self.entries.len() - 1)
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut Entry> {
        self.entries.get_mut(index)
    }

    /// Takes the entry at `index` out again: a declaration that turned out
    /// to have no body where one was needed. Only the last entry can be.
    pub(crate) fn drop_last(&mut self, index: usize) {
        if index + 1 == self.entries.len() {
            self.entries.pop();
        }
    }
}

/// The line number of the `index`th line, counted from 1.
pub(crate) fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// The width of a line's indentation, a tab counting to the next multiple
/// of eight.
pub(crate) fn indent_of(line: &str) -> usize {
    let mut width = 0;
    for character in line.chars() {
        match character {
            ' ' => width += 1,
            '\t' => width = (width / 8 + 1) * 8,
            _ => break,
        }
    }
    width
}

#[cfg(test)]
mod tests;
