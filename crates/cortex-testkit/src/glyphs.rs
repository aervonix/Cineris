//! Matching what a screen shows when the interface may have drawn it in
//! either of its two glyph sets.
//!
//! The interface draws Unicode glyphs (`◆`, `│`) where the terminal shows
//! Unicode, and ASCII stand-ins (`>`, `|`) where it does not: a Windows
//! console gets the ASCII set, and so does a Unix terminal under the `C`
//! locale. A test of the real binary does not know which it will meet, and
//! used to spell both (`starts_with(['│', '|'])`). [`GlyphSets`] holds the
//! sets and matches a text spelled once, as a function of the set.
//!
//! The test kit does not know the sets: they are `cortex-tui`'s
//! (`Glyphs::UNICODE` and `Glyphs::ASCII`), which the test kit cannot depend
//! on. The test names them, so that there is one copy of each glyph, the
//! product's:
//!
//! ```text
//! const SETS: GlyphSets<'static, Glyphs> = GlyphSets::new(&[Glyphs::UNICODE, Glyphs::ASCII]);
//!
//! SETS.shows(session.screen(), |g| format!("{} {text}", g.user))
//! SETS.history_has_row(session.screen(), |g| {
//!     RowPattern::starting_with(g.vertical).containing(model)
//! })
//! ```

use crate::vterm::VTerm;

/// A row of text that starts and holds given texts: the part of a row a
/// test cares about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowPattern {
    start: Option<String>,
    contains: Vec<String>,
}

impl RowPattern {
    /// A row that starts with `text`, white space before it ignored (a box's
    /// side may have a margin before it).
    #[must_use]
    pub fn starting_with(text: &str) -> Self {
        Self {
            start: Some(text.to_owned()),
            contains: Vec::new(),
        }
    }

    /// A row that holds `text` anywhere.
    #[must_use]
    pub fn containing(mut self, text: &str) -> Self {
        self.contains.push(text.to_owned());
        self
    }

    /// Whether `row` matches.
    #[must_use]
    pub fn matches(&self, row: &str) -> bool {
        self.start
            .as_ref()
            .is_none_or(|start| row.trim_start().starts_with(start.as_str()))
            && self.contains.iter().all(|text| row.contains(text.as_str()))
    }
}

/// The glyph sets a screen may have been drawn in, `G` being the product's
/// type for one set.
#[derive(Debug, Clone, Copy)]
pub struct GlyphSets<'a, G> {
    sets: &'a [G],
}

impl<'a, G> GlyphSets<'a, G> {
    /// The sets to match against: every one the interface can draw in.
    #[must_use]
    pub const fn new(sets: &'a [G]) -> Self {
        Self { sets }
    }

    /// Whether any row of the screen holds the text `spell` gives for any of
    /// the sets.
    pub fn shows(&self, term: &VTerm, spell: impl Fn(&G) -> String) -> bool {
        self.sets.iter().any(|set| term.shows(&spell(set)))
    }

    /// Whether any row of the scrollback and the screen matches the pattern
    /// `pattern` gives for any of the sets.
    pub fn history_has_row(&self, term: &VTerm, pattern: impl Fn(&G) -> RowPattern) -> bool {
        let rows = term.history();
        self.sets.iter().any(|set| {
            let pattern = pattern(set);
            rows.iter().any(|row| pattern.matches(row))
        })
    }

    /// `row` without the prefix `spell` gives for the first set whose prefix
    /// it starts with.
    pub fn strip_prefix<'r>(&self, row: &'r str, spell: impl Fn(&G) -> String) -> Option<&'r str> {
        self.sets
            .iter()
            .find_map(|set| row.strip_prefix(spell(set).as_str()))
    }

    /// `row` without the suffix `spell` gives for the first set whose
    /// suffix it ends with.
    pub fn strip_suffix<'r>(&self, row: &'r str, spell: impl Fn(&G) -> String) -> Option<&'r str> {
        self.sets
            .iter()
            .find_map(|set| row.strip_suffix(spell(set).as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sets of two glyphs, as the product has, made up for the test.
    struct Set {
        prompt: &'static str,
        side: &'static str,
    }

    const SETS: GlyphSets<'static, Set> = GlyphSets::new(&[
        Set {
            prompt: "◆",
            side: "│",
        },
        Set {
            prompt: ">",
            side: "|",
        },
    ]);

    fn screen(rows: &[&str]) -> VTerm {
        let mut term = VTerm::new(40, 6);
        term.feed(rows.join("\r\n").as_bytes());
        term
    }

    #[test]
    fn text_spelled_once_is_found_in_either_set() {
        for prompt in ["◆", ">"] {
            let term = screen(&[&format!("{prompt} fix it")]);
            assert!(
                SETS.shows(&term, |g| format!("{} fix it", g.prompt)),
                "{prompt}"
            );
            assert!(!SETS.shows(&term, |g| format!("{} fix that", g.prompt)));
        }
        // A glyph of neither set is not matched.
        assert!(!SETS.shows(&screen(&["* fix it"]), |g| format!("{} fix it", g.prompt)));
    }

    #[test]
    fn a_rows_start_is_matched_after_its_margin_and_a_set_is_not_mixed_with_another() {
        for side in ["│", "|"] {
            let term = screen(&["header", &format!("  {side} model-x ")]);
            assert!(SETS.history_has_row(&term, |g| {
                RowPattern::starting_with(g.side).containing("model-x")
            }));
            assert!(!SETS.history_has_row(&term, |g| {
                RowPattern::starting_with(g.side).containing("model-y")
            }));
        }
        // The side in the middle of a row does not start it.
        let term = screen(&["a | model-x"]);
        assert!(!SETS.history_has_row(&term, |g| {
            RowPattern::starting_with(g.side).containing("model-x")
        }));
    }

    #[test]
    fn a_row_pattern_needs_every_text_it_holds() {
        let pattern = RowPattern::starting_with("|")
            .containing("model-x")
            .containing("high");
        assert!(pattern.matches("| model-x high"));
        assert!(!pattern.matches("| model-x low"));
        assert!(!pattern.matches("| model-y high"));
        // No start asks for none; no text asks for none.
        assert!(RowPattern::default().matches("anything"));
        assert!(RowPattern::default().containing("any").matches("anything"));
    }

    #[test]
    fn a_prefix_is_taken_off_in_whichever_set_the_row_has() {
        let spell = |g: &Set| format!("{} {} ", g.side, g.prompt);
        assert_eq!(SETS.strip_prefix("│ ◆ typed", spell), Some("typed"));
        assert_eq!(SETS.strip_prefix("| > typed", spell), Some("typed"));
        assert_eq!(SETS.strip_prefix("│ > typed", spell), None);
        let side = |g: &Set| g.side.to_owned();
        assert_eq!(SETS.strip_suffix("typed │", side), Some("typed "));
        assert_eq!(SETS.strip_suffix("typed |", side), Some("typed "));
        assert_eq!(SETS.strip_suffix("typed", side), None);
    }
}
