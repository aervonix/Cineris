//! The plan panel's rows (`docs/architecture/09-tui.md`, "Panels" and
//! "Editing the plan"; screen 4).
//!
//! ```text
//! ◇ Plan  1 of 4 done
//!   ☑ allow _ between digits
//!   ◧ reject a trailing _
//!   ☐ add a test for 1_000
//!   ☐ run clippy
//! ```
//!
//! The first row is `◇` in the terminal's own colour, `Plan` in bold and
//! the count muted. Each item sits two columns in, one a row, cut with `…`:
//! a done item's `☑` is the success colour and its text muted; the active
//! item's `◧` is the accent; a pending item's `☐` is muted and its text
//! plain; a skipped item's `⊘` and text are muted. The focused item's text
//! is drawn as the selection, the accent's tint in bold (screen 4's active
//! item, which has the focus there). The ASCII set draws `[x] [>] [ ] [-]`.
//!
//! Collapsed, the panel is its first row and the active item, or the first
//! not yet done when none is active.

use cortex_protocol::item::{Plan, PlanItem, PlanStatus};
use cortex_term::{Line, Span};

use super::panel::{self, PanelRow};
use super::{Look, one_line};
use crate::layout::cut;
use crate::theme::Token;

/// The plan panel's first row: `◇ Plan  1 of 4 done`.
#[must_use]
pub fn plan_header(plan: &Plan, width: usize, look: &Look) -> Line {
    let done = plan
        .items
        .iter()
        .filter(|item| item.status == PlanStatus::Done)
        .count();
    panel::header(
        "Plan",
        &[Span::new(
            format!("  {done} of {} done", plan.items.len()),
            look.theme.style(Token::Muted),
        )],
        width,
        look,
    )
}

/// One item's row, two columns in, `focused` when it has the focus.
#[must_use]
pub fn plan_item(item: &PlanItem, focused: bool, width: usize, look: &Look) -> Line {
    let theme = &look.theme;
    let glyphs = look.glyphs;
    let (glyph, glyph_token, text_token) = match item.status {
        PlanStatus::Done => (glyphs.plan_done, Token::Success, Token::Muted),
        PlanStatus::Active => (glyphs.plan_active, Token::Accent, Token::Text),
        PlanStatus::Skipped => (glyphs.plan_skipped, Token::Muted, Token::Muted),
        // Pending, and a status this build does not know: still to do.
        _ => (glyphs.plan_todo, Token::Muted, Token::Text),
    };
    let text_token = if focused { Token::Selected } else { text_token };
    let line = Line::plain("  ")
        .with(Span::new(format!("{glyph} "), theme.style(glyph_token)))
        .with(Span::new(one_line(&item.text), theme.style(text_token)));
    cut(&line, width, glyphs.ellipsis)
}

/// The plan drawn whole, as its focused panel shows it: the first row, then
/// each item, the `focused` one drawn as the selection.
#[must_use]
pub fn plan_rows(plan: &Plan, focused: Option<usize>, width: usize, look: &Look) -> Vec<PanelRow> {
    let mut rows = vec![PanelRow::plain(plan_header(plan, width, look))];
    for (index, item) in plan.items.iter().enumerate() {
        rows.push(PanelRow::of(
            index,
            plan_item(item, focused == Some(index), width, look),
        ));
    }
    rows
}

/// The plan collapsed: its first row, and the item being worked on (the
/// active one, or the first not yet done), if there is one.
#[must_use]
pub fn plan_collapsed(plan: &Plan, width: usize, look: &Look) -> Vec<Line> {
    let mut lines = vec![plan_header(plan, width, look)];
    if let Some(item) = current(plan) {
        lines.push(plan_item(item, false, width, look));
    }
    lines
}

/// The item being worked on: the active one, or else the first still to
/// do.
#[must_use]
pub fn current(plan: &Plan) -> Option<&PlanItem> {
    plan.items
        .iter()
        .find(|item| item.status == PlanStatus::Active)
        .or_else(|| {
            plan.items
                .iter()
                .find(|item| !matches!(item.status, PlanStatus::Done | PlanStatus::Skipped))
        })
}

#[cfg(test)]
mod tests;
