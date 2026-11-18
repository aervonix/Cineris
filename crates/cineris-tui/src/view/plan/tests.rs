#![allow(
    clippy::disallowed_methods,
    reason = "the tests read the reference screens"
)]

use cineris_protocol::PlanItemId;
use cineris_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn item(id: &str, text: &str, status: PlanStatus) -> PlanItem {
    PlanItem {
        id: PlanItemId::from(id),
        text: text.to_owned(),
        status,
    }
}

/// Screen 4's plan.
pub(crate) fn screen_plan() -> Plan {
    Plan {
        items: vec![
            item("p1", "allow _ between digits", PlanStatus::Done),
            item("p2", "reject a trailing _", PlanStatus::Active),
            item("p3", "add a test for 1_000", PlanStatus::Pending),
            item("p4", "run clippy", PlanStatus::Pending),
        ],
        revision: 3,
    }
}

/// Every status, an item too long for the narrowest window, and one with
/// wide characters.
fn every_status() -> Plan {
    Plan {
        items: vec![
            item("a", "read the scanner", PlanStatus::Done),
            item("b", "skip the old fixtures", PlanStatus::Skipped),
            item(
                "c",
                "allow _ between digits in every literal the lexer reads, decimal, hexadecimal, octal and binary alike",
                PlanStatus::Active,
            ),
            item("d", "名前を確認する", PlanStatus::Pending),
            item(
                "e",
                "a status from a newer core",
                PlanStatus::from_wire("blocked"),
            ),
        ],
        revision: 1,
    }
}

#[test]
fn the_plan_in_every_variant() {
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let width = variant.width();
        let plan = every_status();
        let collapsed = plan_collapsed(&plan, width, &look);
        let focused: Vec<Line> = plan_rows(&plan, Some(3), width, &look)
            .into_iter()
            .map(|row| row.line)
            .collect();
        for (name, lines) in [("collapsed", collapsed), ("focused on the fourth", focused)] {
            let problems = testing::problems(&lines, width);
            assert!(problems.is_empty(), "{name}: {problems:?}");
            parts.push((format!("{} — {name}", variant.title()), lines));
        }
    }
    testing::assert_snapshot("plan", &testing::sections(&parts));
}

#[test]
fn screen_fours_plan_rows_are_drawn_cell_for_cell() {
    let reference = std::fs::read_to_string(format!(
        "{}/../../docs/architecture/tui-look/4.txt",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .replace("\r\n", "\n");
    let start = reference
        .lines()
        .position(|line| line.starts_with("◇ Plan"))
        .expect("screen 4 has the plan");
    let expected: Vec<&str> = reference.lines().skip(start).take(5).collect();
    let got: Vec<String> = plan_rows(&screen_plan(), Some(1), 99, &look())
        .iter()
        .map(|row| row.line.text().trim_end().to_owned())
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn each_status_and_the_focus_take_screen_fours_tokens() {
    let look = look();
    let style = |token| look.theme.style(token);
    let rows = plan_rows(&screen_plan(), Some(1), 99, &look);
    let spans = |at: usize| rows[at].line.spans().to_vec();
    // `◇` plain, `Plan` bold, the count muted.
    let header = spans(0);
    assert_eq!(header[0].style(), style(Token::Text));
    assert_eq!(header[2].text(), "Plan");
    assert_eq!(header[2].style(), style(Token::Title));
    assert_eq!(header[3].style(), style(Token::Muted));
    // Done: success glyph, muted text.
    assert_eq!(spans(1)[1].style(), style(Token::Success));
    assert_eq!(spans(1)[2].style(), style(Token::Muted));
    // Active and focused: the accent's glyph, the selection's text.
    assert_eq!(spans(2)[1].text(), "◧ ");
    assert_eq!(spans(2)[1].style(), style(Token::Accent));
    assert_eq!(spans(2)[2].style(), style(Token::Selected));
    // Pending: muted glyph, plain text.
    assert_eq!(spans(3)[1].style(), style(Token::Muted));
    assert_eq!(spans(3)[2].style(), style(Token::Text));
    // The focus on a pending item draws its text as the selection, and the
    // active item's text plain.
    let moved = plan_rows(&screen_plan(), Some(2), 99, &look);
    assert_eq!(moved[3].line.spans()[2].style(), style(Token::Selected));
    assert_eq!(moved[2].line.spans()[2].style(), style(Token::Text));
    // Rows are tagged with their items, the first row with none.
    let tags: Vec<Option<usize>> = rows.iter().map(|row| row.item).collect();
    assert_eq!(tags, [None, Some(0), Some(1), Some(2), Some(3)]);
}

#[test]
fn collapsed_it_is_its_first_row_and_the_item_being_worked_on() {
    let look = look();
    let text = |plan: &Plan| -> Vec<String> {
        plan_collapsed(plan, 99, &look)
            .iter()
            .map(|line| line.text().into_owned())
            .collect()
    };
    assert_eq!(
        text(&screen_plan()),
        ["◇ Plan  1 of 4 done", "  ◧ reject a trailing _"]
    );
    // With none active, the first still to do.
    let mut plan = screen_plan();
    plan.items[1].status = PlanStatus::Done;
    assert_eq!(text(&plan)[1], "  ☐ add a test for 1_000");
    // With nothing left to do, its first row alone.
    for item in &mut plan.items {
        item.status = PlanStatus::Skipped;
    }
    assert_eq!(text(&plan), ["◇ Plan  0 of 4 done"]);
}

#[test]
fn the_ascii_set_draws_its_own_marks() {
    let look = Look::new(Theme::new(ColorDepth::Mono, None), &Glyphs::ASCII);
    let rows: Vec<String> = plan_rows(&every_status(), None, 200, &look)
        .iter()
        .map(|row| row.line.text().into_owned())
        .collect();
    assert_eq!(rows[0], "o Plan  1 of 5 done");
    assert!(rows[1].starts_with("  [x] "), "{rows:?}");
    assert!(rows[2].starts_with("  [-] "), "{rows:?}");
    assert!(rows[3].starts_with("  [>] "), "{rows:?}");
    assert!(rows[4].starts_with("  [ ] "), "{rows:?}");
}

#[test]
fn hostile_items_are_drawn_visible_on_one_row_at_every_width() {
    let hostile = [
        "\u{1b}[31mred\u{1b}[0m",
        "right\u{202e}to left",
        "zero\u{200b}width",
        "line\nbreak\tand tab",
        "\u{9b}2J clear",
        "\u{e0041}tag",
    ];
    let plan = Plan {
        items: hostile
            .iter()
            .enumerate()
            .map(|(at, text)| item(&format!("h{at}"), text, PlanStatus::Pending))
            .collect(),
        revision: 1,
    };
    let look = look();
    for width in 1..=100 {
        let mut lines: Vec<Line> = plan_rows(&plan, Some(2), width, &look)
            .into_iter()
            .map(|row| row.line)
            .collect();
        lines.extend(plan_collapsed(&plan, width, &look));
        let problems = testing::problems(&lines, width);
        assert!(problems.is_empty(), "at {width}: {problems:#?}");
        assert_eq!(lines.len(), plan.items.len() + 1 + 2, "one row an item");
    }
}
