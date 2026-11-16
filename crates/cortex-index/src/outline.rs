//! What an outline is: a file's definitions, each with its line, its depth
//! and what kind of thing it is (`docs/architecture/15-efficiency.md`,
//! "Repository map").
//!
//! An outline is approximate by design (decision 0022): the extractors read
//! lines, not grammars, so code with an unusual layout can lose a
//! definition or name one wrongly. The model checks by reading the part it
//! needs.

use std::fmt::Write as _;

use cortex_base::visible;

use crate::language::Language;

/// The most entries one file's outline keeps. A generated file with fifty
/// thousand functions is outlined by its first ones, and says so.
pub const MAX_ENTRIES: usize = 1_000;

/// The most characters a definition's head keeps.
pub const HEAD_CHARS: usize = 160;

/// The most characters a definition's name keeps: a name is what the map
/// shows of a definition, and a generated or hostile file can make one of
/// megabytes.
pub const NAME_CHARS: usize = 80;

/// The most characters a doc comment's summary keeps.
pub const DOC_CHARS: usize = 100;

/// What kind of definition an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A free function.
    Function,
    /// A function inside a type, an `impl` or an interface.
    Method,
    /// A class.
    Class,
    /// A struct, a record or a union.
    Struct,
    /// An enumeration.
    Enum,
    /// A trait, an interface or a protocol.
    Interface,
    /// An `impl` block or an extension: a type's methods, defined apart.
    Impl,
    /// A module or a namespace.
    Module,
    /// A type alias or another named type.
    Type,
    /// A constant or a static.
    Const,
    /// A macro.
    Macro,
    /// A property of a type.
    Property,
    /// A heading of a document.
    Section,
    /// A block found by indentation alone, in a file no extractor reads.
    Block,
}

impl Kind {
    /// Every kind, in the order the cache numbers them.
    pub const ALL: [Self; 14] = [
        Self::Function,
        Self::Method,
        Self::Class,
        Self::Struct,
        Self::Enum,
        Self::Interface,
        Self::Impl,
        Self::Module,
        Self::Type,
        Self::Const,
        Self::Macro,
        Self::Property,
        Self::Section,
        Self::Block,
    ];

    /// The short word the map and the cache use for it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Method => "method",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Impl => "impl",
            Self::Module => "mod",
            Self::Type => "type",
            Self::Const => "const",
            Self::Macro => "macro",
            Self::Property => "prop",
            Self::Section => "section",
            Self::Block => "block",
        }
    }

    /// The kind a word names, as [`word`](Self::word) gives it.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }

    /// Whether definitions can be nested inside one of this kind.
    #[must_use]
    pub fn holds_others(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Struct
                | Self::Enum
                | Self::Interface
                | Self::Impl
                | Self::Module
                | Self::Section
                | Self::Block
        )
    }
}

/// One definition of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its line, counted from 1.
    pub line: u32,
    /// How deep it is nested: 0 for the top level, 1 for a method of a
    /// top-level class.
    pub depth: u8,
    /// What it is.
    pub kind: Kind,
    /// Its name: `parse`, `Parser`, `Display for Parser`.
    pub name: String,
    /// The line that defines it, trimmed and cut before its body: what the
    /// `outline` tool shows.
    pub head: String,
    /// The first line of its doc comment, where one sits next to it.
    pub doc: Option<String>,
}

/// The outline of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    /// The language whose extractor made it. `None` when the indentation
    /// fallback did.
    pub language: Option<Language>,
    /// The definitions, in the order they are in the file.
    pub entries: Vec<Entry>,
    /// More definitions were found than [`MAX_ENTRIES`]: the outline holds
    /// the first ones.
    pub truncated: bool,
}

impl Outline {
    /// The outline as the `outline` tool and `read` show it: one definition
    /// a line, its line number first, nested ones indented, a doc comment's
    /// summary after the head.
    #[must_use]
    pub fn render(&self) -> String {
        let width = self
            .entries
            .iter()
            .map(|entry| entry.line)
            .max()
            .unwrap_or(1)
            .to_string()
            .len();
        let mut text = String::new();
        for entry in &self.entries {
            let _ = write!(
                text,
                "{:>width$}  {:indent$}{}",
                entry.line,
                "",
                visible(&entry.head),
                indent = usize::from(entry.depth) * 2
            );
            if let Some(doc) = &entry.doc {
                let _ = write!(text, "  — {}", visible(doc));
            }
            text.push('\n');
        }
        if self.truncated {
            let _ = writeln!(
                text,
                "[The outline stops at its first {MAX_ENTRIES} definitions.]"
            );
        }
        text
    }
}

/// Cuts `text` to at most `chars` characters, marking a cut with `…`.
#[must_use]
pub(crate) fn clip(text: &str, chars: usize) -> String {
    // Only the first `chars + 1` characters are looked at, so a line of
    // megabytes costs no more than a short one.
    if text.char_indices().nth(chars).is_none() {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `text` with every run of white space made one space, and trimmed.
#[must_use]
pub(crate) fn squeeze(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_reads_back_from_its_word() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_word(kind.word()), Some(kind));
        }
        assert_eq!(Kind::from_word("nothing"), None);
    }

    /// A head and a doc are shown through `cortex_base::visible`: a tag
    /// character (the independent review of step 18) is an escape too.
    #[test]
    fn an_outline_shows_what_would_act_unseen_as_escapes() {
        use cortex_testkit::hostile;
        // Every name of the hostile set that acts unseen, in a head and in
        // a doc.
        for kind in [
            hostile::Kind::Control,
            hostile::Kind::Bidi,
            hostile::Kind::Format,
            hostile::Kind::LineBreak,
            hostile::Kind::Escape,
        ] {
            for name in hostile::of(kind) {
                let outline = Outline {
                    language: None,
                    entries: vec![Entry {
                        line: 1,
                        depth: 0,
                        kind: Kind::Block,
                        name: "a".into(),
                        head: format!("a{}b", name.text),
                        doc: Some(format!("c{}", name.text)),
                    }],
                    truncated: false,
                };
                assert_eq!(
                    outline.render(),
                    format!("1  a{shown}b  — c{shown}\n", shown = name.shown),
                    "{}",
                    name.label
                );
            }
        }
    }

    #[test]
    fn clipping_marks_the_cut_and_keeps_what_fits() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcd", 3), "ab…");
        assert_eq!(clip("ééé", 5), "ééé");
        assert_eq!(clip("", 3), "");
    }

    #[test]
    fn an_outline_renders_with_aligned_line_numbers_and_indentation() {
        let outline = Outline {
            language: Some(Language::Rust),
            entries: vec![
                Entry {
                    line: 3,
                    depth: 0,
                    kind: Kind::Struct,
                    name: "A".into(),
                    head: "pub struct A".into(),
                    doc: Some("An A.".into()),
                },
                Entry {
                    line: 12,
                    depth: 1,
                    kind: Kind::Method,
                    name: "new".into(),
                    head: "fn new() -> Self".into(),
                    doc: None,
                },
            ],
            truncated: false,
        };
        assert_eq!(
            outline.render(),
            " 3  pub struct A  — An A.\n12    fn new() -> Self\n"
        );
    }
}
