//! The doc comment that sits right above a definition: its first line of
//! text is the entry's summary. Cheap, so every extractor takes it.

use crate::outline::{DOC_CHARS, clip};

/// How a language writes the comments that document what follows them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DocStyle {
    /// Line comments that are docs: `///` in Rust, `//` in Go, `#` in Ruby.
    pub(crate) lines: &'static [&'static str],
    /// Block comments that are docs: `/**`, `/*!`.
    pub(crate) blocks: &'static [&'static str],
}

impl DocStyle {
    pub(crate) const JAVADOC: Self = Self {
        lines: &[],
        blocks: &["/**"],
    };
}

/// The doc comment being read above the next definition.
#[derive(Debug, Default)]
pub(crate) struct Docs {
    /// The summary found so far.
    text: Option<String>,
    /// Inside a doc block that has not ended yet.
    in_block: bool,
}

impl Docs {
    /// A line with no code on it: blank, or a comment.
    pub(crate) fn comment_line(&mut self, raw: &str, style: DocStyle) {
        let line = raw.trim();
        if line.is_empty() {
            // A blank line parts a comment from what follows it.
            if !self.in_block {
                self.clear();
            }
            return;
        }
        if self.in_block {
            if let Some((before, _)) = line.split_once("*/") {
                self.in_block = false;
                self.offer(before.trim_start_matches('*'));
            } else {
                self.offer(line.trim_start_matches('*'));
            }
            return;
        }
        if let Some(opener) = style
            .blocks
            .iter()
            .find(|opener| line.starts_with(**opener))
        {
            let rest = &line[opener.len()..];
            // `/**/` is an empty comment, and `/***` a rule of stars.
            if rest.starts_with('*') || rest.starts_with('/') {
                self.clear();
                return;
            }
            match rest.split_once("*/") {
                Some((inside, _)) => self.offer(inside),
                None => {
                    self.in_block = true;
                    self.offer(rest);
                }
            }
            return;
        }
        if let Some(prefix) = style
            .lines
            .iter()
            .filter(|prefix| line.starts_with(**prefix))
            .max_by_key(|prefix| prefix.len())
        {
            let rest = &line[prefix.len()..];
            // `////` is a rule, and `#!` a script's first line.
            if !rest.starts_with(['/', '!']) || prefix.ends_with('!') {
                self.offer(rest);
                return;
            }
        }
        // Another kind of comment: not a doc.
        self.clear();
    }

    /// Takes a line of a doc comment's text as the summary, unless one has
    /// been found already or the line is not prose (a tag, an empty line).
    fn offer(&mut self, text: &str) {
        if self.text.is_some() {
            return;
        }
        let text = text.trim().trim_end_matches("*/").trim();
        if text.is_empty() || text.starts_with(['@', '<', '#', '=']) || text.starts_with("```") {
            return;
        }
        self.text = Some(clip(text, DOC_CHARS));
    }

    /// The summary for a definition that begins here, and a fresh start.
    pub(crate) fn take(&mut self) -> Option<String> {
        let text = self.text.take();
        self.clear();
        text
    }

    /// A line of code that is not a definition: what came before it is not
    /// the next definition's doc.
    pub(crate) fn clear(&mut self) {
        self.text = None;
        self.in_block = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUST: DocStyle = DocStyle {
        lines: &["///"],
        blocks: &["/**"],
    };

    fn doc_of(lines: &[&str], style: DocStyle) -> Option<String> {
        let mut docs = Docs::default();
        for line in lines {
            docs.comment_line(line, style);
        }
        docs.take()
    }

    #[test]
    fn the_first_line_of_prose_is_the_summary() {
        assert_eq!(
            doc_of(&["/// Parses a file.", "///", "/// More."], RUST),
            Some("Parses a file.".into())
        );
        assert_eq!(
            doc_of(&["/**", " * @deprecated", " * Reads it.", " */"], RUST),
            Some("Reads it.".into())
        );
        assert_eq!(
            doc_of(&["/** One line. */"], RUST),
            Some("One line.".into())
        );
    }

    #[test]
    fn a_blank_line_or_another_comment_parts_a_doc_from_its_definition() {
        assert_eq!(doc_of(&["/// Old.", ""], RUST), None);
        assert_eq!(doc_of(&["/// Old.", "// plain"], RUST), None);
        assert_eq!(doc_of(&["//// rule"], RUST), None);
        assert_eq!(doc_of(&["/***/"], RUST), None);
    }
}
