//! View functions: state, a width and a [`Look`] in, styled lines out
//! (`docs/architecture/09-tui.md`, "Structure of `cineris-tui`").
//!
//! A view is pure. It reads its state and draws it, and keeps nothing: the
//! same state, width and look give the same lines, which is what the
//! snapshot tests hold it to and what lets the full-screen presenter draw the
//! same views as the inline one.
//!
//! **The width a view is given is the columns a line may use,** which is one
//! less than the window's (`cineris_term::Inline::width`): no row reaches the
//! terminal's last column. A view never returns a line wider than that.
//!
//! **Content passes through `cineris_term::sanitize`** before it is laid out:
//! text from a model, a tool or a file never reaches a line with a control
//! character in it, nor with a character that acts unseen (a bidirectional
//! override, a character of no width, a tag character) other than as its
//! escape. What a view makes of content after that, such as the character
//! a markdown entity stands for, passes through it again. `testing::problems`
//! finds either in drawn lines, for the tests of hostile text.

use std::time::Duration;

use crate::glyphs::Glyphs;
use crate::theme::Theme;

pub mod blocks;
pub mod header;
pub mod hint;
pub mod round;

// Step 21's views: the input box and the menus.
pub mod input;
pub mod menu;

// Step 23: the application's dialogs.
pub mod dialog;
pub mod viewer;

// Step 24: the panels.
pub mod panel;
pub mod plan;
pub mod project;
pub mod review;
pub mod usage;

// Step 25: the first run.
pub mod setup;

pub use blocks::{error, listing, notice, reasoning, reasoning_live, turn_summary, user};
pub use header::{Header, header};
pub use hint::{Hint, HintRow, RunningKeys, Speed, hint_row};
pub use round::{
    Call, CallState, Change, Detail, DiffLine, Measure, Placement, Round, Target, round,
    shows_detail,
};

/// Text from outside on one line: control characters as visible symbols,
/// and tabs and line breaks, which `cineris_term::sanitize` keeps, as
/// spaces. For a field a view draws on one row: a tool's name, a file name,
/// the header's and the hint row's values.
pub(crate) fn one_line(text: &str) -> String {
    cineris_term::sanitize(text).replace(['\n', '\t'], " ")
}

/// How a view draws, beside its state: the theme, the glyph set and the
/// `tui` settings that shape the transcript. Step 23 fills it from the
/// configuration; until then, [`Look::new`] gives the defaults.
#[derive(Debug, Clone)]
pub struct Look {
    /// The colours, for the terminal being drawn to.
    pub theme: Theme,
    /// The glyph set.
    pub glyphs: &'static Glyphs,
    /// Blank lines between blocks, or only between turns (`tui.density`).
    pub density: Density,
    /// The widest prose may be, in columns, its two-column indent included
    /// (`tui.prose_width`, 80).
    pub prose_width: usize,
    /// The most diff lines a box opens on by itself, and the most rows of
    /// detail an open box in the live region shows (`tui.tool_detail`, 12).
    pub tool_detail: usize,
    /// Which tool detail starts open (`tui.tool_output`).
    pub tool_output: ToolOutput,
    /// What finished reasoning leaves in the transcript (`tui.reasoning`).
    pub reasoning: Reasoning,
    /// A dim summary line after each turn (`tui.turn_summary`, off).
    pub turn_summary: bool,
    /// A still mark in place of the spinner (`tui.reduced_motion`, off).
    pub reduced_motion: bool,
    /// The terminal shows links (`Capabilities::hyperlinks`), so a link's
    /// address is not written out after its text.
    pub hyperlinks: bool,
}

impl Look {
    /// The defaults of every `tui` setting, with `theme` and `glyphs`.
    #[must_use]
    pub fn new(theme: Theme, glyphs: &'static Glyphs) -> Self {
        Self {
            theme,
            glyphs,
            density: Density::Comfortable,
            prose_width: 80,
            tool_detail: 12,
            tool_output: ToolOutput::Auto,
            reasoning: Reasoning::Summary,
            turn_summary: false,
            reduced_motion: false,
            hyperlinks: false,
        }
    }

    /// The spinner's frame for a tick, or its still mark under reduced
    /// motion.
    #[must_use]
    pub fn spinner(&self, tick: usize) -> &'static str {
        if self.reduced_motion {
            self.glyphs.still_spinner()
        } else {
            self.glyphs.spinner_frame(tick)
        }
    }
}

/// `tui.density`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Density {
    /// A blank line between blocks (the default).
    Comfortable,
    /// A blank line only between turns, and reasoning as its first line
    /// alone.
    Compact,
}

/// `tui.tool_output`: which tool detail starts open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolOutput {
    /// A failure's output and a diff of up to `tui.tool_detail` lines start
    /// open; the rest starts closed (the default).
    Auto,
    /// Every detail starts open.
    Always,
    /// Every box stays closed.
    Never,
}

/// `tui.reasoning`: what finished reasoning leaves in the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reasoning {
    /// `∴ Reasoned for 6s` and the provider's summary (the default).
    Summary,
    /// `∴ Reasoned for 6s` alone; the summary is in the viewer.
    Duration,
}

/// A time as the turn and reasoning show it: `3s`, `1m 12s`, `2h 5m`.
#[must_use]
pub fn duration(time: Duration) -> String {
    let secs = time.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, secs % 3600 / 60)
    }
}

/// A time as a tool call shows it: tenths under ten seconds (`3.4s`), then
/// as [`duration`].
#[must_use]
pub fn precise(time: Duration) -> String {
    if time.as_secs() < 10 {
        let tenths = time.as_millis() / 100;
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else {
        duration(time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_the_screens_show_them() {
        assert_eq!(duration(Duration::from_millis(14_900)), "14s");
        assert_eq!(duration(Duration::from_secs(72)), "1m 12s");
        assert_eq!(duration(Duration::from_secs(7_500)), "2h 5m");
        assert_eq!(precise(Duration::from_millis(3_450)), "3.4s");
        assert_eq!(precise(Duration::from_millis(400)), "0.4s");
        assert_eq!(precise(Duration::from_millis(12_300)), "12s");
    }
}
