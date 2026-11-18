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
//!
//! **No input recurses, and none is scanned once per prefix.** A pass's
//! threads have the stack `std::thread::scope` gives, and a stack overflow
//! aborts the process, which no `catch_unwind` sees: so a chain of
//! prefixes (`template<…>`, `extern "…"`, annotations) is stripped in a
//! loop, and each part of a line is looked at a bounded number of times.
//! What an extractor keeps is bounded too: a name to [`NAME_CHARS`], the
//! part of a line a head or a name is made from to [`LINE_BYTES`], the
//! entries to one more than [`MAX_ENTRIES`], and a brace that opens no
//! definition is counted, not kept.

mod brace;
mod docs;
mod indent;
mod langs;
mod markdown;

use crate::language::Language;
use crate::outline::{Entry, MAX_ENTRIES, NAME_CHARS, Outline, clip};

/// The most bytes of a line an extractor copies to make a head or a name
/// from. Far more than [`HEAD_CHARS`](crate::outline::HEAD_CHARS) needs,
/// so only a line of kilobytes, or one mostly of white space, is cut
/// before its head is: it keeps a line of megabytes from being copied
/// whole, several times over, to keep 160 characters of it.
pub(crate) const LINE_BYTES: usize = 4096;

/// The start of `text`, at most `bytes` long, cut at a character's start.
pub(crate) fn prefix(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The first characters of `text` that `keep` accepts, no more than a
/// name keeps and one more, which [`Builder::push`] turns into the mark of
/// a cut: a name of megabytes is never built.
pub(crate) fn name_of(text: &str, keep: impl Fn(char) -> bool) -> String {
    text.chars()
        .take_while(|c| keep(*c))
        .take(NAME_CHARS + 1)
        .collect()
}

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
    // The builder held one entry more than the cap: if it survived, there
    // were more definitions than the outline keeps.
    let truncated = out.truncated || out.entries.len() > MAX_ENTRIES;
    out.entries.truncate(MAX_ENTRIES);
    Outline {
        language,
        entries: out.entries,
        truncated,
    }
}

/// The entries an extractor has found so far.
///
/// It holds one entry more than [`MAX_ENTRIES`], so that `truncated` is
/// said only when more definitions than the cap survive: an entry can be
/// taken out again (a C prototype that gets no body, a `typedef struct`
/// that gets no name), and that one is always the last. Every entry before
/// the last has survived by the time another is pushed, so a push refused
/// with all the places full is truly a cut.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    entries: Vec<Entry>,
    /// A definition was refused with every place held by one that stays.
    truncated: bool,
}

impl Builder {
    /// Adds an entry, its name clipped to [`NAME_CHARS`], and returns its
    /// index; `None` past the cap.
    pub(crate) fn push(&mut self, mut entry: Entry) -> Option<usize> {
        if self.entries.len() > MAX_ENTRIES {
            self.truncated = true;
            return None;
        }
        if entry.name.char_indices().nth(NAME_CHARS).is_some() {
            entry.name = clip(&entry.name, NAME_CHARS);
        }
        self.entries.push(entry);
        Some(self.entries.len() - 1)
    }

    /// More definitions were found than the outline keeps, which the
    /// extractor knows without pushing them.
    pub(crate) fn cut(&mut self) {
        self.truncated = true;
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut Entry> {
        self.entries.get_mut(index)
    }

    /// Takes the entry at `index` out again: a declaration that turned out
    /// to have no body where one was needed, or a definition whose name
    /// never came. Only the last entry can be.
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
